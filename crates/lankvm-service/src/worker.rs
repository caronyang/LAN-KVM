//! One hook worker per input desktop.
//!
//! The worker opens the active input desktop (Default or Winlogon), attaches
//! its thread to it, installs low-level hooks, and runs a message loop. It
//! deliberately records only counts, never key codes or clipboard data.

use std::collections::HashSet;
use std::ffi::c_void;
use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use lankvm_platform::windows as win;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, SetThreadDesktop, DESKTOP_HOOKCONTROL, DESKTOP_READOBJECTS,
    DESKTOP_WRITEOBJECTS, DF_ALLOWOTHERACCOUNTHOOK,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HC_ACTION, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_MOUSEMOVE, WM_QUIT, WM_SYSKEYDOWN,
};

use crate::inject::{self, InjectEvent};
use crate::logger::log;

static KEY_COUNT: AtomicU64 = AtomicU64::new(0);
static MOUSE_COUNT: AtomicU64 = AtomicU64::new(0);
static INJECT_COUNT: AtomicU64 = AtomicU64::new(0);
static INJECT_FAIL: AtomicU64 = AtomicU64::new(0);

pub struct DesktopWorker {
    thread_id: u32,
    join: JoinHandle<()>,
}

impl DesktopWorker {
    pub fn start(desktop: &str, install_hooks: bool) -> Result<Self> {
        let desktop_name = desktop.to_string();
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = thread::spawn(move || worker_main(&desktop_name, install_hooks, ready_tx));

        let thread_id = match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(id)) => id,
            Ok(Err(err)) => {
                let _ = join.join();
                return Err(anyhow!("hook worker failed: {err}"));
            }
            Err(_) => {
                let _ = join.join();
                return Err(anyhow!("hook worker did not become ready"));
            }
        };
        Ok(Self { thread_id, join })
    }

    pub fn stop(self) {
        let Self { thread_id, join } = self;
        if thread_id != 0 {
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
        }
        let _ = join.join();
    }
}

fn worker_main(desktop: &str, install_hooks: bool, ready: mpsc::Sender<Result<u32, String>>) {
    let desktop_handle = unsafe {
        OpenInputDesktop(
            DF_ALLOWOTHERACCOUNTHOOK,
            0,
            DESKTOP_HOOKCONTROL | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS,
        )
    };
    if desktop_handle.is_null() {
        let err = format!("OpenInputDesktop failed: {}", io::Error::last_os_error());
        let _ = ready.send(Err(err.clone()));
        log("session", format!("{desktop}: {err}"));
        return;
    }

    let attached = unsafe { SetThreadDesktop(desktop_handle) } != 0;
    if !attached {
        let err = format!("SetThreadDesktop failed: {}", io::Error::last_os_error());
        let _ = ready.send(Err(err.clone()));
        log("session", format!("{desktop}: {err}"));
        unsafe { CloseDesktop(desktop_handle) };
        return;
    }

    let mut keyboard = null_mut();
    let mut mouse = null_mut();
    if install_hooks {
        let module = unsafe { GetModuleHandleW(null()) };
        keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
        mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };

        if keyboard.is_null() || mouse.is_null() {
            let err = format!(
                "SetWindowsHookExW failed: keyboard={} mouse={} error={}",
                !keyboard.is_null(),
                !mouse.is_null(),
                io::Error::last_os_error()
            );
            if !keyboard.is_null() {
                unsafe { UnhookWindowsHookEx(keyboard) };
            }
            if !mouse.is_null() {
                unsafe { UnhookWindowsHookEx(mouse) };
            }
            unsafe { CloseDesktop(desktop_handle) };
            let _ = ready.send(Err(err.clone()));
            log("session", format!("{desktop}: {err}"));
            return;
        }
    }

    let (inject_tx, inject_rx) = mpsc::channel();
    let inject_desktop = desktop.to_string();
    let inject_handle = desktop_handle as isize;
    let injector =
        thread::spawn(move || injector_main(inject_rx, inject_desktop, inject_handle));
    inject::set_sender(inject_tx);

    let thread_id = unsafe { GetCurrentThreadId() };
    log(
        "session",
        format!(
            "{desktop}: {} installed (thread={thread_id})",
            if install_hooks {
                "hook worker"
            } else {
                "injector worker"
            }
        ),
    );
    let _ = ready.send(Ok(thread_id));

    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    inject::clear_sender();
    let _ = injector.join();
    unsafe {
        if !keyboard.is_null() {
            UnhookWindowsHookEx(keyboard);
        }
        if !mouse.is_null() {
            UnhookWindowsHookEx(mouse);
        }
        CloseDesktop(desktop_handle);
    }
    log(
        "session",
        format!(
            "{desktop}: {} stopped",
            if install_hooks {
                "hook worker"
            } else {
                "injector worker"
            }
        ),
    );
}

