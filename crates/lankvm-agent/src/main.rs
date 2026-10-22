//! Product-side entry point (skeleton).
//!
//! The shipped agent must not contain development-only key logging, hook
//! testing, or unauthenticated network PoC code. Those live in
//! `lankvm-devtools`, which is not distributed.

use std::env;
use std::process::ExitCode;

use anyhow::{anyhow, Result};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("probe") => {
            let report = lankvm_platform::windows::probe()?;
            print_report(&report);
            Ok(())
        }
        Some("version") | Some("-V") | Some("--version") => {
            println!("LAN-KVM agent {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help") | Some("-h") | Some("--help") | None => {
            usage();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown command: {other}")),
    }
}

fn usage() {
    println!(
        r#"LAN-KVM agent {}

Usage:
  lankvm-agent probe      Print session, desktop, DPI, monitors, cursor
  lankvm-agent version    Print version

Development tools (hooks/injection/PoC) are in lankvm-devtools and are not part
of the shipped product."#,
        env!("CARGO_PKG_VERSION")
    );
}

fn print_report(report: &lankvm_platform::ProbeReport) {
    println!("LAN-KVM probe");
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
