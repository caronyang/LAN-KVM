//! Windows P0 backend: probing, hooks, and injection.
//!
//! This module is deliberately conservative: it never installs a hook or
//! injects input unless an explicit command asks for it.

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use lankvm_core::{MonitorInfo, Point, Rect};

use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation,
    TokenIntegrityLevel, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSGetActiveConsoleSessionId,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_READOBJECTS,
    DF_ALLOWOTHERACCOUNTHOOK, UOI_NAME,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, OpenProcessToken,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, ClipCursor, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics,
    PostThreadMessageW, SetCursorPos, SetWindowsHookExW, ShowCursor, TranslateMessage,
    UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLMHF_INJECTED, MONITORINFOF_PRIMARY, MSG, MSLLHOOKSTRUCT, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::ProbeReport;

const MAGIC: usize = 0x4C4B_564D; // "LKVM"

static DPI_ONCE: Once = Once::new();
static MOVE_EVENTS: AtomicU64 = AtomicU64::new(0);
static SUPPRESS: AtomicBool = AtomicBool::new(false);

pub fn enable_dpi_awareness() {
    DPI_ONCE.call_once(|| unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

pub fn probe() -> Result<ProbeReport> {
    enable_dpi_awareness();

    let monitors = monitors()?;
    let mut bounds = virtual_bounds()?;
    if bounds.is_empty() {
        bounds = monitors
            .iter()
            .map(|m| m.rect)
            .fold(Rect::default(), |acc, r| acc.union(r));
    }

    let desktop = match active_desktop_name() {
        Ok(name) => (name, None),
        Err(err) => ("unavailable".to_string(), Some(err.to_string())),
    };

    let (elevated, integrity) = elevation_info();

    Ok(ProbeReport {
        pid: unsafe { GetCurrentProcessId() },
        session_id: session_id(),
        active_console_session: unsafe { WTSGetActiveConsoleSessionId() },
        elevated,
        integrity,
        desktop: desktop.0,
        desktop_error: desktop.1,
        monitors,
        virtual_bounds: bounds,
        cursor: cursor_pos()?,
    })
}

pub fn monitors() -> Result<Vec<MonitorInfo>> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(monitor_enum_proc),
            &mut handles as *mut Vec<HMONITOR> as LPARAM,
        )
    };
    if ok == 0 {
        return Err(anyhow!(
            "EnumDisplayMonitors failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mut result = Vec::with_capacity(handles.len());
    for (index, hmon) in handles.into_iter().enumerate() {
        let mut info: MONITORINFOEXW = unsafe { zeroed() };
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let ok = unsafe { GetMonitorInfoW(hmon, &mut info.monitorInfo) };
        if ok == 0 {
            continue;
        }

        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = unsafe {
            GetDpiForMonitor(
                hmon,
                MDT_EFFECTIVE_DPI,
                &mut dpi_x as *mut u32,
                &mut dpi_y as *mut u32,
            )
        };

        let name_len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..name_len]);

        result.push(MonitorInfo {
            id: index as u32,
            name,
            rect: rect_from(info.monitorInfo.rcMonitor),
            work_area: rect_from(info.monitorInfo.rcWork),
            dpi: if dpi_x == 0 { 96 } else { dpi_x },
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }

    if result.is_empty() {
        return Err(anyhow!("no monitors reported by Windows"));
    }
    Ok(result)
}

pub fn virtual_bounds() -> Result<Rect> {
    unsafe {
        Ok(Rect::from_xywh(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        ))
    }
}

pub fn cursor_pos() -> Result<Point> {
    let mut point: POINT = unsafe { zeroed() };
    let ok = unsafe { GetCursorPos(&mut point) };
    if ok == 0 {
        return Err(anyhow!(
            "GetCursorPos failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(Point::new(point.x, point.y))
}

pub fn active_desktop_name() -> Result<String> {
    let desktop = unsafe { OpenInputDesktop(DF_ALLOWOTHERACCOUNTHOOK, 0, DESKTOP_READOBJECTS) };
    if desktop.is_null() {
        return Err(anyhow!(
            "OpenInputDesktop failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mut buffer = [0u16; 256];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            buffer.as_mut_ptr() as *mut c_void,
            (buffer.len() * size_of::<u16>()) as u32,
            &mut needed,
        )
    };
    unsafe {
        CloseDesktop(desktop);
    }

    if ok == 0 {
        return Err(anyhow!(
            "GetUserObjectInformationW failed: {}",
            io::Error::last_os_error()
        ));
    }

    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..len]))
}

pub fn session_id() -> u32 {
    let mut session = 0u32;
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    if ok == 0 {
        0
    } else {
        session
    }
}

fn elevation_info() -> (bool, String) {
    let mut token: HANDLE = null_mut();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 || token.is_null() {
        return (false, "unknown".to_string());
    }

    let mut elevated = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    let elevation_ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevated as *mut TOKEN_ELEVATION as *mut c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } != 0;

    // Use an aligned buffer for TOKEN_MANDATORY_LABEL.
    let mut buffer = [0usize; 32];
    let integrity_ok = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr() as *mut c_void,
            (buffer.len() * size_of::<usize>()) as u32,
            &mut returned,
        )
    } != 0;

    let integrity = if integrity_ok {
        unsafe {
            let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            if label.Label.Sid.is_null() {
                "unknown".to_string()
            } else {
                let count = *GetSidSubAuthorityCount(label.Label.Sid);
                if count == 0 {
                    "unknown".to_string()
                } else {
                    let rid = *GetSidSubAuthority(label.Label.Sid, (count - 1) as u32);
                    integrity_name(rid).to_string()
                }
            }
        }
    } else {
        "unknown".to_string()
    };

    unsafe {
        CloseHandle(token);
    }

    (elevation_ok && elevated.TokenIsElevated != 0, integrity)
}

