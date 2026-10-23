#[cfg(not(windows))]
fn main() {
    eprintln!("LAN-KVM service is Windows-only");
    std::process::exit(1);
}

#[cfg(windows)]
mod config;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod net;
#[cfg(windows)]
mod remote_inject;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod winjector;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::env;
    use std::process::ExitCode;

    fn run() -> anyhow::Result<()> {
        let mut args = env::args().skip(1);
        match args.next().as_deref() {
            Some("run") => service::run_dispatcher(),
            Some("session") => session::run(args),
            Some("winjector") => {
                let mut port = remote_inject::PORT;
                let mut parent_pid = 0u32;
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--port" => {
                            if let Some(value) = args.next() {
                                port = value.parse().unwrap_or(port);
                            }
                        }
                        "--parent-pid" => {
                            if let Some(value) = args.next() {
                                parent_pid = value.parse().unwrap_or(0);
                            }
                        }
                        _ => {}
                    }
                }
                winjector::run(port, parent_pid)
            }
            Some("install") => service::install(),
            Some("uninstall") => service::uninstall(),
            Some("start") => service::start(),
            Some("stop") => service::stop(),
            Some("probe") => {
                let report = lankvm_platform::windows::probe()?;
                print_report(&report);
                Ok(())
            }
            Some("help") | Some("-h") | Some("--help") | None => {
                usage();
                Ok(())
            }
            Some(other) => Err(anyhow::anyhow!("unknown command: {other}")),
        }
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn usage() {
    println!(
        r#"LAN-KVM service (P0)

Usage (Administrator required for install/uninstall/start/stop):
  lankvm-service install      Install as LocalSystem and start
  lankvm-service uninstall    Stop and remove the service
  lankvm-service start        Start the installed service
  lankvm-service stop         Stop the service
  lankvm-service run          Service entry point (used by SCM)
  lankvm-service session      Run the session agent in the current session
  lankvm-service probe        Print session/desktop/DPI/monitor info

Logs:
  C:\ProgramData\LAN-KVM\logs\service.log
  C:\ProgramData\LAN-KVM\logs\session.log

The service launches a SYSTEM session agent in the active console session.
It logs Default/Winlogon desktop transitions. Input hook/injection is the next step."#
    );
}

#[cfg(windows)]
fn print_report(report: &lankvm_platform::ProbeReport) {
    println!("LAN-KVM service probe");
    println!("  pid: {}", report.pid);
    println!(
        "  session: {} (active console session: {})",
        report.session_id,
        if report.active_console_session == u32::MAX {
            "none".to_string()
        } else {
            report.active_console_session.to_string()
        }
    );
    println!(
        "  elevation: {} / integrity: {}",
        report.elevated, report.integrity
    );
    match &report.desktop_error {
        None => println!("  input desktop: {}", report.desktop),
        Some(err) => println!("  input desktop: {} ({})", report.desktop, err),
    }
    println!(
        "  virtual bounds: {}x{}+{},{}",
        report.virtual_bounds.width(),
        report.virtual_bounds.height(),
        report.virtual_bounds.left,
        report.virtual_bounds.top
    );
    println!("  cursor: ({}, {})", report.cursor.x, report.cursor.y);
    println!("  monitors:");
    for monitor in &report.monitors {
        println!(
            "    #{} {} {}x{}+{},{} dpi={}{}",
            monitor.id,
            if monitor.name.is_empty() {
                "<unnamed>"
            } else {
                &monitor.name
            },
            monitor.rect.width(),
            monitor.rect.height(),
            monitor.rect.left,
            monitor.rect.top,
            monitor.dpi,
            if monitor.primary { " primary" } else { "" }
        );
    }
}
