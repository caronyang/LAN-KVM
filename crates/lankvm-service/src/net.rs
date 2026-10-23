//! P0 UDP client for the service session agent.
//!
//! This is the same unauthenticated protocol as lankvm-devtools. It exists
//! only to prove that the SYSTEM agent can inject input on the active desktop
//! (including Winlogon). Production will use QUIC/TLS + pairing.

use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

    log(
        "session",
        format!("P0 UDP client listening on 0.0.0.0:{port} (unencrypted; test only)"),
    );

    thread::spawn(move || client_loop(socket, stop, release_px));
    Ok(())
}

fn client_loop(socket: UdpSocket, stop: Arc<AtomicBool>, release_px: i32) {
    let release_px = release_px.max(1);
    let mut buf = [0u8; 64];
    let mut seq = 0u32;
    let mut last_hello = false;
    let mut controlled = false;
    let mut entry_edge = 1i32;
    let mut virtual_x = 0i32;
    let mut bounds = None;
    let mut last_desktop_gen = desktop::generation();

    while !stop.load(Ordering::SeqCst) {
        // The input desktop changed (lock/unlock/UAC). The cursor on the new
        // desktop is unrelated to the previous entry point, so drop the stale
        // accumulator instead of releasing in the middle of the screen.
        let gen = desktop::generation();
        if gen != last_desktop_gen {
            last_desktop_gen = gen;
            virtual_x = 0;
            log("session", "desktop changed -> edge accumulator reset");
        }

        // Do not auto-release on injection failure here: it makes the lock
        // screen impossible to test. The master can always use
        // Ctrl+Alt+Shift+Q, and the helper process below is the real fix.

        let (n, addr) = match socket.recv_from(&mut buf) {
            Ok(value) => value,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(err) => {
                log("session", format!("UDP receive failed: {err}"));
                break;
            }
        };

        let Some(packet) = Packet::decode(&buf[..n]) else {
            continue;
        };

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
                    let mut release = Packet::new(PacketKind::Release, seq);
                    seq = seq.wrapping_add(1);
                    release.a = 5; // edge
                    let _ = socket.send_to(&release.encode(), addr);
                    controlled = false;
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
}
