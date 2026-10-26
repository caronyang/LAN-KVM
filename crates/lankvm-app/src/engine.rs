use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::state::{config_dir, App, Session};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn engine_log_path() -> PathBuf {
    config_dir().join("engine.log")
}

fn stop_file_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "lankvm-stop-{}-{}.flag",
        std::process::id(),
        nanos
    ))
}

fn spawn(app: &Arc<App>, mut args: Vec<String>, role: &str, peer_id: &str, peer_ip: &str, data_port: u16) -> Result<()> {
    let stop_file = stop_file_path();
    let _ = fs::remove_file(&stop_file);
    args.push("--stop-file".to_string());
    args.push(stop_file.display().to_string());
    args.push("--parent-pid".to_string());
    args.push(std::process::id().to_string());

    let log_path = engine_log_path();
    // Keep older engine runs: overwriting lost the log of the run that hung.
    if fs::metadata(&log_path).map(|m| m.len() > 8 * 1024 * 1024).unwrap_or(false) {
        let _ = fs::remove_file(&log_path);
    }
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("open engine.log")?;
    let _ = writeln!(
        log,
        "===== engine {role} start {} peer={peer_ip}:{data_port} =====",
        crate::state::now_hms()
    );
    let log2 = log.try_clone().context("clone engine.log")?;


    let exe = std::env::current_exe().context("current_exe")?;
    let mut command = Command::new(exe);
    command
        .args(["--engine"])
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let child = command.spawn().context("spawn engine")?;
    let pid = child.id();
    app.log(format!(
        "engine {role} started pid={pid} peer={peer_ip}:{data_port}"
    ));

    *app.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    *app.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session {
        peer_id: peer_id.to_string(),
        role: role.to_string(),
        peer_ip: peer_ip.to_string(),
        data_port,
        pid,
        stop_file,
        started: Instant::now(),
        last_seen: Instant::now(),
        uses_service: false,
    });
    Ok(())
}

pub fn start_master(
    app: &Arc<App>,
    peer_id: &str,
    peer_ip: &str,
    data_port: u16,
    edge: &str,
    trigger_px: f32,
    trigger_ms: u64,
) -> Result<()> {
    let args = vec![
        "master".to_string(),
        "--peer".to_string(),
        peer_ip.to_string(),
        "--port".to_string(),
        data_port.to_string(),
        "--edge".to_string(),
        edge.to_string(),
        "--trigger-px".to_string(),
        trigger_px.to_string(),
        "--trigger-ms".to_string(),
        trigger_ms.to_string(),
    ];
    spawn(app, args, "master", peer_id, peer_ip, data_port)
}

#[allow(dead_code)]
pub fn start_client(
    app: &Arc<App>,
    peer_id: &str,
    peer_ip: &str,
    local_data_port: u16,
    release_px: i32,
) -> Result<()> {
    let args = vec![
        "client".to_string(),
        "--port".to_string(),
        local_data_port.to_string(),
        "--release-px".to_string(),
        release_px.to_string(),
    ];
    spawn(app, args, "client", peer_id, peer_ip, local_data_port)
}

pub fn stop_session(app: &Arc<App>, send_peer_stop: bool) {
    let _guard = app.stop_lock.lock().unwrap_or_else(|e| e.into_inner());
    let session = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(session) = session else {
        return;
    };

    if session.uses_service {
        *app.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
        app.log(format!(
            "service client session cleared (peer={})",
            session.peer_id
        ));
        if send_peer_stop {
            crate::net::send_session_stop(app, &session.peer_ip, session.peer_id.as_str());
        }
        return;
    }

    let _ = fs::write(&session.stop_file, b"stop");

    let mut child = app.engine.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(child) = child.as_mut() {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }

    let _ = fs::remove_file(&session.stop_file);
    *app.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
    app.log(format!(
        "engine {} stopped (peer={})",
        session.role, session.peer_id
    ));

    if send_peer_stop {
        crate::net::send_session_stop(app, &session.peer_ip, session.peer_id.as_str());
    }
}

pub fn emergency_release(app: &Arc<App>, reason: &str) {
    let had_session = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    app.log(format!(
        "EMERGENCY release ({reason}){}",
        if had_session { "" } else { " - no active session" }
    ));
    // stop_session writes the stop file, waits up to 3 s, then kills the
    // engine. Killing it is what guarantees the cursor comes back even if the
    // engine's message loop is wedged.
    stop_session(app, true);
    *app.suppress_until.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Instant::now() + Duration::from_secs(60));
}

pub fn engine_alive(app: &Arc<App>) -> bool {
    let uses_service = app
        .session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.uses_service)
        .unwrap_or(false);
    if uses_service {
        return app.service_ok.load(Ordering::Relaxed);
    }

    let mut guard = app.engine.lock().unwrap_or_else(|e| e.into_inner());
    let Some(child) = guard.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            let code = status.code().unwrap_or(-1);
            *guard = None;
            *app.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
            app.log(format!("engine exited (code={code})"));
            false
        }
        Ok(None) => true,
        Err(_) => false,
    }
}

/// Called in the hidden engine child process.
pub fn run_engine(args: impl Iterator<Item = String>) -> ! {
    let mut role = String::new();
    let mut peer = String::new();
    let mut port = crate::DEFAULT_DATA_PORT;
    let mut edge = lankvm_core::Edge::Right;
    let mut trigger_px = 20.0f32;
    let mut trigger_ms = 300u64;
    let mut release_px = 2i32;
    let mut stop_file: Option<PathBuf> = None;
    let mut parent_pid = 0u32;

    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "master" | "client" => role = arg,
            "--peer" => peer = args.next().unwrap_or_default(),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--edge" => {
                edge = match args.next().as_deref() {
                    Some("left") => lankvm_core::Edge::Left,
                    _ => lankvm_core::Edge::Right,
                }
            }
            "--trigger-px" => trigger_px = args.next().and_then(|v| v.parse().ok()).unwrap_or(trigger_px),
            "--trigger-ms" => trigger_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(trigger_ms),
            "--release-px" => release_px = args.next().and_then(|v| v.parse().ok()).unwrap_or(release_px),
            "--stop-file" => stop_file = args.next().map(PathBuf::from),
            "--parent-pid" => parent_pid = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            _ => {}
        }
    }

    if let Some(path) = stop_file {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(100));
            if path.exists() {
                let _ = fs::remove_file(&path);
                lankvm_platform::poc::request_stop();
                break;
            }
        });
    }

    if parent_pid != 0 {
        thread::spawn(move || watch_parent(parent_pid));
    }

    let result = match role.as_str() {
        "master" => {
            if peer.is_empty() {
                eprintln!("engine master needs --peer");
                std::process::exit(2);
            }
            lankvm_platform::poc::run_master(&peer, port, edge, 0, trigger_px, trigger_ms)
        }
        "client" => lankvm_platform::poc::run_client(port, 0, release_px),
        _ => {
            eprintln!("engine needs master|client");
            std::process::exit(2);
        }
    };

    if let Err(err) = result {
        eprintln!("engine error: {err:#}");
        std::process::exit(1);
    }
    std::process::exit(0);
}

#[cfg(windows)]
fn watch_parent(parent_pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE};

    const SYNCHRONIZE: u32 = 0x0010_0000;

    unsafe {
        let handle = OpenProcess(SYNCHRONIZE, 0, parent_pid);
        if handle.is_null() {
            lankvm_platform::poc::request_stop();
            return;
        }
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
        // The app exited without stopping us (crash or forced close). Stop
        // gracefully so the cursor is unclipped and keys are released.
        lankvm_platform::poc::request_stop();
    }
}
