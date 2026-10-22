//! Clipboard backends and format conversion.
//!
//! The Windows implementation uses `arboard` for text/images and
//! `clipboard-win` for CF_HDROP file lists. The crate is intentionally
//! state-free: callers decide when to read/write and how to sync.

use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        png: Vec<u8>,
    },
    Files(Vec<PathBuf>),
}

pub fn read_text() -> Result<String> {
    let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
    clipboard.get_text().context("read clipboard text")
}

pub fn write_text(value: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
    clipboard
        .set_text(value.to_owned())
        .context("write clipboard text")
}

pub fn read_image_png() -> Result<(u32, u32, Vec<u8>)> {
    let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
    let image = clipboard.get_image().context("read clipboard image")?;
    let png = encode_png(image.width as u32, image.height as u32, &image.bytes)?;
    Ok((image.width as u32, image.height as u32, png))
}

pub fn write_image_png(png: &[u8]) -> Result<()> {
    let (width, height, rgba) = decode_png(png)?;
    let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
    clipboard
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(rgba),
        })
        .context("write clipboard image")
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(anyhow!("image has zero dimension"));
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() < expected {
        return Err(anyhow!(
            "RGBA buffer too short: got {}, expected {}",
            rgba.len(),
            expected
        ));
    }
    let buffer = RgbaImage::from_raw(width, height, rgba[..expected].to_vec())
        .ok_or_else(|| anyhow!("invalid RGBA buffer"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .context("encode PNG")?;
    Ok(png)
}

pub fn decode_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let decoded = image::load_from_memory_with_format(png, ImageFormat::Png)
        .context("decode PNG")?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok((width, height, decoded.into_raw()))
}

#[cfg(windows)]
pub fn read_files() -> Result<Vec<PathBuf>> {
    use clipboard_win::{formats, Clipboard, Getter};
    let _clipboard = Clipboard::new_attempts(10).context("open clipboard")?;
    let mut paths: Vec<PathBuf> = Vec::new();
    formats::FileList
        .read_clipboard(&mut paths)
        .context("read CF_HDROP")?;
    Ok(paths)
}

#[cfg(not(windows))]
pub fn read_files() -> Result<Vec<PathBuf>> {
    Err(anyhow!("file clipboard is Windows-only"))
}

#[cfg(windows)]
pub fn write_files(paths: &[PathBuf]) -> Result<()> {
    use clipboard_win::{formats, Clipboard, Setter};
    let strings: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let _clipboard = Clipboard::new_attempts(10).context("open clipboard")?;
    formats::FileList
        .write_clipboard(&strings)
        .context("write CF_HDROP")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn write_files(_paths: &[PathBuf]) -> Result<()> {
    Err(anyhow!("file clipboard is Windows-only"))
}

#[cfg(windows)]
pub fn sequence() -> u32 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
pub fn sequence() -> u32 {
    0
}

#[derive(Debug, Clone)]
pub struct ClipboardWatcher {
    last_sequence: u32,
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        Self {
            last_sequence: sequence(),
        }
    }

    pub fn acknowledge(&mut self) {
        self.last_sequence = sequence();
    }

    pub fn poll(&mut self) -> Option<ClipboardContent> {
        let current = sequence();
        if current == self.last_sequence {
            return None;
        }
        self.last_sequence = current;
        read_supported().ok()
    }
}

impl Default for ClipboardWatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn encode(content: &ClipboardContent) -> Result<Vec<u8>> {
    postcard::to_allocvec(content).context("encode clipboard content")
}

pub fn decode(bytes: &[u8]) -> Result<ClipboardContent> {
    postcard::from_bytes(bytes).context("decode clipboard content")
}

pub fn content_summary(content: &ClipboardContent) -> String {
    match content {
        ClipboardContent::Text(text) => format!("text ({} chars)", text.chars().count()),
        ClipboardContent::Image { width, height, png } => {
            format!("image {width}x{height} ({} bytes PNG)", png.len())
        }
        ClipboardContent::Files(files) => format!("files ({})", files.len()),
    }
}

pub fn read_supported() -> Result<ClipboardContent> {
    if let Ok(text) = read_text() {
        if !text.is_empty() {
            return Ok(ClipboardContent::Text(text));
        }
    }
    if let Ok(files) = read_files() {
        if !files.is_empty() {
            return Ok(ClipboardContent::Files(files));
        }
    }
    if let Ok((width, height, png)) = read_image_png() {
        return Ok(ClipboardContent::Image { width, height, png });
    }
    Err(anyhow!("no supported clipboard content"))
}

pub fn write_supported(content: &ClipboardContent) -> Result<()> {
    match content {
        ClipboardContent::Text(text) => write_text(text),
        ClipboardContent::Image { png, .. } => write_image_png(png),
        ClipboardContent::Files(paths) => write_files(paths),
    }
}

pub fn files_exist(paths: &[PathBuf]) -> bool {
    paths.iter().all(|p| Path::new(p).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let width = 4;
        let height = 3;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for i in 0..(width * height) {
            rgba.extend_from_slice(&[(i % 256) as u8, 10, 20, 255]);
        }
        let png = encode_png(width, height, &rgba).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let (w, h, decoded) = decode_png(&png).unwrap();
        assert_eq!((w, h), (width, height));
        assert_eq!(decoded, rgba);
    }

    #[test]
    fn rejects_bad_buffers() {
        assert!(encode_png(0, 1, &[]).is_err());
        assert!(encode_png(2, 2, &[0, 0, 0]).is_err());
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn supported_variants_round_trip_shape() {
        let content = ClipboardContent::Text("hello".into());
        match content {
            ClipboardContent::Text(text) => assert_eq!(text, "hello"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn wire_round_trip() {
        let items = [
            ClipboardContent::Text("hello 中文".into()),
            ClipboardContent::Image {
                width: 2,
                height: 2,
                png: vec![1, 2, 3, 4],
            },
            ClipboardContent::Files(vec![PathBuf::from(r"C:\tmp\a.txt")]),
        ];
        for item in items {
            let bytes = encode(&item).unwrap();
            assert_eq!(decode(&bytes).unwrap(), item);
        }
    }
}
