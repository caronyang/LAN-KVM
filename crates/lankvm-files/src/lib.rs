//! Chunked, hashed local file transfer engine.
//!
//! This is the local half of the future network file transfer: it streams
//! chunks, hashes the bytes, writes a `.part` file, and atomically renames it.
//! The transport layer will call the same engine with a network-backed
//! `Read`/`Write` implementation.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CopyOptions {
    pub chunk_size: usize,
    pub overwrite: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            overwrite: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, Default)]
pub struct CopyReport {
    pub files: Vec<FileReport>,
    pub total_bytes: u64,
}

pub fn copy_files(
    sources: &[PathBuf],
    destination_dir: &Path,
    options: &CopyOptions,
) -> Result<CopyReport> {
    if sources.is_empty() {
        return Err(anyhow!("no source files"));
    }
    if options.chunk_size == 0 {
        return Err(anyhow!("chunk size must be greater than zero"));
    }
    fs::create_dir_all(destination_dir).context("create destination directory")?;

    let mut report = CopyReport::default();
    for source in sources {
        let metadata =
            fs::metadata(source).with_context(|| format!("stat source {}", source.display()))?;
        if !metadata.is_file() {
            return Err(anyhow!("not a regular file: {}", source.display()));
        }

        let name = safe_file_name(source)?;
        let destination = unique_destination(destination_dir, &name, options.overwrite)?;
        let part = destination.with_extension(format!(
            "{}.part",
            destination
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("tmp")
        ));

        let mut reader = BufReader::with_capacity(
            options.chunk_size.max(4096),
            File::open(source).with_context(|| format!("open {}", source.display()))?,
        );
        let mut writer = BufWriter::with_capacity(
            options.chunk_size.max(4096),
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&part)
                .with_context(|| format!("create {}", part.display()))?,
        );

        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; options.chunk_size.max(4096)];
        let mut total = 0u64;
        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("read {}", source.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .with_context(|| format!("write {}", part.display()))?;
            total += read as u64;
        }

        writer.flush().context("flush destination")?;
        writer.get_ref().sync_all().context("sync destination")?;
        drop(writer);

        if fs::metadata(&part).map(|m| m.len()).unwrap_or(0) != total {
            let _ = fs::remove_file(&part);
            return Err(anyhow!("short write for {}", source.display()));
        }

        if destination.exists() {
            if options.overwrite {
                fs::remove_file(&destination)
                    .with_context(|| format!("replace {}", destination.display()))?;
            } else {
                let _ = fs::remove_file(&part);
                return Err(anyhow!(
                    "destination already exists: {}",
                    destination.display()
                ));
            }
        }

        fs::rename(&part, &destination)
            .with_context(|| format!("rename {} to {}", part.display(), destination.display()))?;

        report.total_bytes += total;
        report.files.push(FileReport {
            source: source.clone(),
            destination,
            bytes: total,
            blake3: hasher.finalize().to_hex().to_string(),
        });
    }

    Ok(report)
}

pub fn safe_file_name(source: &Path) -> Result<String> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("invalid file name: {}", source.display()))?;
    if name.is_empty() || name == "." || name == ".." {
        return Err(anyhow!("invalid file name: {}", source.display()));
    }
    if name.contains('/') || name.contains('\\') || name.contains(':') || name.contains("..") {
        return Err(anyhow!("unsafe file name: {name}"));
    }
    Ok(name.to_string())
}

pub fn unique_destination(dir: &Path, name: &str, overwrite: bool) -> Result<PathBuf> {
    let candidate = dir.join(name);
    if overwrite || !candidate.exists() {
        return Ok(candidate);
    }

    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let extension = path.extension().and_then(|e| e.to_str());
    for index in 1..10_000 {
        let file_name = match extension {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = dir.join(file_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!("could not find an available destination name"))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; DEFAULT_CHUNK_SIZE];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lankvm-files-{name}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copies_text_and_hashes() {
        let dir = temp_dir("text");
        let src = dir.join("hello.txt");
        fs::write(&src, b"hello LAN-KVM").unwrap();
        let dest = dir.join("out");
        let report = copy_files(&[src.clone()], &dest, &CopyOptions::default()).unwrap();
        assert_eq!(report.files.len(), 1);
        assert_eq!(fs::read(dest.join("hello.txt")).unwrap(), b"hello LAN-KVM");
        assert_eq!(report.files[0].bytes, 13);
        assert_eq!(report.files[0].blake3, hash_file(&src).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copies_large_binary_in_chunks() {
        let dir = temp_dir("binary");
        let src = dir.join("big.bin");
        let data: Vec<u8> = (0..(3 * 1024 * 1024 + 17))
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        fs::write(&src, &data).unwrap();
        let dest = dir.join("out");
        let options = CopyOptions {
            chunk_size: 64 * 1024,
            overwrite: false,
        };
        let report = copy_files(&[src], &dest, &options).unwrap();
        assert_eq!(fs::read(dest.join("big.bin")).unwrap(), data);
        assert_eq!(report.total_bytes, data.len() as u64);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn avoids_name_collisions() {
        let dir = temp_dir("collision");
        let src = dir.join("a.txt");
        fs::write(&src, b"one").unwrap();
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"existing").unwrap();
        let report = copy_files(&[src], &dest, &CopyOptions::default()).unwrap();
        assert!(report.files[0].destination.ends_with("a (1).txt"));
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"existing");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_unsafe_names() {
        assert!(safe_file_name(Path::new("..")).is_err());
        assert!(safe_file_name(Path::new("ok.txt")).is_ok());
    }
}
