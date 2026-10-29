use std::ffi::{OsStr, OsString};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use lankvm_net::{Packet, PacketKind};

use anyhow::{Context, Result};
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::launcher::{active_session, enable_privileges, launch_session, Child};
use crate::logger::log;

pub const SERVICE_NAME: &str = "LAN-KVM";
pub const DISPLAY_NAME: &str = "LAN-KVM Service";

define_windows_service!(ffi_service_main, service_main);

pub fn run_dispatcher() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow::anyhow!("service dispatcher failed: {e}"))
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        log("service", format!("fatal: {err:#}"));
    }
}

fn run_service() -> Result<()> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let event_handler = move |event| -> ServiceControlHandlerResult {
        match event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status = service_control_handler::register(SERVICE_NAME, event_handler)
        .context("register service control handler")?;

    status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    log("service", "service started");
    let config = crate::config::load();
    log(
        "service",
        format!(
            "client_port={:?} config={}",
            config.client_port,
            crate::config::config_path().display()
        ),
    );
    let failed = enable_privileges();
    if !failed.is_empty() {
        log(
            "service",
            format!("could not enable privileges: {}", failed.join(", ")),
        );
    }

    // Announce this machine's persistent app id even when no user is logged
    // in. The session agent also announces; duplicates carry the same address
    // and are harmless.
    let identity_stop = Arc::new(AtomicBool::new(false));
    if let Some(port) = config.client_port {
        crate::identity::start(identity_stop.clone(), port);
    }

    let mut child: Option<Child> = None;
    let mut last_attempt = Instant::now() - Duration::from_secs(30);
    let mut last_active_log = u32::MAX;
    let mut last_active_seen = u32::MAX;
    let mut last_health = Instant::now();
    let mut health_failures = 0u32;

    while shutdown_rx.try_recv().is_err() {
        let active = active_session();
        if active != last_active_seen {
            log("service", format!("active session -> {active}"));
            last_active_seen = active;
        }
        if active == u32::MAX {
            if last_active_log != active {
                log("service", "no active console session; waiting");
                last_active_log = active;
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let needs_launch = child
            .as_ref()
            .map(|c| !c.is_running() || c.session_id != active)
            .unwrap_or(true);

        if needs_launch && last_attempt.elapsed() >= Duration::from_secs(5) {
            if let Some(mut old) = child.take() {
                if !old.is_running() {
                    log(
                        "service",
                        format!(
                            "session agent pid={} exited code={}",
                            old.pid,
                            old.exit_code()
                        ),
                    );
                }
                log(
                    "service",
                    format!(
                        "stopping session agent pid={} session={}",
                        old.pid, old.session_id
                    ),
                );
                old.stop();
            }

            match launch_session(active) {
                Ok(new_child) => {
                    log(
                        "service",
                        format!(
                            "started session agent pid={} session={}",
                            new_child.pid, new_child.session_id
                        ),
                    );
                    child = Some(new_child);
                }
                Err(err) => {
                    log(
                        "service",
                        format!("failed to start session agent in session {active}: {err:#}"),
                    );
                }
            }
            last_attempt = Instant::now();
        }

        if let Some(port) = config.client_port {
            if last_health.elapsed() >= Duration::from_secs(5) {
                last_health = Instant::now();
                let running = child.as_ref().map(|c| c.is_running()).unwrap_or(false);
                if running {
                    if probe_client(port) {
                        health_failures = 0;
                    } else {
                        health_failures = health_failures.saturating_add(1);
                        log(
                            "service",
                            format!("client health probe failed ({health_failures}/6)"),
                        );
                        if health_failures >= 6 {
                            log(
                                "service",
                                "restarting session agent: UDP client not responding",
                            );
                            if let Some(mut old) = child.take() {
                                old.stop();
                            }
                            health_failures = 0;
                            last_attempt = Instant::now() - Duration::from_secs(5);
                        }
                    }
                } else {
                    health_failures = 0;
                }
            }
        }

        thread::sleep(Duration::from_secs(1));
    }

    if let Some(mut child) = child.take() {
        log(
            "service",
            format!(
                "stopping session agent pid={} session={}",
                child.pid, child.session_id
            ),
        );
        child.stop();
    }

    identity_stop.store(true, Ordering::SeqCst);

    status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    log("service", "service stopped");
    Ok(())
}

fn probe_client(port: u16) -> bool {
    let Ok(socket) = UdpSocket::bind(("127.0.0.1", 0)) else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
    let mut ping = Packet::new(PacketKind::Ping, 0);
    ping.a = 0;
    if socket.send_to(&ping.encode(), ("127.0.0.1", port)).is_err() {
        return false;
    }
    let mut buffer = [0u8; 64];
    // A busy agent can miss a single probe. Retry once within the same check
    // before counting a failure.
    for _ in 0..2 {
        if let Ok((n, _)) = socket.recv_from(&mut buffer) {
            if Packet::decode(&buffer[..n])
                .map(|packet| packet.kind == PacketKind::Pong && packet.d == 1)
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

pub fn install() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("open service manager (run as Administrator)")?;

    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.clone(),
        launch_arguments: vec![OsString::from("run")],
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };

    if let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
    ) {
        // Reinstall/upgrade: never leave an existing service with an old
        // binary, a demand start type, or a stopped state.
        service
            .change_config(&info)
            .context("update existing service config")?;
        let _ = service.set_description(
            "LAN-KVM privileged supervisor. Starts the session agent that handles input and secure desktop access.",
        );
        let status = service.query_status().context("query existing service")?;
        if status.current_state != ServiceState::Running {
            service
                .start::<&OsStr>(&[])
                .context("start existing service")?;
        }
        println!(
            "Updated {SERVICE_NAME} to AutoStart and ensured it is running (was {:?}).",
            status.current_state
        );
        return Ok(());
    }

    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("create service")?;
    service.set_description(
        "LAN-KVM privileged supervisor. Starts the session agent that handles input and secure desktop access.",
    )?;
    service.start::<&OsStr>(&[]).context("start service")?;

    println!(
        "Installed and started {SERVICE_NAME} from {}",
        exe.display()
    );
    println!("Logs: C:\\ProgramData\\LAN-KVM\\logs");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open service manager (run as Administrator)")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        )
        .context("open service")?;

    let status = service.query_status().context("query service status")?;
    if status.current_state != ServiceState::Stopped {
        let _ = service.stop();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(250));
            if let Ok(status) = service.query_status() {
                if status.current_state == ServiceState::Stopped {
                    break;
                }
            }
        }
    }

    service.delete().context("delete service")?;
    println!("Uninstalled {SERVICE_NAME}");
    Ok(())
}

pub fn start() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open service manager (run as Administrator)")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .context("open service")?;
    service.start::<&OsStr>(&[]).context("start service")?;
    println!("Started {SERVICE_NAME}");
    Ok(())
}

pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open service manager (run as Administrator)")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )
        .context("open service")?;
    service.stop().context("stop service")?;
    println!("Stop requested for {SERVICE_NAME}");
    Ok(())
}
