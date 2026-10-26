use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage,
    IDI_APPLICATION, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
    TPM_TOPALIGN, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
};

use crate::state::{save, App};

const WM_TRAY: u32 = WM_APP + 10;
const ID_OPEN: usize = 1001;
const ID_SCAN: usize = 1002;
const ID_CONNECT: usize = 1003;
const ID_LOGS: usize = 1004;
const ID_QUIT: usize = 1005;
const ID_EMERGENCY: usize = 1006;
const TRAY_UID: u32 = 1;
const HOTKEY_ID: i32 = 0x4B01;

static APP: OnceLock<Arc<App>> = OnceLock::new();

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn run(app: Arc<App>) -> ! {
    let _ = APP.set(app.clone());

    unsafe {
        let hinstance = GetModuleHandleW(null());
        let class_name = wide("LankvmTrayWindow");
        let window_name = wide("LAN-KVM");

        let mut icon = LoadIconW(hinstance, 1usize as *const u16);
        if icon.is_null() {
            icon = LoadIconW(null_mut(), IDI_APPLICATION);
        }

        let window_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: icon,
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&window_class) == 0 {
            app.log("tray: RegisterClassW failed");
        }

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance,
            null_mut(),
        );
        if hwnd.is_null() {
            app.log("tray: CreateWindowExW failed; running without tray");
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }

        // Global emergency hotkey. Registered by the app (not the engine) so it
        // still works when the engine is wedged and the cursor is clipped.
        let mods = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
        if RegisterHotKey(hwnd, HOTKEY_ID, mods, b'R' as u32) == 0 {
            app.log("tray: emergency hotkey Ctrl+Alt+Shift+R could not be registered");
        } else {
            app.log("tray: emergency hotkey Ctrl+Alt+Shift+R registered");
        }

        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        set_tip(&mut nid, &status_text(&app));
        if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
            app.log("tray: Shell_NotifyIconW(NIM_ADD) failed");
        } else {
            app.log("tray: icon installed");
        }
        SetTimer(hwnd, 1, 2000, None);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    std::process::exit(0);
}

fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
    let encoded: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let count = encoded.len().min(nid.szTip.len());
    nid.szTip = [0; 128];
    nid.szTip[..count].copy_from_slice(&encoded[..count]);
}

fn status_text(app: &Arc<App>) -> String {
    let session = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    match session {
        Some(session) if session.role == "master" => format!(
            "LAN-KVM - {} ({})",
            tr(app, "主控中", "Controlling"),
            session.peer_ip
        ),
        Some(_) => tr(
            app,
            "LAN-KVM - 被控中（SYSTEM 服務）",
            "LAN-KVM - Controlled (SYSTEM service)",
        )
        .to_string(),
        None if !app.service_ok.load(Ordering::Relaxed) => tr(
            app,
            "LAN-KVM - client 服務未就緒",
            "LAN-KVM - client service not ready",
        )
        .to_string(),
        None => tr(app, "LAN-KVM - 待機", "LAN-KVM - Idle").to_string(),
    }
}

/// The tray follows the product language setting (`config.lang`, default "en"),
/// which the web UI's language selector also writes.
fn tr(app: &Arc<App>, zh_text: &'static str, en_text: &'static str) -> &'static str {
    let is_zh = app
        .config
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lang
        .eq_ignore_ascii_case("zh");
    if is_zh {
        zh_text
    } else {
        en_text
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONUP => open_ui(),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            handle_command(hwnd, id);
            0
        }
        WM_HOTKEY => {
            if let Some(app) = APP.get() {
                crate::engine::emergency_release(app, "hotkey");
            }
            0
        }
        WM_TIMER => {
            if let Some(app) = APP.get() {
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = TRAY_UID;
                nid.uFlags = NIF_TIP;
                set_tip(&mut nid, &status_text(app));
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
            0
        }
        WM_DESTROY => {
            let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn open_ui() {
    if let Some(app) = APP.get() {
        crate::open_browser(&format!("http://127.0.0.1:{}/", app.http_port));
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Some(app) = APP.get() else {
        return;
    };
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }

    let status = status_text(app);
    let status_w = wide(&status);
    AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, status_w.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());

    let open = wide(tr(app, "開啟設定", "Open settings"));
    AppendMenuW(menu, MF_STRING, ID_OPEN, open.as_ptr());
    let scan = wide(tr(app, "重新掃描", "Rescan"));
    AppendMenuW(menu, MF_STRING, ID_SCAN, scan.as_ptr());

    let has_session = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    let connect_label = if has_session {
        wide(tr(app, "停止連線", "Stop connection"))
    } else {
        wide(tr(app, "重新連線", "Reconnect"))
    };
    AppendMenuW(menu, MF_STRING, ID_CONNECT, connect_label.as_ptr());

    let emergency = wide(tr(
        app,
        "強制取回滑鼠 (Ctrl+Alt+Shift+R)",
        "Force release mouse (Ctrl+Alt+Shift+R)",
    ));
    AppendMenuW(menu, MF_STRING, ID_EMERGENCY, emergency.as_ptr());

    let logs = wide(tr(app, "收集紀錄", "Collect logs"));
    AppendMenuW(menu, MF_STRING, ID_LOGS, logs.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let quit = wide(tr(app, "結束", "Quit"));
    AppendMenuW(menu, MF_STRING, ID_QUIT, quit.as_ptr());

    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
}

fn handle_command(hwnd: HWND, id: usize) {
    let Some(app) = APP.get() else {
        return;
    };
    match id {
        ID_OPEN => open_ui(),
        ID_SCAN => crate::net::announce_now(app),
        ID_CONNECT => {
            let has_session = app
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            if has_session {
                crate::engine::stop_session(app, true);
                *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Instant::now() + Duration::from_secs(60));
                app.log("tray: session stopped");
            } else {
                *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
                {
                    let mut config = app.config.lock().unwrap_or_else(|e| e.into_inner());
                    config.paused = false;
                    save(app.profile.as_deref(), &config);
                }
                app.log("tray: reconnect requested");
            }
        }
        ID_EMERGENCY => crate::engine::emergency_release(app, "tray"),
        ID_LOGS => match crate::logs::collect(app) {
            Ok(path) => app.log(format!("tray: logs at {}", path.display())),
            Err(err) => app.log(format!("tray: log collection failed: {err:#}")),
        },
        ID_QUIT => {
            crate::engine::stop_session(app, true);
            app.log("tray: quit");
            unsafe {
                DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
}
