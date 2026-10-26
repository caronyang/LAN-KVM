//! Two-machine LAN PoC: master captures input, client injects it.
//!
//! This is deliberately unauthenticated and unencrypted. It exists to measure
//! edge switching and input latency before the QUIC/pairing layer is built.

use std::collections::HashSet;
use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::net::{SocketAddr, UdpSocket};
use std::ptr::{null, null_mut};
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering,
};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use lankvm_core::{Edge, Point, Rect};
use lankvm_input::EdgeDetector;
use lankvm_net::{Packet, PacketKind};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_MENU, VK_Q, VK_SHIFT,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    HC_ACTION, HHOOK, HWND_MESSAGE, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_INPUT, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW,
};

use crate::windows as win;

pub const DEFAULT_PORT: u16 = 45_871;const WM_LANKVM_RELEASE: u32 = WM_APP + 1;
const WM_LANKVM_STOP: u32 = WM_APP + 2;

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

macro_rules! plog {
    ($($arg:tt)*) => {
        println!("[{}] {}", epoch_secs(), format_args!($($arg)*))
    };
}

static EVENT_TX: OnceLock<Sender<Event>> = OnceLock::new();
static REMOTE: AtomicBool = AtomicBool::new(false);
static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
/// Last time we heard anything from the peer (PONG, keepalive or release).
static LAST_SEEN_MS: AtomicU64 = AtomicU64::new(0);
static REMOTE_MOTION: AtomicU64 = AtomicU64::new(0);
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
static CURSOR_HIDER: AtomicIsize = AtomicIsize::new(0);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static RELEASE_REASON: AtomicU32 = AtomicU32::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static BOUNDS: OnceLock<Rect> = OnceLock::new();
static EDGE_KIND: OnceLock<Edge> = OnceLock::new();
static EDGE: OnceLock<Mutex<EdgeDetector>> = OnceLock::new();

/// Unconsumed relative mouse motion. The sender coalesces it into one packet
/// every few milliseconds instead of sending one packet per raw input event.
static PENDING_DX: AtomicI32 = AtomicI32::new(0);
static PENDING_DY: AtomicI32 = AtomicI32::new(0);
static MOTION_DIRTY: AtomicBool = AtomicBool::new(false);

/// Diagnostics for the 5-second `[net]` line.
static TX_PACKETS: AtomicU64 = AtomicU64::new(0);
static RX_PACKETS: AtomicU64 = AtomicU64::new(0);
static RTT_MAX_WINDOW: AtomicU64 = AtomicU64::new(0);
static LAST_RTT_MS: AtomicU64 = AtomicU64::new(0);
static LAST_RTT_LOG_MS: AtomicU64 = AtomicU64::new(0);
static LAST_LOCAL_WARN_MS: AtomicU64 = AtomicU64::new(0);

/// Silence longer than this releases control. Wi-Fi scans/roams can easily
/// stall 1-4 s, so the old 3 s threshold caused spurious releases.
const PEER_TIMEOUT_MS: u64 = 10_000;
/// Coalescing interval: at most 250 mouse packets per second.
const MOTION_INTERVAL: Duration = Duration::from_millis(4);

/// Ask the running session in this process to stop gracefully.
///
/// Safe to call from another thread. The master unclips/shows the cursor on
/// exit; the client releases injected keys. Do not kill the process instead.
pub fn request_stop() {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
    let tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            PostThreadMessageW(tid, WM_LANKVM_STOP, 0, 0);
        }
    }
}

