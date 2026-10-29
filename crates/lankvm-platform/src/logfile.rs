//! Size-based rotation and tail reads for plain-text append-only logs.
//!
//! LAN-KVM logs are written from several processes at once (service, session
//! agent, winjector helper, tray app), so rotation cannot rely on in-memory
//! bookkeeping that another process cannot see. Instead every writer tracks
//! only how many bytes it has appended and probes the real file size once per
//! [`PROBE_EVERY`] bytes. When a file is over its limit it is rolled to
//! `name.1`, older generations shift down, and the writer keeps appending to a
//! fresh file.
//!
//! Failures are deliberately ignored: if another process holds the file open
//! without sharing delete access, the rename fails and rotation is retried at
//! the next probe. Logging must never break the caller.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// How many bytes a writer may append before the next size probe.
const PROBE_EVERY: u64 = 1024 * 1024;

static PENDING: AtomicU64 = AtomicU64::new(0);

/// Rotation policy for one log file.
#[derive(Clone, Copy, Debug)]
pub struct Rotation {
    /// Rotate once the file grows past this many bytes.
    pub max_bytes: u64,
    /// How many rotated generations to keep (`name.1` is the newest).
    pub keep: u32,
}

impl Rotation {
    pub const fn new(max_bytes: u64, keep: u32) -> Self {
        Self { max_bytes, keep }
    }
}

/// `name.log` + index 2 -> `name.log.2`.
pub fn rotated_path(path: &Path, index: u32) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{index}"));
    path.with_file_name(name)
}

/// Cheap rotation check: call after appending `written` bytes.
///
/// The file size is only read once per MiB written, so the hot logging path
/// stays a single `write_all` in the common case.
pub fn maybe_rotate(path: &Path, rotation: Rotation, written: u64) {
    if written == 0 {
        return;
    }
    let pending = PENDING.fetch_add(written, Ordering::Relaxed) + written;
    if pending < PROBE_EVERY {
        return;
    }
    PENDING.store(0, Ordering::Relaxed);

    let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size > rotation.max_bytes {
        let _ = rotate(path, rotation.keep);
    }
}

/// Roll `path` to `path.1`, shifting existing generations down one slot and
/// dropping the oldest. `keep = 0` deletes the file instead.
pub fn rotate(path: &Path, keep: u32) -> io::Result<()> {
    if keep == 0 {
        return fs::remove_file(path);
    }

    let _ = fs::remove_file(rotated_path(path, keep));
    for index in (1..keep).rev() {
        let from = rotated_path(path, index);
        if from.exists() {
            let _ = fs::rename(&from, rotated_path(path, index + 1));
        }
    }
    fs::rename(path, rotated_path(path, 1))
}

/// Read at most the last `max_bytes` of a text file, dropping the partial
/// first line so callers never display half a record.
pub fn tail_text(path: &Path, max_bytes: u64) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut buffer = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer).into_owned();
    if start == 0 {
        return Ok(text);
    }
    match text.find('\n') {
        Some(position) => Ok(text[position + 1..].to_string()),
        None => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("lankvm-log-{tag}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rotate_pushes_generations_down() {
        let dir = temp_dir("rotate");
        let path = dir.join("session.log");
        fs::write(&path, b"newest").unwrap();
        fs::write(rotated_path(&path, 1), b"older").unwrap();
        fs::write(rotated_path(&path, 2), b"oldest").unwrap();

        rotate(&path, 3).unwrap();

        assert!(!path.exists());
        assert_eq!(fs::read(rotated_path(&path, 1)).unwrap(), b"newest");
        assert_eq!(fs::read(rotated_path(&path, 2)).unwrap(), b"older");
        assert_eq!(fs::read(rotated_path(&path, 3)).unwrap(), b"oldest");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotate_drops_the_oldest_generation() {
        let dir = temp_dir("drop");
        let path = dir.join("app.log");
        fs::write(&path, b"newest").unwrap();
        fs::write(rotated_path(&path, 1), b"older").unwrap();
        fs::write(rotated_path(&path, 2), b"oldest").unwrap();

        rotate(&path, 2).unwrap();

        assert_eq!(fs::read(rotated_path(&path, 1)).unwrap(), b"newest");
        assert_eq!(fs::read(rotated_path(&path, 2)).unwrap(), b"older");
        assert!(!rotated_path(&path, 3).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn maybe_rotate_rolls_files_over_the_limit() {
        let dir = temp_dir("maybe");
        let path = dir.join("app.log");
        fs::write(&path, vec![b'x'; 32]).unwrap();

        // A write smaller than the probe interval must not even stat the file.
        maybe_rotate(&path, Rotation::new(16, 2), 1);
        assert!(path.exists());

        // Crossing the probe interval checks the real size and rotates.
        maybe_rotate(&path, Rotation::new(16, 2), PROBE_EVERY);
        assert!(!path.exists());
        assert_eq!(fs::read(rotated_path(&path, 1)).unwrap().len(), 32);

        // A fresh, small file survives the next probe.
        fs::write(&path, b"tiny").unwrap();
        maybe_rotate(&path, Rotation::new(16, 2), PROBE_EVERY);
        assert!(path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tail_text_keeps_whole_last_lines() {
        let dir = temp_dir("tail");
        let path = dir.join("engine.log");
        fs::write(&path, b"first line is long\nsecond\nthird\n").unwrap();

        let tail = tail_text(&path, 14).unwrap();
        assert_eq!(tail, "second\nthird\n");

        let whole = tail_text(&path, 4096).unwrap();
        assert_eq!(whole, "first line is long\nsecond\nthird\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
