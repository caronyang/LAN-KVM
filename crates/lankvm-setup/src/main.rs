#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
static APP_BYTES: &[u8] = include_bytes!("../../../target/release/lankvm-app.exe");
#[cfg(windows)]
static SERVICE_BYTES: &[u8] = include_bytes!("../../../target/release/lankvm-service.exe");

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use std::net::TcpStream;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, SW_SHOWNORMAL,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn message(text: &str, error: bool) {
        let title = wide("LAN-KVM 安裝程式");
        let body = wide(text);
        let flags = if error {
            MB_OK | MB_ICONERROR
        } else {
            MB_OK | MB_ICONINFORMATION
        };
        unsafe {
            MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), flags);
        }
    }

    fn setup_log(line: &str) {
        use std::io::Write;
        let path = std::env::temp_dir().join("lankvm-setup.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "[{}] {line}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            );
        }
    }

    fn run(program: &str, args: &[&str]) -> bool {
        let ok = Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        setup_log(&format!("{program} {} -> {ok}", args.join(" ")));
        ok
    }

    fn quit_running_app() {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", 45880)) {
            let req = "POST /api/quit HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
            use std::io::Write;
            let _ = stream.write_all(req.as_bytes());
            use std::io::Read;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            thread::sleep(Duration::from_millis(900));
        }
    }

    fn firewall_rule(name: &str, port: u16) -> bool {
        firewall_rule_protocol(name, port, "UDP")
    }

    fn firewall_rule_tcp(name: &str, port: u16) -> bool {
        firewall_rule_protocol(name, port, "TCP")
    }

    fn firewall_rule_protocol(name: &str, port: u16, protocol: &str) -> bool {
        let _ = run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={name}"),
            ],
        );
        let port = port.to_string();
        run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={name}"),
                "dir=in",
                "action=allow",
                &format!("protocol={protocol}"),
                &format!("localport={port}"),
                "profile=any",
            ],
        )
    }

    // Elevate once; the whole install needs the firewall and Program Files.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s == "--extract").unwrap_or(false) {
        if let Some(dir) = args.get(1) {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("lankvm-app.exe"), APP_BYTES);
            let _ = std::fs::write(dir.join("lankvm-service.exe"), SERVICE_BYTES);
        }
        return;
    }

    let is_admin = unsafe { IsUserAnAdmin() } != 0;
    if !is_admin {
        let exe = std::env::current_exe().unwrap_or_default();
        let verb = wide("runas");
        let file = wide(&exe.display().to_string());
        let params = wide("");
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            message("需要系統管理員權限才能安裝（防火牆規則）。", true);
        }
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            message(&format!("找不到安裝程式：{err}"), true);
            return;
        }
    };
    let _ = exe;

    quit_running_app();
    thread::sleep(Duration::from_millis(800));
    // Clear orphan engine clients from older builds; they would otherwise
    // keep UDP 45872 and prevent the SYSTEM service from injecting.
    let _ = run("taskkill.exe", &["/F", "/IM", "lankvm-app.exe", "/T"]);

    // The old P0 service occupies the same ports; remove it if present.
    let _ = run("sc.exe", &["stop", "LAN-KVM"]);
    for _ in 0..40 {
        let running = Command::new("sc.exe")
            .args(["query", "LAN-KVM"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("RUNNING"))
            .unwrap_or(false);
        if !running {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    let _ = run("sc.exe", &["delete", "LAN-KVM"]);
    for _ in 0..80 {
        let exists = Command::new("sc.exe")
            .args(["query", "LAN-KVM"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !exists {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    // Kill leftover session agents that may still own a visible console in the
    // user session (older builds launched them without CREATE_NO_WINDOW).
    let _ = run("taskkill.exe", &["/F", "/IM", "lankvm-service.exe", "/T"]);
    thread::sleep(Duration::from_millis(500));

    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
    let install_dir = PathBuf::from(program_files).join("LAN-KVM");
    if let Err(err) = std::fs::create_dir_all(&install_dir) {
        message(&format!("無法建立安裝目錄：{err}"), true);
        return;
    }

    let app_target = install_dir.join("lankvm-app.exe");
    setup_log(&format!("install app target={}", app_target.display()));
    if let Err(err) = std::fs::write(&app_target, APP_BYTES) {
        message(
            &format!(
                "寫入程式失敗（可能正在執行或被防毒鎖定）：{err}\n請先從系統匣結束 LAN-KVM 再試一次。"
            ),
            true,
        );
        return;
    }
    setup_log(&format!("app bytes written: {}", APP_BYTES.len()));

    // The SYSTEM service is the client engine; it is what makes Winlogon and
    // UAC injection possible. It is embedded in this single setup.exe.
    let service_target = install_dir.join("lankvm-service.exe");
    let service_ok = match std::fs::write(&service_target, SERVICE_BYTES) {
        Ok(_) => {
            setup_log(&format!("service bytes written: {}", SERVICE_BYTES.len()));
            let config_dir = PathBuf::from("C:\\ProgramData\\LAN-KVM");
            if let Err(err) = std::fs::create_dir_all(&config_dir) {
                setup_log(&format!("config dir create failed: {err}"));
            }
            let config_path = config_dir.join("config.txt");
            match std::fs::write(&config_path, "client_port=45872\nrelease_px=10\n") {
                Ok(_) => setup_log(&format!("config written: {}", config_path.display())),
                Err(err) => setup_log(&format!(
                    "config write failed: {}: {err}",
                    config_path.display()
                )),
            }
            // Wait for a previous service to be fully deleted.
            for _ in 0..80 {
                let status = Command::new("sc.exe")
                    .args(["query", "LAN-KVM"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                if status.map(|s| !s.success()).unwrap_or(true) {
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
            let installed = run(&service_target.display().to_string(), &["install"]);
            setup_log(&format!("service install -> {installed}"));
            installed
        }
        Err(err) => {
            setup_log(&format!("service write failed: {err}"));
            false
        }
    };

    // Inbound UDP: discovery, control/pairing, input session.
    let mut firewall_ok = true;
    firewall_ok &= firewall_rule("LAN-KVM Discovery", 45870);
    firewall_ok &= firewall_rule("LAN-KVM Control", 45871);
    firewall_ok &= firewall_rule("LAN-KVM Data", 45872);
    firewall_ok &= firewall_rule_tcp("LAN-KVM Logs", 45873);
    firewall_ok &= firewall_rule_tcp("LAN-KVM Clipboard", 45875);
    setup_log(&format!("firewall_ok={firewall_ok}"));

    // Belt and braces for boot: if SCM does not start the service for any
    // reason, an ONSTART task as SYSTEM starts it, and SCM recovery restarts
    // it if it crashes.
    let _ = run(
        "sc.exe",
        &[
            "failure",
            "LAN-KVM",
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/15000/restart/60000",
        ],
    );
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let sc_path = format!("{system_root}\\System32\\sc.exe");
    let task_action = format!("\"{sc_path}\" start LAN-KVM");
    let task_ok = run(
        "schtasks.exe",
        &[
            "/Create",
            "/TN",
            "LAN-KVM Boot Start",
            "/SC",
            "ONSTART",
            "/RU",
            "SYSTEM",
            "/RL",
            "HIGHEST",
            "/TR",
            &task_action,
            "/F",
        ],
    );
    setup_log(&format!("boot task -> {task_ok}"));

    // Extra safety: if the ONSTART attempt was blocked (AV/EDR timing), retry
    // every 5 minutes with the same SYSTEM action. sc start on a running
    // service is a harmless no-op error.
    let ensure_ok = run(
        "schtasks.exe",
        &[
            "/Create",
            "/TN",
            "LAN-KVM Ensure Service",
            "/SC",
            "MINUTE",
            "/MO",
            "5",
            "/RU",
            "SYSTEM",
            "/RL",
            "HIGHEST",
            "/TR",
            &task_action,
            "/F",
        ],
    );
    setup_log(&format!("ensure task -> {ensure_ok}"));

    let target = app_target.display().to_string();
    let run_value = format!("\"{target}\" --silent");
    let _ = run(
        "reg.exe",
        &[
            "add",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            "/v",
            "LAN-KVM",
            "/t",
            "REG_SZ",
            "/d",
            &run_value,
            "/f",
        ],
    );

    let shortcut_script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $paths = @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop')); \
         foreach ($p in $paths) {{ \
           $s = $ws.CreateShortcut((Join-Path $p 'LAN-KVM.lnk')); \
           $s.TargetPath = '{target}'; \
           $s.WorkingDirectory = '{dir}'; \
           $s.IconLocation = '{target},0'; \
           $s.Save() \
         }}",
        target = target,
        dir = install_dir.display()
    );
    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &shortcut_script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    // explorer.exe launches the app as the logged-in (non-elevated) user.
    let app_for_explorer = Path::new(&target);
    let _ = Command::new("explorer.exe")
        .arg(app_for_explorer)
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    let firewall_note = if firewall_ok {
        "已自動加入防火牆規則（UDP 45870/45871/45872、TCP 45873/45875）。"
    } else {
        "注意：防火牆規則建立失敗。請用管理員 PowerShell 手動加 UDP 45870-45872 與 TCP 45873/45875，或洽 IT。"
    };
    let service_note = if service_ok {
        "SYSTEM 服務已安裝並啟動（支援登入畫面/UAC）。"
    } else {
        "注意：SYSTEM 服務安裝失敗（可能被防毒隔離）。登入畫面/UAC 控制將無法使用；請看 %TEMP%\\lankvm-setup.log。"
    };

    message(
        &format!(
            "LAN-KVM 安裝完成。\n\n\
             {firewall_note}\n\
             {service_note}\n\
             已設定開機啟動與桌面/開始功能表捷徑。\n\
             瀏覽器會開啟設定頁面；兩台電腦都安裝後，在「掃描到的電腦」選擇對方配對。\n\n\
             安裝記錄：%TEMP%\\lankvm-setup.log\n\
             移除方式：先在 UI 按「結束程式」，再刪除 C:\\Program Files\\LAN-KVM。"
        ),
        false,
    );
}
