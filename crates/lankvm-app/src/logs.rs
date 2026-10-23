use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::state::{config_dir, hostname, App};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn collect(app: &Arc<App>) -> Result<PathBuf> {
    let stamp = {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{secs}")
    };

    let profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let desktop = profile.join("Desktop");
    let onedrive_desktop = profile.join("OneDrive").join("Desktop");
    let base = if desktop.exists() {
        desktop
    } else if onedrive_desktop.exists() {
        onedrive_desktop
    } else {
        profile
    };
    let dir = base.join(format!("LAN-KVM-logs-{}-{}", hostname(), stamp));
    fs::create_dir_all(&dir).context("create log directory")?;

    let mut notes = Vec::new();

    let mut copy = |from: PathBuf, name: &str| {
        if from.exists() {
            match fs::copy(&from, dir.join(name)) {
                Ok(_) => notes.push(format!("copied {name}")),
                Err(err) => notes.push(format!("FAILED {name}: {err}")),
            }
        } else {
            notes.push(format!("missing {name} ({})", from.display()));
        }
    };

    copy(config_dir().join("app.log"), "app.log");
    copy(config_dir().join("engine.log"), "engine.log");
    copy(config_dir().join("app.json"), "app.json");
    copy(
        PathBuf::from("C:\\ProgramData\\LAN-KVM\\logs\\service.log"),
        "service.log",
    );
    copy(
        PathBuf::from("C:\\ProgramData\\LAN-KVM\\logs\\session.log"),
        "session.log",
    );
    copy(
        std::env::temp_dir().join("lankvm-setup.log"),
        "setup.log",
    );

    let state = app.snapshot();
    let _ = fs::write(
        dir.join("state.json"),
        serde_json::to_string_pretty(&state).unwrap_or_default(),
    );

    let mut system = String::new();
    system.push_str(&format!(
        "host={}\ncollected_at={}\nports=http:{},control:{},data:{},discovery:{}\n",
        hostname(),
        stamp,
        app.http_port,
        app.control_port,
        app.data_port,
        app.discovery_port
    ));
    for (label, program, args) in [
        ("ipconfig", "ipconfig.exe", vec!["/all"]),
        ("service", "sc.exe", vec!["query", "LAN-KVM"]),
        ("udp", "netstat.exe", vec!["-ano", "-p", "UDP"]),
        (
            "firewall_discovery",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Discovery",
            ],
        ),
        (
            "firewall_control",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Control",
            ],
        ),
        (
            "firewall_data",
            "netsh.exe",
            vec![
                "advfirewall",
                "firewall",
                "show",
                "rule",
                "name=LAN-KVM Data",
            ],
        ),
    ] {
        system.push_str(&format!("\n===== {label} =====\n"));
        match run(program, &args) {
            Ok(text) => system.push_str(&text),
            Err(err) => system.push_str(&format!("error: {err}\n")),
        }
    }
    let _ = fs::write(dir.join("system.txt"), system);

    notes.push(format!("state.json / system.txt written"));
    let mut readme = String::from(
        "LAN-KVM log bundle\n\nSend the whole folder to the developer.\n\nFiles:\n",
    );
    for note in &notes {
        readme.push_str(&format!("- {note}\n"));
    }
    let _ = fs::write(dir.join("README.txt"), readme);

    // Show the folder so the user can grab it immediately.
    let _ = Command::new("explorer.exe")
        .arg(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();

    app.log(format!("logs collected to {}", dir.display()));
    Ok(dir)
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().context("run command")?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}