fn integrity_name(rid: u32) -> &'static str {
    match rid {
        0x0000 => "Untrusted",
        0x1000 => "Low",
        0x2000 => "Medium",
        0x3000 => "High",
        0x4000 => "System",
        0x5000 => "Protected",
        _ => "Unknown",
    }
}

unsafe extern "system" fn monitor_enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let handles = &mut *(lparam as *mut Vec<HMONITOR>);
    handles.push(hmon);
    1
}

fn rect_from(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

// ---------------------------------------------------------------------------
// Hook test
// ---------------------------------------------------------------------------

pub fn run_hook_test(seconds: u64, suppress: bool) -> Result<()> {
    enable_dpi_awareness();
    SUPPRESS.store(suppress, Ordering::Relaxed);
    if suppress {
        println!("WARNING: suppression is ON. Keyboard/buttons are blocked while this runs.");
        println!("The test stops automatically after {seconds}s; do not run it without a timer.");
    }

    let module = unsafe { GetModuleHandleW(null()) };
    let keyboard_hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), module, 0) };
    if keyboard_hook.is_null() {
        return Err(anyhow!(
            "SetWindowsHookExW(WH_KEYBOARD_LL) failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), module, 0) };
    if mouse_hook.is_null() {
        unsafe {
            UnhookWindowsHookEx(keyboard_hook);
        }
        return Err(anyhow!(
            "SetWindowsHookExW(WH_MOUSE_LL) failed: {}",
            io::Error::last_os_error()
        ));
    }

    println!("Hooks installed. Move/click/type; logging key and button events.");
    println!("Mouse-move events are counted but not printed.");
    if seconds > 0 {
        println!("Stopping in {seconds}s...");
    } else {
        println!("Press Ctrl+C to stop.");
    }

    let thread_id = unsafe { GetCurrentThreadId() };
    let timer = if seconds > 0 {
        Some(thread::spawn(move || {
            thread::sleep(Duration::from_secs(seconds));
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
        }))
    } else {
        None
    };

    let mut msg: MSG = unsafe { zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if ret <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if let Some(timer) = timer {
        let _ = timer.join();
    }

    unsafe {
        UnhookWindowsHookEx(keyboard_hook);
        UnhookWindowsHookEx(mouse_hook);
    }
    SUPPRESS.store(false, Ordering::Relaxed);

    println!(
        "Hooks removed. Mouse-move events observed: {}",
        MOVE_EVENTS.swap(0, Ordering::Relaxed)
    );
    Ok(())
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = &*(lparam as *const KBDLLHOOKSTRUCT);
        let injected = event.flags & LLKHF_INJECTED != 0;
        let message = wparam as u32;
        let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
        if down || up {
            println!(
                "[key] {} vk=0x{:02X} scan=0x{:02X} ext={} injected={} extra=0x{:X}",
                if down { "DOWN" } else { "UP  " },
                event.vkCode,
                event.scanCode,
                (event.flags & LLKHF_EXTENDED) != 0,
                injected,
                event.dwExtraInfo
            );
        }
        if SUPPRESS.load(Ordering::Relaxed) && !injected {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = &*(lparam as *const MSLLHOOKSTRUCT);
        let injected = event.flags & LLMHF_INJECTED != 0;
        let message = wparam as u32;

        if message == WM_MOUSEMOVE {
            MOVE_EVENTS.fetch_add(1, Ordering::Relaxed);
        } else {
            let name = mouse_message_name(message);
            if let Some(name) = name {
                println!(
                    "[mouse] {} at ({}, {}) data=0x{:X} injected={} extra=0x{:X}",
                    name, event.pt.x, event.pt.y, event.mouseData, injected, event.dwExtraInfo
                );
            }
        }

        // Suppress buttons/wheel but never mouse motion: clipping is used to
        // keep the local cursor pinned while remote control is active.
        if SUPPRESS.load(Ordering::Relaxed) && !injected && message != WM_MOUSEMOVE {
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn mouse_message_name(message: u32) -> Option<&'static str> {
    Some(match message {
        WM_LBUTTONDOWN => "LEFT DOWN",
        WM_LBUTTONUP => "LEFT UP",
        WM_RBUTTONDOWN => "RIGHT DOWN",
        WM_RBUTTONUP => "RIGHT UP",
        WM_MBUTTONDOWN => "MIDDLE DOWN",
        WM_MBUTTONUP => "MIDDLE UP",
        WM_XBUTTONDOWN => "XBUTTON DOWN",
        WM_XBUTTONUP => "XBUTTON UP",
        WM_MOUSEWHEEL => "WHEEL",
        WM_MOUSEHWHEEL => "HWHEEL",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Injection and cursor helpers
// ---------------------------------------------------------------------------

pub fn move_relative(dx: i32, dy: i32) -> bool {
    send_mouse(MOUSEINPUT {
        dx,
        dy,
        mouseData: 0,
        dwFlags: MOUSEEVENTF_MOVE,
        time: 0,
        dwExtraInfo: MAGIC,
    })
}

pub fn move_absolute(x: i32, y: i32, bounds: Rect) -> bool {
    let width = bounds.width();
    let height = bounds.height();
    if width <= 1 || height <= 1 {
        return false;
    }
    let nx = normalize(x - bounds.left, width);
    let ny = normalize(y - bounds.top, height);
    send_mouse(MOUSEINPUT {
        dx: nx,
        dy: ny,
        mouseData: 0,
        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        time: 0,
        dwExtraInfo: MAGIC,
    })
}

fn normalize(value: i32, length: i32) -> i32 {
    let v = value.clamp(0, length - 1) as i64;
    ((v * 65_535) / (length as i64 - 1)) as i32
}

pub fn mouse_button(button: u8, down: bool) -> bool {
    let mut data = 0u32;
    let flags = match (button, down) {
        (1, true) => MOUSEEVENTF_LEFTDOWN,
        (1, false) => MOUSEEVENTF_LEFTUP,
        (2, true) => MOUSEEVENTF_RIGHTDOWN,
        (2, false) => MOUSEEVENTF_RIGHTUP,
        (3, true) => MOUSEEVENTF_MIDDLEDOWN,
        (3, false) => MOUSEEVENTF_MIDDLEUP,
        (4, true) => {
            data = 1;
            MOUSEEVENTF_XDOWN
        }
        (4, false) => {
            data = 1;
            MOUSEEVENTF_XUP
        }
        (5, true) => {
            data = 2;
            MOUSEEVENTF_XDOWN
        }
        (5, false) => {
            data = 2;
            MOUSEEVENTF_XUP
        }
        _ => return false,
    };
    send_mouse(MOUSEINPUT {
        dx: 0,
        dy: 0,
        mouseData: data,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: MAGIC,
    })
}

pub fn wheel(delta: i32, horizontal: bool) -> bool {
    send_mouse(MOUSEINPUT {
        dx: 0,
        dy: 0,
        mouseData: delta as u32,
        dwFlags: if horizontal {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        },
        time: 0,
        dwExtraInfo: MAGIC,
    })
}

pub fn key_scancode(scan: u16, down: bool, extended: bool) -> bool {
    let mut flags = KEYEVENTF_SCANCODE;
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    };
    send(input)
}

fn send_mouse(mouse: MOUSEINPUT) -> bool {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 { mi: mouse },
    })
}

fn send(input: INPUT) -> bool {
    unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) == 1 }
}

pub fn warp_cursor(x: i32, y: i32) -> bool {
    unsafe { SetCursorPos(x, y) != 0 }
}

pub fn set_cursor_visible(visible: bool) {
    unsafe {
        for _ in 0..32 {
            let count = ShowCursor(if visible { 1 } else { 0 });
            if (visible && count >= 0) || (!visible && count < 0) {
                break;
            }
        }
    }
}

pub fn clip_to(rect: Rect) {
    let native = RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    unsafe {
        ClipCursor(&native);
    }
}

pub fn release_clip() {
    unsafe {
        ClipCursor(null());
    }
}
