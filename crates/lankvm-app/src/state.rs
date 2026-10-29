use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn default_true() -> bool {
    true
}

fn default_edge() -> String {
    "right".to_string()
}

fn default_trigger_px() -> f32 {
    20.0
}

fn default_trigger_ms() -> u64 {
    300
}

fn default_release_px() -> i32 {
    10
}

fn default_lang() -> String {
    "en".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedPeer {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub control_port: u16,
    pub data_port: u16,
    /// "master" on the machine that owns the input, "client" on the other.
    pub role: String,
    #[serde(default = "default_true")]
    pub auto_connect: bool,
    #[serde(default = "default_edge")]
    pub edge: String,
    #[serde(default = "default_trigger_px")]
    pub trigger_px: f32,
    #[serde(default = "default_trigger_ms")]
    pub trigger_ms: u64,
    #[serde(default = "default_release_px")]
    pub release_px: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub id: String,
    pub name: String,
    #[serde(default = "default_edge")]
    pub edge: String,
    #[serde(default = "default_trigger_px")]
    pub trigger_px: f32,
    #[serde(default = "default_trigger_ms")]
    pub trigger_ms: u64,
    #[serde(default = "default_release_px")]
    pub release_px: i32,
    #[serde(default)]
    pub paused: bool,
    /// UI language: "en" (default) or "zh". Shared by the web UI and the tray.
    #[serde(default = "default_lang")]
    pub lang: String,
    #[serde(default)]
    pub paired: Vec<PairedPeer>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            id: new_id(),
            name: hostname(),
            edge: default_edge(),
            trigger_px: default_trigger_px(),
            trigger_ms: default_trigger_ms(),
            release_px: default_release_px(),
            paused: false,
            lang: default_lang(),
            paired: Vec::new(),
        }
    }

    pub fn peer(&self, id: &str) -> Option<&PairedPeer> {
        self.paired.iter().find(|p| p.id == id)
    }

    pub fn peer_mut(&mut self, id: &str) -> Option<&mut PairedPeer> {
        self.paired.iter_mut().find(|p| p.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub control_port: u16,
    pub data_port: u16,
    pub last_seen: Instant,
}

#[derive(Clone, Debug)]
pub struct Pairing {
    pub peer_id: String,
    pub peer_name: String,
    pub peer_ip: String,
    pub peer_control_port: u16,
    pub peer_data_port: u16,
    /// "host" = this machine shows the code, "guest" = this machine enters it.
    pub role: String,
    pub code: Option<String>,
    pub awaiting_code: bool,
    pub error: Option<String>,
    pub created: Instant,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub peer_id: String,
    pub role: String,
    pub peer_ip: String,
    pub data_port: u16,
    pub pid: u32,
    pub stop_file: PathBuf,
    pub started: Instant,
    pub last_seen: Instant,
    /// Client sessions are served by the SYSTEM service, not a child engine.
    pub uses_service: bool,
}

pub struct App {
    pub config: Mutex<Config>,
    pub peers: Mutex<HashMap<String, PeerInfo>>,
    pub pairing: Mutex<Option<Pairing>>,
    pub session: Mutex<Option<Session>>,
    pub logs: Mutex<Vec<String>>,
    pub suppress_until: Mutex<Option<Instant>>,
    pub engine: Mutex<Option<std::process::Child>>,
    pub stop_lock: Mutex<()>,
    pub service_ok: AtomicBool,
    pub http_port: u16,
    pub control_port: u16,
    pub data_port: u16,
    pub discovery_port: u16,
    pub profile: Option<String>,
}

impl App {
    pub fn new(
        config: Config,
        http_port: u16,
        control_port: u16,
        data_port: u16,
        discovery_port: u16,
        profile: Option<String>,
    ) -> Self {
        Self {
            config: Mutex::new(config),
            peers: Mutex::new(HashMap::new()),
            pairing: Mutex::new(None),
            session: Mutex::new(None),
            logs: Mutex::new(Vec::new()),
            suppress_until: Mutex::new(None),
            engine: Mutex::new(None),
            stop_lock: Mutex::new(()),
            service_ok: AtomicBool::new(false),
            http_port,
            control_port,
            data_port,
            discovery_port,
            profile,
        }
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = format!("[{}] {}", now_hms(), line.into());
        eprintln!("{line}");
        let _ = fs::create_dir_all(config_dir());
        {
            static WRITE_LOCK: Mutex<()> = Mutex::new(());
            let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(mut file) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(config_dir().join("app.log"))
            {
                let _ = file.write_all(format!("{line}\n").as_bytes());
            }
        }
        let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        logs.push(line);
        if logs.len() > 300 {
            let drain = logs.len() - 300;
            logs.drain(0..drain);
        }
    }

    pub fn peer_upsert(&self, info: PeerInfo) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        peers.insert(info.id.clone(), info);
    }

    pub fn peer_by_id(&self, id: &str) -> Option<PeerInfo> {
        self.peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn snapshot(&self) -> Value {
        let config = self
            .config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let peers = self.peers.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let pairing = self
            .pairing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let session = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let logs = self.logs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let suppressed = self
            .suppress_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|t| t > Instant::now())
            .unwrap_or(false);

        let mut peer_list: Vec<Value> = peers
            .values()
            .map(|p| {
                let paired = config.peer(&p.id).cloned();
                json!({
                    "id": p.id,
                    "name": p.name,
                    "ip": p.ip,
                    "control_port": p.control_port,
                    "data_port": p.data_port,
                    "online": true,
                    "age_secs": p.last_seen.elapsed().as_secs(),
                    "paired": paired.is_some(),
                    "paired_role": paired.as_ref().map(|x| x.role.clone()),
                    "auto_connect": paired.as_ref().map(|x| x.auto_connect).unwrap_or(false),
                })
            })
            .collect();
        peer_list.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });

        json!({
            "id": config.id,
            "name": config.name,
            "version": env!("CARGO_PKG_VERSION"),
            "paused": config.paused,
            "lang": config.lang,
            "suppressed": suppressed,
            "service_running": self.service_ok.load(Ordering::Relaxed),
            "settings": {
                "edge": config.edge,
                "trigger_px": config.trigger_px,
                "trigger_ms": config.trigger_ms,
                "release_px": config.release_px,
            },
            "ports": {
                "http": self.http_port,
                "control": self.control_port,
                "data": self.data_port,
                "discovery": self.discovery_port,
            },
            "peers": peer_list,
            "paired": config.paired.iter().map(|p| json!({
                "id": p.id,
                "name": p.name,
                "ip": p.ip,
                "role": p.role,
                "auto_connect": p.auto_connect,
                "edge": p.edge,
            })).collect::<Vec<_>>(),
            "pairing": pairing.map(|p| json!({
                "peer_id": p.peer_id,
                "peer_name": p.peer_name,
                "role": p.role,
                "code": p.code,
                "awaiting_code": p.awaiting_code,
                "error": p.error,
            })),
            "session": session.map(|s| json!({
                "peer_id": s.peer_id,
                "role": s.role,
                "peer_ip": s.peer_ip,
                "data_port": s.data_port,
                "pid": s.pid,
                "uses_service": s.uses_service,
                "uptime_secs": s.started.elapsed().as_secs(),
            })),
            "logs": logs.iter().rev().take(30).cloned().collect::<Vec<_>>(),
        })
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("LAN-KVM")
}

pub fn config_path(profile: Option<&str>) -> PathBuf {
    let file = match profile {
        Some(profile) if !profile.is_empty() => format!("app-{profile}.json"),
        _ => "app.json".to_string(),
    };
    config_dir().join(file)
}

pub fn load_or_create(profile: Option<&str>) -> (Config, bool) {
    if let Ok(text) = fs::read_to_string(config_path(profile)) {
        if let Ok(mut config) = serde_json::from_str::<Config>(&text) {
            // 2 px was the old default and released control on the slightest
            // edge contact. Migrate it once to the new default.
            if config.release_px <= 2 {
                config.release_px = default_release_px();
            }
            for peer in &mut config.paired {
                if peer.release_px <= 2 {
                    peer.release_px = default_release_px();
                }
            }
            return (config, true);
        }
    }
    (Config::new(), false)
}

pub fn save(profile: Option<&str>, config: &Config) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(config) {
        let _ = fs::write(config_path(profile), text);
    }
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "LAN-KVM".to_string())
}

pub fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:04x}", nanos, std::process::id() & 0xffff)
}

pub fn now_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
