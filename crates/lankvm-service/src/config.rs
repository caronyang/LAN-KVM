use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// If set, the session agent listens for the P0 UDP client protocol.
    pub client_port: Option<u16>,
    /// How far past the entry edge the cursor must be pushed before the
    /// client returns control to the master (pixels). Default 2.
    pub release_px: Option<i32>,
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("LAN-KVM").join("config.txt")
}

pub fn load() -> Config {
    let Ok(text) = fs::read_to_string(config_path()) else {
        return Config::default();
    };
    let mut config = Config::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("client_port") {
            config.client_port = value.trim().parse().ok();
        }
        if key.trim().eq_ignore_ascii_case("release_px") {
            config.release_px = value.trim().parse().ok();
        }
    }
    config
}
