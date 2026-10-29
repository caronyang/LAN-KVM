use std::fs;
use std::path::PathBuf;

/// The app's input/data port. The SYSTEM service is the client engine, so it
/// must listen here even when `config.txt` is missing or was written by an
/// older version. 45871 is the app's *control* port and must never be used.
pub const DEFAULT_DATA_PORT: u16 = 45_872;

#[derive(Debug, Clone)]
pub struct Config {
    /// The session agent listens for the P0 UDP client protocol on this port.
    pub client_port: Option<u16>,
    /// How far past the entry edge the cursor must be pushed before the
    /// client returns control to the master (pixels).
    pub release_px: Option<i32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_port: Some(DEFAULT_DATA_PORT),
            release_px: Some(10),
        }
    }
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
            // 45871 is the app's control port. Legacy configs used it here by
            // mistake, which made the service steal the app's port. Migrate
            // them to the data port instead of failing to start the engine.
            config.client_port = match value.trim().parse::<u16>() {
                Ok(45_871) => Some(DEFAULT_DATA_PORT),
                Ok(0) => Some(DEFAULT_DATA_PORT),
                Ok(port) => Some(port),
                Err(_) => Some(DEFAULT_DATA_PORT),
            };
        }
        if key.trim().eq_ignore_ascii_case("release_px") {
            config.release_px = value.trim().parse().ok();
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Config {
        let mut config = Config::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("client_port") {
                config.client_port = match value.trim().parse::<u16>() {
                    Ok(45_871) => Some(DEFAULT_DATA_PORT),
                    Ok(0) => Some(DEFAULT_DATA_PORT),
                    Ok(port) => Some(port),
                    Err(_) => Some(DEFAULT_DATA_PORT),
                };
            }
            if key.trim().eq_ignore_ascii_case("release_px") {
                config.release_px = value.trim().parse().ok();
            }
        }
        config
    }

    #[test]
    fn defaults_to_data_port() {
        assert_eq!(Config::default().client_port, Some(DEFAULT_DATA_PORT));
    }

    #[test]
    fn migrates_legacy_control_port() {
        assert_eq!(
            parse("client_port=45871").client_port,
            Some(DEFAULT_DATA_PORT)
        );
        assert_eq!(parse("client_port=0").client_port, Some(DEFAULT_DATA_PORT));
    }

    #[test]
    fn accepts_explicit_data_port() {
        assert_eq!(parse("client_port=45872").client_port, Some(45_872));
        assert_eq!(parse("release_px=12").release_px, Some(12));
    }
}
