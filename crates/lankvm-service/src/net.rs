//! P0 UDP client for the service session agent.
//!
//! This is the same unauthenticated protocol as lankvm-devtools. It exists
//! only to prove that the SYSTEM agent can inject input on the active desktop
//! (including Winlogon). Production will use QUIC/TLS + pairing.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use lankvm_net::{Packet, PacketKind};

use crate::desktop;
use crate::inject::{self, submit, InjectEvent};
use crate::logger::log;
use lankvm_platform::windows as win;

pub fn start_client(port: u16, stop: Arc<AtomicBool>, release_px: i32) -> Result<()> {
    let socket =
        UdpSocket::bind(("0.0.0.0", port)).with_context(|| format!("bind UDP port {port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    // A send to a dead master must not block the receive loop.
    let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
    // Large buffers absorb input bursts and Wi-Fi stalls.
    let sock = socket2::SockRef::from(&socket);
    let _ = sock.set_recv_buffer_size(1 << 20);
    let _ = sock.set_send_buffer_size(1 << 20);

    log(
        "session",
        format!("P0 UDP client listening on 0.0.0.0:{port} (unencrypted; test only)"),
    );

    thread::spawn(move || client_loop(socket, stop, release_px));
    Ok(())
}

fn client_loop(socket: UdpSocket, stop: Arc<AtomicBool>, release_px: i32) {
    // 1-2 px was far too easy to trigger by accident; require a real push.
    let release_px = release_px.max(6);
    let mut buf = [0u8; 64];
    let mut seq = 0u32;
    let mut last_hello = false;
    let mut controlled = false;
    let mut entry_edge = 1i32;
    let mut virtual_x = 0i32;
    let mut edge_hits = 0u32;
    let mut bounds = None;
    let mut last_desktop_gen = desktop::generation();
    // Only send unsolicited keepalives to an address that recently sent us
    // real input. Local health probes use a short-lived socket; replying to
    // them would earn an ICMP port-unreachable and kill this loop with
    // WSAECONNRESET (10054).
    let mut keepalive_to: Option<(SocketAddr, Instant)> = None;
    let mut master_addr: Option<SocketAddr> = None;
    let mut last_keepalive = Instant::now();
    let mut last_packet = Instant::now();
    let mut last_stats = Instant::now();
    let mut rx_packets = 0u64;
    let mut max_gap_ms = 0u64;

    while !stop.load(Ordering::SeqCst) {
        if last_stats.elapsed() >= Duration::from_secs(5) {
            last_stats = Instant::now();
            log(
                "session",
                format!("[net] rx={rx_packets} max_gap={max_gap_ms}ms controlled={controlled}"),
            );
            rx_packets = 0;
            max_gap_ms = 0;
        }

        // Unsolicited keepalive: the master's timeout must not depend on its
        // own PING surviving a Wi-Fi stall. Stop after 5 s without real input
        // so we never spray a dead port.
        if let Some((addr, since)) = keepalive_to {
            if since.elapsed() > Duration::from_secs(5) {
                keepalive_to = None;
            } else if last_keepalive.elapsed() >= Duration::from_millis(500) {
                last_keepalive = Instant::now();
                let mut pong = Packet::new(PacketKind::Pong, seq);
                seq = seq.wrapping_add(1);
                pong.a = 0;
                let _ = socket.send_to(&pong.encode(), addr);
            }
        }

        // The input desktop changed (lock/unlock/UAC). The cursor on the new
        // desktop is unrelated to the previous entry point, so drop the stale
        // accumulator instead of releasing in the middle of the screen.
        let gen = desktop::generation();
        if gen != last_desktop_gen {
            last_desktop_gen = gen;
            virtual_x = 0;
            edge_hits = 0;
            log("session", "desktop changed -> edge accumulator reset");
        }

        // The master disappeared (killed, network gone). Release any keys we
        // are holding so the desktop is not left with a stuck modifier, and
        // tell the master to drop control if it is still alive.
        if controlled && last_packet.elapsed() > Duration::from_secs(15) {
            controlled = false;
            edge_hits = 0;
            submit(InjectEvent::Release);
            if let Some(addr) = master_addr {
                let mut release = Packet::new(PacketKind::Release, seq);
                seq = seq.wrapping_add(1);
                release.a = 3; // timeout
                let _ = socket.send_to(&release.encode(), addr);
            }
            log("session", "master silent 15 s; released keys and control");
        }

        // Do not auto-release on injection failure here: it makes the lock
        // screen impossible to test. The master can always use
        // Ctrl+Alt+Shift+Q, and the helper process below is the real fix.

        let (n, addr) = match socket.recv_from(&mut buf) {
            Ok(value) => value,
            Err(err) if is_ignorable_udp_error(&err) => continue,
            Err(err) => {
                log("session", format!("UDP receive failed: {err}"));
                break;
            }
        };

        let Some(packet) = Packet::decode(&buf[..n]) else {
            continue;
        };
        master_addr = Some(addr);
        {
            let now = Instant::now();
            let gap = now.duration_since(last_packet).as_millis() as u64;
            if gap > max_gap_ms {
                max_gap_ms = gap;
            }
            last_packet = now;
            rx_packets += 1;
        }
        // A local health probe is a bare Ping from a short-lived socket. Only
        // real input packets arm the keepalive, so we never reply to a probe
        // port that is already closed.
        if !matches!(packet.kind, PacketKind::Ping | PacketKind::Pong) {
            keepalive_to = Some((addr, Instant::now()));
        }

        match packet.kind {
            PacketKind::Hello => {
                if !last_hello {
                    log("session", format!("master connected from {addr}"));
                    last_hello = true;
                }
            }
            PacketKind::Switch => {
                entry_edge = packet.b.clamp(1, 2);
                virtual_x = 0;
                edge_hits = 0;
                controlled = true;
                bounds = win::virtual_bounds().ok();
                log(
                    "session",
                    format!("switch: controlled=true entry_edge={entry_edge} bounds={bounds:?}"),
                );
                submit(InjectEvent::Switch {
                    ratio: packet.a.clamp(0, 65_535) as u16,
                    entry_edge,
                });
            }
            PacketKind::Motion => {
                if !controlled {
                    continue;
                }
                virtual_x = virtual_x.saturating_add(packet.a);
                // Hand back only when the real cursor is at the entry edge and
                // is being pushed further outward. The old dx-only check
                // released in the middle of the screen after a desktop switch.
                let outward = if entry_edge == 1 {
                    packet.a < 0
                } else {
                    packet.a > 0
                };
                let at_edge = match bounds.as_ref() {
                    Some(bounds) => win::cursor_pos()
                        .ok()
                        .map(|cursor| {
                            if entry_edge == 1 {
                                cursor.x <= bounds.left
                            } else {
                                cursor.x >= bounds.right - 1
                            }
                        })
                        .unwrap_or(false),
                    None => false,
                };
                let past = if entry_edge == 1 {
                    virtual_x < -release_px
                } else {
                    virtual_x > release_px
                };
                if outward && at_edge && past {
                    edge_hits += 1;
                } else {
                    edge_hits = 0;
                }
                // Require two consecutive outward packets at the edge: a single
                // flick near the edge must not hand control back.
                if edge_hits >= 2 {
                    let mut release = Packet::new(PacketKind::Release, seq);
                    seq = seq.wrapping_add(1);
                    release.a = 5; // edge
                    let _ = socket.send_to(&release.encode(), addr);
                    controlled = false;
                    edge_hits = 0;
                    submit(InjectEvent::Release);
                    log(
                        "session",
                        format!(
                            "cursor at entry edge -> release (virtual_x={virtual_x}, edge={entry_edge})"
                        ),
                    );
                    continue;
                }
                submit(InjectEvent::Motion {
                    dx: packet.a,
                    dy: packet.b,
                });
            }
            PacketKind::Button => {
                if controlled {
                    submit(InjectEvent::Button {
                        button: packet.a as u8,
                        down: packet.b != 0,
                    });
                }
            }
            PacketKind::Key => {
                if controlled {
                    submit(InjectEvent::Key {
                        scan: packet.a as u16,
                        down: packet.b != 0,
                        extended: packet.c != 0,
                        shift: packet.flags & 1 != 0,
                        shift_known: packet.flags & 2 != 0,
                    });
                }
            }
            PacketKind::Wheel => {
                if controlled {
                    submit(InjectEvent::Wheel {
                        delta: packet.a,
                        horizontal: packet.b != 0,
                    });
                }
            }
            PacketKind::Release => {
                controlled = false;
                inject::clear_fail_streak();
                submit(InjectEvent::Release);
                log("session", "release received from master");
            }
            PacketKind::Ping => {
                let mut pong = Packet::new(PacketKind::Pong, seq);
                seq = seq.wrapping_add(1);
                pong.a = packet.a;
                // Marker so the app's health probe only accepts the SYSTEM
                // service, not an orphan user-level client.
                pong.d = 1;
                if let Err(err) = socket.send_to(&pong.encode(), addr) {
                    log("session", format!("UDP pong failed: {err}"));
                }
            }
            PacketKind::Pong => {}
        }
    }

    log("session", "UDP client stopped");
    // A fatal socket error leaves us alive but deaf; the service considers
    // that a healthy process and waits 30 s before restarting. Exit so it can
    // relaunch immediately instead.
    if !stop.load(Ordering::SeqCst) {
        log(
            "session",
            "UDP client ended unexpectedly; exiting session agent",
        );
        std::process::exit(1);
    }
}

/// Windows reports ICMP port-unreachable for a previous send as
/// WSAECONNRESET on the *next* UDP receive. That is normal for UDP and must
/// not kill the listener.
fn is_ignorable_udp_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionRefused
    )
}