#[derive(Debug, Clone)]
enum Event {
    Hello,
    Switch {
        ratio: u16,
        entry_edge: i32,
    },
    Button {
        button: u8,
        down: bool,
    },
    Key {
        scan: u16,
        down: bool,
        extended: bool,
        shift: bool,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Release {
        reason: &'static str,
    },
}

pub fn run_master(
    peer: &str,
    port: u16,
    edge: Edge,
    seconds: u64,
    trigger_px: f32,
    trigger_ms: u64,
) -> Result<()> {
    win::enable_dpi_awareness();
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    LAST_SEEN_MS.store(0, Ordering::SeqCst);
    PENDING_DX.store(0, Ordering::SeqCst);
    PENDING_DY.store(0, Ordering::SeqCst);
    MOTION_DIRTY.store(false, Ordering::SeqCst);

    let socket =
        UdpSocket::bind(("0.0.0.0", 0)).with_context(|| "failed to bind master UDP socket")?;
    socket
        .connect((peer, port))
        .with_context(|| format!("failed to resolve/connect to {peer}:{port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    // Sends must never block: when the peer disappears the Wi-Fi/ARP path can
    // stall for seconds and a blocking send would freeze the heartbeat thread
    // (and with it the release timeout).
    let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
    // Bursts of input plus PONGs are tiny; a larger buffer rides out Wi-Fi
    // stalls without dropping packets.
    let sock = socket2::SockRef::from(&socket);
    let _ = sock.set_recv_buffer_size(1 << 20);
    let _ = sock.set_send_buffer_size(1 << 20);

    let start = Instant::now();
    let _ = START.set(start);
    let bounds = win::virtual_bounds()?;
    let _ = BOUNDS.set(bounds);
    let _ = EDGE_KIND.set(edge);
    let _ = EDGE.set(Mutex::new(EdgeDetector::new(edge, trigger_px, trigger_ms)));

    let (tx, rx) = unbounded();
    EVENT_TX
        .set(tx)
        .map_err(|_| anyhow!("internal error: event channel already initialized"))?;

    let main_thread = unsafe { GetCurrentThreadId() };
    MAIN_THREAD_ID.store(main_thread, Ordering::SeqCst);

    let stop = Arc::new(AtomicBool::new(false));
    let sender_socket = socket.try_clone()?;
    let sender_stop = stop.clone();
    let sender_thread = thread::spawn(move || sender_loop(sender_socket, rx, sender_stop));

    let receiver_socket = socket.try_clone()?;
    let receiver_stop = stop.clone();
    let receiver_thread = thread::spawn(move || receiver_loop(receiver_socket, receiver_stop));

    let heartbeat_socket = socket.try_clone()?;
    let heartbeat_stop = stop.clone();
    let heartbeat_thread =
        thread::spawn(move || heartbeat_loop(heartbeat_socket, heartbeat_stop));

    // Independent watchdog: only reads atomics and posts a message, so a stuck
    // network send can never keep the cursor clipped/hidden forever.
    let watchdog_stop = stop.clone();
    let watchdog_thread = thread::spawn(move || watchdog_loop(watchdog_stop));

    let hwnd = create_raw_window()?;
    register_raw_mouse(hwnd)?;
    let hooks = install_hooks()?;

    println!("[master] peer {peer}:{port}, edge {:?}", edge);
    println!("[master] push through the {:?} edge to take control", edge);
    println!("[master] release hotkey: Ctrl+Alt+Shift+Q");
    println!("[master] this PoC is unencrypted; use only on a trusted LAN");

    send_event(Event::Hello);
    // Immediate liveness probe so the first edge push has a fresh response.
    send_ping(&socket);

    let timer = if seconds > 0 {
        println!("[master] stopping in {seconds}s");
        Some(thread::spawn(move || {
            thread::sleep(Duration::from_secs(seconds));
            post_main(WM_LANKVM_STOP);
        }))
    } else {
        None
    };

    let guard = CursorGuard;

    let mut msg: MSG = unsafe { zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if ret <= 0 {
            break;
        }
        match msg.message {
            WM_LANKVM_RELEASE => {
                let reason = reason_name(RELEASE_REASON.load(Ordering::SeqCst));
                do_release(reason);
            }
            WM_LANKVM_STOP => break,
            _ => unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }

    if let Some(timer) = timer {
        let _ = timer.join();
    }

    do_release("stop");
    stop.store(true, Ordering::SeqCst);
    unsafe {
        UnhookWindowsHookEx(hooks.0);
        UnhookWindowsHookEx(hooks.1);
        DestroyWindow(hwnd);
    }
    drop(guard);

    let _ = sender_thread.join();
    let _ = receiver_thread.join();
    let _ = heartbeat_thread.join();
    let _ = watchdog_thread.join();

    println!("[master] stopped");
    Ok(())
}

pub fn run_client(port: u16, seconds: u64, release_px: i32) -> Result<()> {
    win::enable_dpi_awareness();
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    // A 1-2 px overshoot releases on the slightest edge contact; require a
    // deliberate push.
    let release_px = release_px.max(6);

    let socket = UdpSocket::bind(("0.0.0.0", port))
        .with_context(|| format!("failed to bind UDP port {port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
    let sock = socket2::SockRef::from(&socket);
    let _ = sock.set_recv_buffer_size(1 << 20);
    let _ = sock.set_send_buffer_size(1 << 20);

    let stop = Arc::new(AtomicBool::new(false));
    let timer = if seconds > 0 {
        let stop = stop.clone();
        Some(thread::spawn(move || {
            thread::sleep(Duration::from_secs(seconds));
            stop.store(true, Ordering::SeqCst);
        }))
    } else {
        None
    };

    println!("[client] listening on 0.0.0.0:{port}");
    println!("[client] this PoC is unencrypted; use only on a trusted LAN");

    let mut controlled = false;
    let mut virtual_x = 0i32;
    let mut entry_edge = 1i32;
    let mut edge_hits = 0u32;
    let mut pressed: HashSet<(u16, bool)> = HashSet::new();
    let mut shift_down = false;
    let mut seq = 0u32;
    let mut master_addr: Option<SocketAddr> = None;
    let mut keepalive_to: Option<(SocketAddr, Instant)> = None;
    let mut last_keepalive = Instant::now();
    let mut last_packet = Instant::now();
    let mut buf = [0u8; 64];

    while !stop.load(Ordering::SeqCst) && !STOP_REQUESTED.load(Ordering::SeqCst) {
        if controlled && last_packet.elapsed() > Duration::from_secs(15) {
            controlled = false;
            release_keys(&mut pressed);
            shift_down = false;
            plog!("[client] master silent 15 s; released keys");
        }
        // Unsolicited keepalive so the master's liveness does not depend on its
        // PING surviving the network. Only armed by real input, and dropped
        // after 5 s, so we never spray a closed port (ICMP -> WSAECONNRESET).
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

        let (n, addr) = match socket.recv_from(&mut buf) {
            Ok(value) => value,
            Err(err) if is_ignorable_udp_error(&err) => continue,
            Err(err) => return Err(err).context("client UDP receive failed"),
        };

        let Some(packet) = Packet::decode(&buf[..n]) else {
            continue;
        };
        last_packet = Instant::now();
        master_addr = Some(addr);
        if !matches!(packet.kind, PacketKind::Ping | PacketKind::Pong) {
            keepalive_to = Some((addr, Instant::now()));
        }

        match packet.kind {
            PacketKind::Hello => {
                println!("[client] master connected from {addr}");
            }
            PacketKind::Switch => {
                let bounds = win::virtual_bounds()?;
                entry_edge = packet.b.clamp(1, 2);
                let ratio = packet.a.clamp(0, 65_535) as f32 / 65_535.0;
                let y = bounds.top + (ratio * (bounds.height().max(2) - 1) as f32).round() as i32;
                let x = if entry_edge == 2 {
                    bounds.right - 1
                } else {
                    bounds.left
                };
                win::warp_cursor(x, y);
                controlled = true;
                virtual_x = 0;
                edge_hits = 0;
                release_keys(&mut pressed);
                shift_down = false;
                plog!(
                    "[client] controlled; entry at ({x}, {y}) from {}",
                    if entry_edge == 2 { "right" } else { "left" }
                );
            }
            PacketKind::Motion => {
                if !controlled {
                    continue;
                }
                win::move_relative(packet.a, packet.b);
                virtual_x = virtual_x.saturating_add(packet.a);
                let outward = if entry_edge == 1 {
                    packet.a < 0
                } else {
                    packet.a > 0
                };
                let past = if entry_edge == 1 {
                    virtual_x < -release_px
                } else {
                    virtual_x > release_px
                };
                if outward && past {
                    edge_hits += 1;
                } else {
                    edge_hits = 0;
                }
                // Two consecutive outward packets: a single flick near the
                // edge should not hand control back.
                if edge_hits >= 2 {
                    send_release(&socket, addr, &mut seq, 5);
                    controlled = false;
                    edge_hits = 0;
                    release_keys(&mut pressed);
                    shift_down = false;
                    plog!("[client] released at edge");
                }
            }
            PacketKind::Button => {
                if controlled {
                    win::mouse_button(packet.a as u8, packet.b != 0);
                }
            }
            PacketKind::Key => {
                if controlled {
                    let scan = packet.a as u16;
                    let down = packet.b != 0;
                    let is_shift = matches!(scan, 0x2A | 0x36);
                    let shift = packet.flags & 1 != 0;
                    if is_shift {
                        if down != shift_down {
                            win::key_scancode(0x2A, down, false);
                            shift_down = down;
                            if down {
                                pressed.insert((0x2A, false));
                            } else {
                                pressed.remove(&(0x2A, false));
                            }
                        }
                    } else {
                        if shift != shift_down {
                            win::key_scancode(0x2A, shift, false);
                            shift_down = shift;
                            if shift {
                                pressed.insert((0x2A, false));
                            } else {
                                pressed.remove(&(0x2A, false));
                            }
                        }
                        let extended = packet.c != 0;
                        win::key_scancode(scan, down, extended);
                        if down {
                            pressed.insert((scan, extended));
                        } else {
                            pressed.remove(&(scan, extended));
                        }
                    }
                }
            }
            PacketKind::Wheel => {
                if controlled {
                    win::wheel(packet.a, packet.b != 0);
                }
            }
            PacketKind::Release => {
                if controlled {
                    controlled = false;
                    release_keys(&mut pressed);
                    shift_down = false;
                    plog!("[client] released by master");
                }
            }
            PacketKind::Ping => {
                let mut pong = Packet::new(PacketKind::Pong, seq);
                seq = seq.wrapping_add(1);
                pong.a = packet.a;
                let _ = socket.send_to(&pong.encode(), addr);
            }
            PacketKind::Pong => {}
        }
    }

    if controlled {
        if let Some(addr) = master_addr {
            send_release(&socket, addr, &mut seq, 4);
        }
        release_keys(&mut pressed);
    }
    if let Some(timer) = timer {
        let _ = timer.join();
    }
    println!("[client] stopped");
    Ok(())
}

fn sender_loop(socket: UdpSocket, rx: Receiver<Event>, stop: Arc<AtomicBool>) {
    let mut seq = 0u32;
    let mut last_motion = Instant::now() - MOTION_INTERVAL;
    while !stop.load(Ordering::SeqCst) {
        // Coalesce raw mouse motion into at most one packet per MOTION_INTERVAL.
        // This keeps the wire rate bounded even with a 1000 Hz mouse and stops
        // the send queue from growing during a network stall.
        if REMOTE.load(Ordering::SeqCst)
            && MOTION_DIRTY.load(Ordering::SeqCst)
            && last_motion.elapsed() >= MOTION_INTERVAL
        {
            let dx = PENDING_DX.swap(0, Ordering::SeqCst);
            let dy = PENDING_DY.swap(0, Ordering::SeqCst);
            MOTION_DIRTY.store(false, Ordering::SeqCst);
            // A raw event may have landed between the swap and the store above.
            if PENDING_DX.load(Ordering::SeqCst) != 0 || PENDING_DY.load(Ordering::SeqCst) != 0 {
                MOTION_DIRTY.store(true, Ordering::SeqCst);
            }
            if dx != 0 || dy != 0 {
                let mut packet = Packet::new(PacketKind::Motion, seq);
                seq = seq.wrapping_add(1);
                packet.a = dx.clamp(-2000, 2000);
                packet.b = dy.clamp(-2000, 2000);
                if let Err(err) = socket.send(&packet.encode()) {
                    eprintln!("[net] send failed: {err}");
                } else {
                    TX_PACKETS.fetch_add(1, Ordering::Relaxed);
                }
            }
            last_motion = Instant::now();
        }

        let event = match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(event) => event,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };
        let mut packet = match event {
            Event::Hello => Packet::new(PacketKind::Hello, seq),
            Event::Switch { ratio, entry_edge } => {
                let mut p = Packet::new(PacketKind::Switch, seq);
                p.a = ratio as i32;
                p.b = entry_edge;
                p
            }
            Event::Button { button, down } => {
                let mut p = Packet::new(PacketKind::Button, seq);
                p.a = button as i32;
                p.b = down as i32;
                p
            }
            Event::Key {
                scan,
                down,
                extended,
                shift,
            } => {
                let mut p = Packet::new(PacketKind::Key, seq);
                p.a = scan as i32;
                p.b = down as i32;
                p.c = extended as i32;
                p.flags = if shift { 1 } else { 0 };
                p
            }
            Event::Wheel { delta, horizontal } => {
                let mut p = Packet::new(PacketKind::Wheel, seq);
                p.a = delta;
                p.b = horizontal as i32;
                p
            }
            Event::Release { reason } => {
                let mut p = Packet::new(PacketKind::Release, seq);
                p.a = reason_code(reason);
                p
            }
        };
        packet.seq = seq;
        seq = seq.wrapping_add(1);
        if let Err(err) = socket.send(&packet.encode()) {
            eprintln!("[net] send failed: {err}");
        } else {
            TX_PACKETS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn send_ping(socket: &UdpSocket) {
    let mut packet = Packet::new(PacketKind::Ping, 0);
    // Timestamp taken right before the send, so RTT is not inflated by any
    // queueing on our side.
    packet.a = now_ms() as i32;
    if socket.send(&packet.encode()).is_ok() {
        TX_PACKETS.fetch_add(1, Ordering::Relaxed);
    }
}

fn receiver_loop(socket: UdpSocket, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::SeqCst) {
        match socket.recv_from(&mut buf) {
            Ok((n, _addr)) => {
                let Some(packet) = Packet::decode(&buf[..n]) else {
                    continue;
                };
                match packet.kind {
                    PacketKind::Pong => {
                        let now = now_ms();
                        if LAST_SEEN_MS.swap(now, Ordering::SeqCst) == 0 {
                            plog!("[master] client online");
                        }
                        RX_PACKETS.fetch_add(1, Ordering::Relaxed);
                        let sent = packet.a.max(0) as u64;
                        // a == 0 is the client's unsolicited keepalive.
                        if sent != 0 {
                            let rtt = now.saturating_sub(sent);
                            LAST_RTT_MS.store(rtt, Ordering::SeqCst);
                            RTT_MAX_WINDOW.fetch_max(rtt, Ordering::SeqCst);
                            let last_log = LAST_RTT_LOG_MS.load(Ordering::SeqCst);
                            if now.saturating_sub(last_log) >= 1_000 {
                                LAST_RTT_LOG_MS.store(now, Ordering::SeqCst);
                                plog!("[latency] rtt={rtt} ms");
                            }
                        }
                    }
                    PacketKind::Release => {
                        LAST_SEEN_MS.store(now_ms(), Ordering::SeqCst);
                        RX_PACKETS.fetch_add(1, Ordering::Relaxed);
                        post_release(reason_name(packet.a.max(0) as u32));
                    }
                    PacketKind::Hello => {
                        println!("[net] client hello");
                    }
                    _ => {}
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionRefused
                ) =>
            {
                continue
            }
            Err(err) => {
                eprintln!("[net] receive failed: {err}");
                break;
            }
        }
    }
}

/// Independent liveness check. Runs in its own thread and never performs I/O,
/// so the master always hands control back even if the network stack hangs.
fn watchdog_loop(stop: Arc<AtomicBool>) {
    let mut soft_since: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(250));
        if !REMOTE.load(Ordering::SeqCst) {
            soft_since = None;
            continue;
        }
        let last = LAST_SEEN_MS.load(Ordering::SeqCst);
        let age = now_ms().saturating_sub(last);
        if last == 0 || age <= PEER_TIMEOUT_MS {
            soft_since = None;
            continue;
        }
        match soft_since {
            None => {
                soft_since = Some(Instant::now());
                plog!("[master] watchdog: peer silent for {age} ms; releasing control");
                post_release("timeout");
            }
            Some(since) if since.elapsed() > Duration::from_secs(5) => {
                // The soft release did not reach the message loop. Killing the
                // process is the only guaranteed way to drop ClipCursor, the
                // cursor-hider window and the hidden-cursor count.
                plog!("[master] watchdog: release did not take effect; terminating the engine");
                std::process::exit(4);
            }
            _ => {}
        }
    }
}

fn heartbeat_loop(socket: UdpSocket, stop: Arc<AtomicBool>) {
    let mut last_stats = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        // 4 Hz while controlling (fast failure detection), 1 Hz when idle.
        let interval_ms = if REMOTE.load(Ordering::SeqCst) { 250 } else { 1_000 };
        let mut slept = 0u64;
        while slept < interval_ms {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
            slept += 50;
        }

        send_ping(&socket);

        if last_stats.elapsed() >= Duration::from_secs(5) {
            last_stats = Instant::now();
            let motion = REMOTE_MOTION.swap(0, Ordering::Relaxed);
            let tx = TX_PACKETS.swap(0, Ordering::Relaxed);
            let rx = RX_PACKETS.swap(0, Ordering::Relaxed);
            let max_rtt = RTT_MAX_WINDOW.swap(0, Ordering::Relaxed);
            let last_rtt = LAST_RTT_MS.load(Ordering::Relaxed);
            let last = LAST_SEEN_MS.load(Ordering::Relaxed);
            let age = if last == 0 {
                0
            } else {
                now_ms().saturating_sub(last)
            };
            if REMOTE.load(Ordering::SeqCst) {
                plog!("[master] remote motion {motion}/s");
            }
            plog!(
                "[net] tx={tx} rx={rx} rtt={last_rtt}ms rtt_max={max_rtt}ms motion={motion}/s silent={age}ms remote={}",
                REMOTE.load(Ordering::SeqCst)
            );
        }
    }
}

fn install_hooks() -> Result<(HHOOK, HHOOK)> {
    let module = unsafe { GetModuleHandleW(null()) };
    let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
    if keyboard.is_null() {
        return Err(anyhow!(
            "SetWindowsHookExW(WH_KEYBOARD_LL) failed: {}",
            io::Error::last_os_error()
        ));
    }
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if mouse.is_null() {
        unsafe {
            UnhookWindowsHookEx(keyboard);
        }
        return Err(anyhow!(
            "SetWindowsHookExW(WH_MOUSE_LL) failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok((keyboard, mouse))
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && REMOTE.load(Ordering::SeqCst) {
        let event = &*(lparam as *const KBDLLHOOKSTRUCT);
        let injected = event.flags & LLKHF_INJECTED != 0;
        if !injected {
            let message = wparam as u32;
            let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
            let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
            if down && event.vkCode == VK_Q as u32 && hotkey_modifiers_down() {
                post_release("hotkey");
                return 1;
            }
            if down || up {
                // Modifier state travels with every key so the client can
                // re-assert Shift if a modifier packet was lost.
                let shift = unsafe { GetAsyncKeyState(VK_SHIFT as i32) < 0 };
                send_event(Event::Key {
                    scan: event.scanCode as u16,
                    down,
                    extended: event.flags & LLKHF_EXTENDED != 0,
                    shift,
                });
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && REMOTE.load(Ordering::SeqCst) {
        let event = &*(lparam as *const MSLLHOOKSTRUCT);
        let injected = event.flags & LLMHF_INJECTED != 0;
        if !injected {
            let message = wparam as u32;
            match message {
                WM_LBUTTONDOWN | WM_LBUTTONUP => {
                    send_event(Event::Button {
                        button: 1,
                        down: message == WM_LBUTTONDOWN,
                    });
                    return 1;
                }
                WM_RBUTTONDOWN | WM_RBUTTONUP => {
                    send_event(Event::Button {
                        button: 2,
                        down: message == WM_RBUTTONDOWN,
                    });
                    return 1;
                }
                WM_MBUTTONDOWN | WM_MBUTTONUP => {
                    send_event(Event::Button {
                        button: 3,
                        down: message == WM_MBUTTONDOWN,
                    });
                    return 1;
                }
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let xbutton = ((event.mouseData >> 16) & 0xFFFF) as u8;
                    send_event(Event::Button {
                        button: if xbutton == 2 { 5 } else { 4 },
                        down: message == WM_XBUTTONDOWN,
                    });
                    return 1;
                }
                WM_MOUSEWHEEL => {
                    let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
                    send_event(Event::Wheel {
                        delta,
                        horizontal: false,
                    });
                    return 1;
                }
                WM_MOUSEHWHEEL => {
                    let delta = ((event.mouseData >> 16) as u16) as i16 as i32;
                    send_event(Event::Wheel {
                        delta,
                        horizontal: true,
                    });
                    return 1;
                }
                WM_MOUSEMOVE => {}
                _ => {}
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe fn hotkey_modifiers_down() -> bool {
    GetAsyncKeyState(VK_CONTROL as i32) < 0
        && GetAsyncKeyState(VK_MENU as i32) < 0
        && GetAsyncKeyState(VK_SHIFT as i32) < 0
}

fn create_raw_window() -> Result<HWND> {
    let module = unsafe { GetModuleHandleW(null()) };
    let class_name: Vec<u16> = "LankvmRawInputPoc\0".encode_utf16().collect();
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(raw_window_proc),
        hInstance: module,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { zeroed() }
    };

    let atom = unsafe { RegisterClassW(&window_class) };
    if atom == 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(1410) {
            return Err(anyhow!("RegisterClassW failed: {err}"));
        }
    }

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            module,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(anyhow!(
            "CreateWindowExW failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(hwnd)
}

fn register_raw_mouse(hwnd: HWND) -> Result<()> {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x02,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    let ok = unsafe { RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) };
    if ok == 0 {
        return Err(anyhow!(
            "RegisterRawInputDevices failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

unsafe extern "system" fn raw_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_INPUT {
        handle_raw_input(lparam);
        return 0;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn handle_raw_input(lparam: LPARAM) {
    let mut raw: RAWINPUT = unsafe { zeroed() };
    let mut size = size_of::<RAWINPUT>() as u32;
    let got = unsafe {
        GetRawInputData(
            lparam as *mut c_void,
            RID_INPUT,
            &mut raw as *mut _ as *mut c_void,
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if got == 0 || got == u32::MAX {
        return;
    }
    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }

    let mouse = unsafe { raw.data.mouse };
    if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
        // Absolute raw input (tablet/touchpad); the P0 path only handles
        // relative mouse motion.
        return;
    }
    let dx = mouse.lLastX;
    let dy = mouse.lLastY;
    if dx == 0 && dy == 0 {
        return;
    }

    if REMOTE.load(Ordering::SeqCst) {
        REMOTE_MOTION.fetch_add(1, Ordering::Relaxed);
        // Accumulate; the sender coalesces this into one packet per few ms.
        PENDING_DX.fetch_add(dx.clamp(-500, 500), Ordering::Relaxed);
        PENDING_DY.fetch_add(dy.clamp(-500, 500), Ordering::Relaxed);
        MOTION_DIRTY.store(true, Ordering::Relaxed);
        return;
    }

    let Some(bounds) = BOUNDS.get() else {
        return;
    };
    if unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0 {
        return;
    }
    let Ok(cursor) = win::cursor_pos() else {
        return;
    };
    let trigger = {
        let Some(detector) = EDGE.get() else {
            return;
        };
        let mut detector = detector.lock().unwrap_or_else(|e| e.into_inner());
        detector.update(cursor, *bounds, dx as f32, dy as f32, now_ms())
    };
    if trigger {
        do_switch(cursor, *bounds);
    }
}

fn do_switch(cursor: Point, bounds: Rect) {
    if REMOTE.load(Ordering::SeqCst) {
        return;
    }

    // Do not hide/pin the local cursor unless the peer has answered recently.
    // This avoids a black-hole state when the IP, firewall, or client is wrong.
    let last_seen = LAST_SEEN_MS.load(Ordering::SeqCst);
    let age = now_ms().saturating_sub(last_seen);
    if last_seen == 0 || age > PEER_TIMEOUT_MS {
        let now = now_ms();
        if now.saturating_sub(LAST_LOCAL_WARN_MS.load(Ordering::SeqCst)) > 2_000 {
            LAST_LOCAL_WARN_MS.store(now, Ordering::SeqCst);
            plog!("[master] peer unreachable (silent {age} ms); staying local");
        }
        if let Some(detector) = EDGE.get() {
            detector.lock().unwrap_or_else(|e| e.into_inner()).reset();
        }
        return;
    }

    if REMOTE.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(edge) = EDGE_KIND.get().copied() else {
        REMOTE.store(false, Ordering::SeqCst);
        return;
    };
    let ratio = if bounds.height() <= 1 {
        0
    } else {
        (((cursor.y - bounds.top) as i64 * 65_535) / (bounds.height() as i64 - 1)).clamp(0, 65_535)
            as u16
    };
    let entry_edge = if edge == Edge::Right { 1 } else { 2 };
    // Keep the real cursor pinned to a single pixel at the edge; without this
    // it slides up/down along the full-height clip while operating the peer.
    let clip = Rect::from_xywh(
        if edge == Edge::Right {
            bounds.right - 1
        } else {
            bounds.left
        },
        cursor.y.clamp(bounds.top, bounds.bottom.saturating_sub(1)),
        1,
        1,
    );

    win::clip_to(clip);
    win::set_cursor_visible(false);
    let hider = win::create_cursor_hider(bounds);
    if let Some(hwnd) = hider {
        CURSOR_HIDER.store(hwnd as isize, Ordering::SeqCst);
    }
    CURSOR_HIDDEN.store(true, Ordering::SeqCst);
    LAST_SEEN_MS.store(now_ms(), Ordering::SeqCst);
    // Start from a clean accumulator: the click that crossed the edge should
    // not be replayed as remote motion.
    PENDING_DX.store(0, Ordering::SeqCst);
    PENDING_DY.store(0, Ordering::SeqCst);
    MOTION_DIRTY.store(false, Ordering::SeqCst);
    send_event(Event::Switch { ratio, entry_edge });

    plog!(
        "[master] remote control ON ({:?} edge, entry ratio {}, cursor hider {})",
        edge,
        ratio,
        if hider.is_some() { "on" } else { "failed" }
    );
}

fn restore_cursor() {
    let hwnd = CURSOR_HIDER.swap(0, Ordering::SeqCst);
    if hwnd != 0 {
        win::destroy_cursor_hider(hwnd as HWND);
    }
    if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
        win::set_cursor_visible(true);
    }
}

fn do_release(reason: &'static str) {
    if !REMOTE.swap(false, Ordering::SeqCst) {
        return;
    }
    send_event(Event::Release { reason });
    win::release_clip();
    restore_cursor();
    PENDING_DX.store(0, Ordering::SeqCst);
    PENDING_DY.store(0, Ordering::SeqCst);
    MOTION_DIRTY.store(false, Ordering::SeqCst);

    // A detector that has fired stays fired until reset; without this the
    // second edge crossing (and every one after it) would be ignored.
    if let Some(detector) = EDGE.get() {
        detector
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset();
    }

    if let (Some(bounds), Some(edge), Ok(cursor)) =
        (BOUNDS.get(), EDGE_KIND.get(), win::cursor_pos())
    {
        let x = if *edge == Edge::Right {
            (bounds.right - 40).max(bounds.left)
        } else {
            (bounds.left + 40).min(bounds.right - 1)
        };
        win::warp_cursor(x, cursor.y);
    }
    let last = LAST_SEEN_MS.load(Ordering::SeqCst);
    let silent = if last == 0 {
        0
    } else {
        now_ms().saturating_sub(last)
    };
    plog!("[master] remote control OFF ({reason}, peer silent {silent} ms)");
}

fn send_event(event: Event) {
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.send(event);
    }
}

fn send_release(socket: &UdpSocket, addr: SocketAddr, seq: &mut u32, reason: i32) {
    let mut packet = Packet::new(PacketKind::Release, *seq);
    *seq = seq.wrapping_add(1);
    packet.a = reason;
    let _ = socket.send_to(&packet.encode(), addr);
}

fn release_keys(pressed: &mut HashSet<(u16, bool)>) {
    for (scan, extended) in pressed.drain() {
        win::key_scancode(scan, false, extended);
    }
}

fn post_main(message: u32) {
    let tid = MAIN_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            PostThreadMessageW(tid, message, 0, 0);
        }
    }
}

fn post_release(reason: &'static str) {
    RELEASE_REASON.store(reason_code(reason) as u32, Ordering::SeqCst);
    post_main(WM_LANKVM_RELEASE);
}

fn now_ms() -> u64 {
    START
        .get()
        .map(|start| start.elapsed().as_millis() as u64)
        .unwrap_or(0)
}

/// Windows surfaces ICMP port-unreachable as WSAECONNRESET (or a one-off
/// WSAETIMEDOUT) on the next UDP receive. Treat those, and the read timeout,
/// as a lost datagram, not a fatal error.
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

fn reason_code(reason: &str) -> i32 {
    match reason {
        "hotkey" => 1,
        "peer" => 2,
        "timeout" => 3,
        "stop" => 4,
        "edge" => 5,
        "local" => 6,
        "inject_fail" => 7,
        _ => 0,
    }
}

fn reason_name(code: u32) -> &'static str {
    match code {
        1 => "hotkey",
        2 => "peer",
        3 => "timeout",
        4 => "stop",
        5 => "edge",
        6 => "local",
        7 => "inject_fail",
        _ => "peer",
    }
}

struct CursorGuard;

impl Drop for CursorGuard {
    fn drop(&mut self) {
        win::release_clip();
        restore_cursor();
    }
}
