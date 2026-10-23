use std::ffi::OsStr;
use std::io;
use std::mem::size_of;
use std::net::TcpStream;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, TokenPrimary, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows_sys::Win32::System::SystemServices::MAXIMUM_ALLOWED;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, TerminateProcess,
    WaitForSingleObject, PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::desktop;
use crate::logger::{log, log_path};
use crate::net;
use crate::remote_inject;use crate::worker::{take_counts, DesktopWorker};

pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let mut seconds = 0u64;
    let mut client = false;
    let mut port = 45_871u16;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow!("--seconds needs a value"))?
                    .parse()?;
            }
            "--session" => {
                let _ = args.next();
            }
            "--client" => client = true,
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| anyhow!("--port needs a value"))?
                    .parse()?;
            }
            other => return Err(anyhow!("unknown session argument: {other}")),
        }
    }
    let stop = Arc::new(AtomicBool::new(false));

    let started = Instant::now();
    log(
        "session",
        format!(
            "session agent started pid={} log={}",
            std::process::id(),
            log_path("session").display()
        ),
    );

    match lankvm_platform::windows::probe() {
        Ok(report) => {
            log(
                "session",
                format!(
                    "pid={} session={} console_session={} elevated={} integrity={} desktop={} bounds={}",
                    report.pid,
                    report.session_id,
                    report.active_console_session,
                    report.elevated,
                    report.integrity,
                    report.desktop,
                    report.virtual_bounds
                ),
            );
            if let Some(err) = report.desktop_error {
                log("session", format!("desktop error: {err}"));
            }
        }
        Err(err) => log("session", format!("probe failed: {err:#}")),
    }

    if client {
        let release_px = crate::config::load().release_px.unwrap_or(2);
        if let Err(err) = net::start_client(port, stop.clone(), release_px) {
            log("session", format!("UDP client failed: {err:#}"));
        }
    }

    let mut last_desktop = String::new();
    let mut last_error = String::new();
    let mut worker: Option<DesktopWorker> = None;
    let mut worker_desktop = String::new();
    let mut next_worker_attempt = Instant::now();
    let mut last_count_log = Instant::now();
    let mut winjector: Option<Winjector> = None;
    let mut winjector_retry = Instant::now();
    let mut winjector_connect_retry = Instant::now();

    if client {
        // Start the Winlogon helper early so locking the desktop does not
        // spend 1-2 seconds creating a process. It stays idle on Winlogon
        // until input is routed there.
        match start_winjector() {
            Ok(started) => {
                log(
                    "session",
                    format!("winjector prestarted pid={}", started.pid),
                );
                winjector = Some(started);
                match connect_winjector() {
                    Ok(stream) => {
                        remote_inject::attach(stream);
                        log("session", "winjector attached (idle until Winlogon)");
                    }
                    Err(err) => {
                        log("session", format!("winjector prestart connect failed: {err:#}"));
                    }
                }
            }
            Err(err) => {
                log(
                    "session",
                    format!("winjector prestart failed (retry on Winlogon): {err:#}"),
                );
            }
        }
    }

    loop {
        if seconds > 0 && started.elapsed() >= Duration::from_secs(seconds) {
            break;
        }

        match lankvm_platform::windows::active_desktop_name() {
            Ok(name) => {
                if name != last_desktop {
                    log("session", format!("input desktop -> {name}"));
                    desktop::bump();
                    last_desktop = name.clone();
                    last_error.clear();
                }

                if !worker_desktop.is_empty() && worker_desktop != name {
                    if let Some(old) = worker.take() {
                        old.stop();
                    }
                    worker_desktop.clear();
                    next_worker_attempt = Instant::now();
                }

                if worker.is_none() && Instant::now() >= next_worker_attempt {
                    match DesktopWorker::start(&name, !client) {
                        Ok(new_worker) => {
                            let _ = take_counts();
                            worker = Some(new_worker);
                            worker_desktop = name.clone();
                            log(
                                "session",
                                format!(
                                    "{name}: {}",
                                    if client {
                                        "injector active"
                                    } else {
                                        "hooks active"
                                    }
                                ),
                            );
                        }
                        Err(err) => {
                            log("session", format!("{name}: cannot install hooks: {err:#}"));
                            next_worker_attempt = Instant::now() + Duration::from_secs(5);
                        }
                    }
                }

                if client {
                    if name.eq_ignore_ascii_case("Winlogon") {
                        let alive = winjector.as_ref().map(|w| w.is_alive()).unwrap_or(false);
                        if !alive && Instant::now() >= winjector_retry {
                            if let Some(mut old) = winjector.take() {
                                old.stop();
                            }
                            match start_winjector() {
                                Ok(started) => {
                                    log(
                                        "session",
                                        format!("winjector started pid={}", started.pid),
                                    );
                                    winjector = Some(started);
                                    winjector_connect_retry = Instant::now();
                                }
                                Err(err) => {
                                    log("session", format!("winjector start failed: {err:#}"));
                                    winjector_retry = Instant::now() + Duration::from_secs(2);
                                }
                            }
                        }
                        if !remote_inject::connected()
                            && Instant::now() >= winjector_connect_retry
                        {
                            match connect_winjector() {
                                Ok(stream) => {
                                    remote_inject::attach(stream);
                                    log("session", "winjector connected");
                                }
                                Err(err) => {
                                    log("session", format!("winjector connect failed: {err:#}"));
                                    winjector_connect_retry =
                                        Instant::now() + Duration::from_secs(2);
                                }
                            }
                        }
                        if remote_inject::connected() && !remote_inject::is_active() {
                            remote_inject::enable();
                            log("session", "winjector enabled (Winlogon)");
                        }
                    } else {
                        if remote_inject::is_active() {
                            remote_inject::disable();
                            log("session", "winjector disabled (left Winlogon)");
                        }
                        winjector_retry = Instant::now();
                        winjector_connect_retry = Instant::now();
                    }
                }
            }
            Err(err) => {
                let message = err.to_string();
                if message != last_error {
                    log("session", format!("input desktop unavailable: {message}"));
                    last_error = message;
                }
                if let Some(old) = worker.take() {
                    old.stop();
                }
                worker_desktop.clear();
            }
        }

        if worker.is_some() && last_count_log.elapsed() >= Duration::from_secs(5) {
            let (keys, mouse, injected, failed) = take_counts();
            log(
                "session",
                format!(
                    "{worker_desktop}: input counts keys={keys} mouse_moves={mouse} injected={injected} inject_fail={failed}"
                ),
            );
            last_count_log = Instant::now();
        }

        thread::sleep(Duration::from_millis(250));
    }

    stop.store(true, Ordering::SeqCst);
    remote_inject::clear();
    if let Some(mut old) = winjector.take() {
        old.stop();
    }
    if let Some(old) = worker.take() {
        old.stop();
    }
    log("session", "session agent stopped");
    Ok(())
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Winjector {
    process: HANDLE,
    pid: u32,
}