fn injector_main(rx: Receiver<InjectEvent>, desktop: String, desktop_handle: isize) {
    // The hook worker is attached to the active desktop, but a new thread
    // otherwise starts on the process desktop (Default). On the secure
    // Winlogon desktop that makes SendInput fail for every event, so attach
    // the injector to the same desktop before applying input. Keep the
    // working Default path untouched.
    if !desktop.eq_ignore_ascii_case("Default") {
        let attached = unsafe { SetThreadDesktop(desktop_handle as *mut c_void) } != 0;
        let error = if attached {
            0
        } else {
            io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        };
        log(
            "session",
            format!("{desktop}: injector desktop attach={attached} err={error}"),
        );
    } else {
        log("session", "Default: injector stays on default desktop");
    }

    let mut pressed: HashSet<(u16, bool)> = HashSet::new();
    while let Ok(event) = rx.recv() {
        apply(&desktop, event, &mut pressed);
    }
    release_keys(&mut pressed);
}

fn apply(desktop: &str, event: InjectEvent, pressed: &mut HashSet<(u16, bool)>) {
    INJECT_COUNT.fetch_add(1, Ordering::Relaxed);
    match event {
        InjectEvent::Switch { ratio, entry_edge } => {
            release_keys(pressed);
            inject::clear_fail_streak();
            log(
                "session",
                format!("{desktop}: inject Switch entry_edge={entry_edge}"),
            );
            if let Ok(bounds) = win::virtual_bounds() {
                let ratio = ratio as f32 / 65_535.0;
                let y = bounds.top + (ratio * (bounds.height().max(2) - 1) as f32).round() as i32;
                let x = if entry_edge == 2 {
                    bounds.right - 1
                } else {
                    bounds.left
                };
                let _ = win::warp_cursor(x, y);
            }
        }
        InjectEvent::Motion { dx, dy } => {
            let ok = win::move_relative(dx, dy);
            inject::note_result(ok);
            if !ok {
                INJECT_FAIL.fetch_add(1, Ordering::Relaxed);
                log_inject_failure(desktop, "Motion");
            }
        }
        InjectEvent::Button { button, down } => {
            let ok = win::mouse_button(button, down);
            inject::note_result(ok);
            if !ok {
                log_inject_failure(desktop, "Button");
            }
        }
        InjectEvent::Key {
            scan,
            down,
            extended,
        } => {
            let ok = win::key_scancode(scan, down, extended);
            inject::note_result(ok);
            if !ok {
                log_inject_failure(desktop, "Key");
            }
            if down {
                pressed.insert((scan, extended));
            } else {
                pressed.remove(&(scan, extended));
            }
        }
        InjectEvent::Wheel { delta, horizontal } => {
            let ok = win::wheel(delta, horizontal);
            inject::note_result(ok);
            if !ok {
                log_inject_failure(desktop, "Wheel");
            }
        }
        InjectEvent::Release => {
            inject::clear_fail_streak();
            log("session", format!("{desktop}: inject Release"));
            release_keys(pressed);
        }
    }
}

fn log_inject_failure(desktop: &str, kind: &str) {
    let streak = inject::fail_streak();
    if streak == 1 || streak % 25 == 0 {
        log(
            "session",
            format!(
                "{desktop}: inject failed kind={kind} streak={streak} err={}",
                io::Error::last_os_error()
            ),
        );
    }
}

fn release_keys(pressed: &mut HashSet<(u16, bool)>) {
    for (scan, extended) in pressed.drain() {
        let _ = win::key_scancode(scan, false, extended);
    }
}

pub fn take_counts() -> (u64, u64, u64, u64) {
    (
        KEY_COUNT.swap(0, Ordering::Relaxed),
        MOUSE_COUNT.swap(0, Ordering::Relaxed),
        INJECT_COUNT.swap(0, Ordering::Relaxed),
        INJECT_FAIL.swap(0, Ordering::Relaxed),
    )
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam as u32;
        if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
            KEY_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam as u32;
        if message == WM_MOUSEMOVE {
            MOUSE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}
