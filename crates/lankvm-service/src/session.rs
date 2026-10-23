use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::logger::{log, log_path};
use crate::net;
use crate::worker::{take_counts, DesktopWorker};

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

    loop {
        if seconds > 0 && started.elapsed() >= Duration::from_secs(seconds) {
            break;
        }

        match lankvm_platform::windows::active_desktop_name() {
            Ok(name) => {
                if name != last_desktop {
                    log("session", format!("input desktop -> {name}"));
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
    if let Some(old) = worker.take() {
        old.stop();
    }
    log("session", "session agent stopped");
    Ok(())
}