impl Winjector {
    fn is_alive(&self) -> bool {
        if self.process.is_null() {
            return false;
        }
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
    }

    fn stop(&mut self) {
        if self.process.is_null() {
            return;
        }
        unsafe {
            TerminateProcess(self.process, 1);
            WaitForSingleObject(self.process, 2_000);
            CloseHandle(self.process);
        }
        self.process = null_mut();
    }
}

impl Drop for Winjector {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_winjector() -> Result<Winjector> {
    let exe = std::env::current_exe()?;
    let exe_wide = to_wide(exe.as_os_str());
    let command = format!(
        "\"{}\" winjector --port {} --parent-pid {}",
        exe.display(),
        remote_inject::PORT,
        std::process::id()
    );
    let mut command_wide = to_wide(&command);
    let mut desktop_wide = to_wide("winsta0\\Winlogon");

    let mut token: HANDLE = null_mut();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
            &mut token,
        )
    } == 0
    {
        bail!(
            "OpenProcessToken failed: {}",
            io::Error::last_os_error()
        );
    }

    let mut primary: HANDLE = null_mut();
    let duplicated = unsafe {
        DuplicateTokenEx(
            token,
            MAXIMUM_ALLOWED,
            null(),
            2, // SecurityImpersonation
            TokenPrimary,
            &mut primary,
        )
    };
    unsafe { CloseHandle(token) };
    if duplicated == 0 {
        bail!("DuplicateTokenEx failed: {}", io::Error::last_os_error());
    }

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop_wide.as_mut_ptr();

    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessAsUserW(
            primary,
            exe_wide.as_ptr(),
            command_wide.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NO_WINDOW,
            null(),
            null(),
            &startup,
            &mut process,
        )
    };
    unsafe { CloseHandle(primary) };

    if created == 0 {
        bail!(
            "CreateProcessAsUserW(winjector, winsta0\\Winlogon) failed: {}",
            io::Error::last_os_error()
        );
    }
    unsafe { CloseHandle(process.hThread) };

    Ok(Winjector {
        process: process.hProcess,
        pid: process.dwProcessId,
    })
}

fn connect_winjector() -> Result<TcpStream> {
    let mut last = String::new();
    for _ in 0..20 {
        match TcpStream::connect(("127.0.0.1", remote_inject::PORT)) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(err) => {
                last = err.to_string();
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    bail!("connect 127.0.0.1:{} failed: {last}", remote_inject::PORT)
}

fn to_wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    let mut wide: Vec<u16> = value.as_ref().encode_wide().collect();
    wide.push(0);
    wide
}
