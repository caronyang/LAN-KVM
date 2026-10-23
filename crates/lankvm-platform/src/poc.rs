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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
static LAST_PONG_MS: AtomicU64 = AtomicU64::new(0);
static REMOTE_MOTION: AtomicU64 = AtomicU64::new(0);
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static RELEASE_REASON: AtomicU32 = AtomicU32::new(0);
static START: OnceLock<Instant> = OnceLock::new();
static BOUNDS: OnceLock<Rect> = OnceLock::new();
static EDGE_KIND: OnceLock<Edge> = OnceLock::new();
static EDGE: OnceLock<Mutex<EdgeDetector>> = OnceLock::new();

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
    Motion {
        dx: i32,
        dy: i32,
    },
    Button {
        button: u8,
        down: bool,
    },
    Key {
        scan: u16,
        down: bool,
        extended: bool,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Release {
        reason: &'static str,
    },
    Ping {
        ms: u32,
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

    let socket =
        UdpSocket::bind(("0.0.0.0", 0)).with_context(|| "failed to bind master UDP socket")?;
    socket
        .connect((peer, port))
        .with_context(|| format!("failed to resolve/connect to {peer}:{port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;

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

    let heartbeat_stop = stop.clone();
    let heartbeat_thread = thread::spawn(move || heartbeat_loop(heartbeat_stop));

    let hwnd = create_raw_window()?;
    register_raw_mouse(hwnd)?;
    let hooks = install_hooks()?;

    println!("[master] peer {peer}:{port}, edge {:?}", edge);
    println!("[master] push through the {:?} edge to take control", edge);
    println!("[master] release hotkey: Ctrl+Alt+Shift+Q");
    println!("[master] this PoC is unencrypted; use only on a trusted LAN");

    send_event(Event::Hello);
    // Immediate liveness probe so the first edge push has a fresh PONG.
    send_event(Event::Ping {
        ms: now_ms() as u32,
    });

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

    println!("[master] stopped");
    Ok(())
}

pub fn run_client(port: u16, seconds: u64, release_px: i32) -> Result<()> {
    win::enable_dpi_awareness();
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let release_px = release_px.max(1);

    let socket = UdpSocket::bind(("0.0.0.0", port))
        .with_context(|| format!("failed to bind UDP port {port}"))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;

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
    let mut pressed: HashSet<(u16, bool)> = HashSet::new();
    let mut seq = 0u32;
    let mut master_addr: Option<SocketAddr> = None;
    let mut buf = [0u8; 64];

    while !stop.load(Ordering::SeqCst) && !STOP_REQUESTED.load(Ordering::SeqCst) {
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
            Err(err) => return Err(err).context("client UDP receive failed"),
        };

        let Some(packet) = Packet::decode(&buf[..n]) else {
            continue;
        };
        master_addr = Some(addr);

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
                let left_exit = entry_edge == 1 && virtual_x < -release_px;
                let right_exit = entry_edge == 2 && virtual_x > release_px;
                if left_exit || right_exit {
                    send_release(&socket, addr, &mut seq, 5);
                    controlled = false;
                    release_keys(&mut pressed);
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
                    let extended = packet.c != 0;
                    win::key_scancode(scan, down, extended);
                    if down {
                        pressed.insert((scan, extended));
                    } else {
                        pressed.remove(&(scan, extended));
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
    while !stop.load(Ordering::SeqCst) {
        let event = match rx.recv_timeout(Duration::from_millis(100)) {
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
            Event::Motion { dx, dy } => {
                let mut p = Packet::new(PacketKind::Motion, seq);
                p.a = dx;
                p.b = dy;
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
            } => {
                let mut p = Packet::new(PacketKind::Key, seq);
                p.a = scan as i32;
                p.b = down as i32;
                p.c = extended as i32;
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
            Event::Ping { ms } => {
                let mut p = Packet::new(PacketKind::Ping, seq);
                p.a = ms as i32;
                p
            }
        };
        packet.seq = seq;
        seq = seq.wrapping_add(1);
        if let Err(err) = socket.send(&packet.encode()) {
            eprintln!("[net] send failed: {err}");
        }
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
                        let sent = packet.a.max(0) as u64;
                        let now = now_ms();
                        let rtt = now.saturating_sub(sent);
                        if LAST_PONG_MS.swap(now, Ordering::SeqCst) == 0 {
                            plog!("[master] client online");
                        }
                        plog!("[latency] rtt={rtt} ms");
                    }
                    PacketKind::Release => {
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
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
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

fn heartbeat_loop(stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        for _ in 0..10 {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        send_event(Event::Ping {
            ms: now_ms() as u32,
        });
        if REMOTE.load(Ordering::SeqCst) {
            let motion = REMOTE_MOTION.swap(0, Ordering::SeqCst);
            plog!("[master] remote motion {motion}/s");
            let last = LAST_PONG_MS.load(Ordering::SeqCst);
            if last != 0 && now_ms().saturating_sub(last) > 3_000 {
                post_release("timeout");
            }
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
                send_event(Event::Key {
                    scan: event.scanCode as u16,
                    down,
                    extended: event.flags & LLKHF_EXTENDED != 0,
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
        send_event(Event::Motion {
            dx: dx.clamp(-500, 500),
            dy: dy.clamp(-500, 500),
        });
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
    let last_pong = LAST_PONG_MS.load(Ordering::SeqCst);
    if last_pong == 0 || now_ms().saturating_sub(last_pong) > 3_000 {
        plog!("[master] client not reachable (no recent PONG); staying local");
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
    let clip = Rect::from_xywh(
        if edge == Edge::Right {
            bounds.right - 1
        } else {
            bounds.left
        },
        bounds.top,
        1,
        bounds.height().max(1),
    );

    win::clip_to(clip);
    win::set_cursor_visible(false);
    CURSOR_HIDDEN.store(true, Ordering::SeqCst);
    LAST_PONG_MS.store(now_ms(), Ordering::SeqCst);
    send_event(Event::Switch { ratio, entry_edge });

    plog!(
        "[master] remote control ON ({:?} edge, entry ratio {})",
        edge,
        ratio
    );
}

fn do_release(reason: &'static str) {
    if !REMOTE.swap(false, Ordering::SeqCst) {
        return;
    }
    send_event(Event::Release { reason });
    win::release_clip();
    if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
        win::set_cursor_visible(true);
    }

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
    plog!("[master] remote control OFF ({reason})");
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
        if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
            win::set_cursor_visible(true);
        }
    }
}
