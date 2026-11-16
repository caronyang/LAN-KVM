//! Clipboard backends and format conversion.
//!
//! The Windows implementation uses `arboard` for text/images,
//! `clipboard-win` for CF_HDROP file lists, and direct Win32 calls for
//! rich formats (HTML / RTF / XML Spreadsheet) so tables copied from Excel
//! keep their layout when pasted on the peer. The crate is intentionally
//! state-free: callers decide when to read/write and how to sync.

use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};

/// One extra clipboard format carried alongside the plain text.
///
/// `name` is a Windows registered format name (`HTML Format`,
/// `Rich Text Format`, `XML Spreadsheet`). `data` is the raw payload exactly
/// as the source application provided it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedFormat {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        png: Vec<u8>,
    },
    Files(Vec<PathBuf>),
    /// Plain text plus rich formats, so pasting into Excel/Word keeps table
    /// structure and styling. `text` is the fallback for plain-text targets.
    Rich {
        text: String,
        formats: Vec<NamedFormat>,
    },
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
    use clipboard_win::{empty, formats, Clipboard, Setter};
    let strings: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let _clipboard = Clipboard::new_attempts(10).context("open clipboard")?;
    // `formats::FileList` writes with `NoClear`: it only replaces CF_HDROP and
    // leaves every other format in place. Explorer's copy operation also puts
    // "Shell IDList Array" (CFSTR_SHELLIDLIST) on the clipboard, and Paste
    // prefers it over CF_HDROP — so without emptying first, pasting a file we
    // just received yields the *previous* file. Clear everything, then set.
    empty().context("empty clipboard")?;
    formats::FileList
        .write_clipboard(&strings)
        .context("write CF_HDROP")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn write_files(_paths: &[PathBuf]) -> Result<()> {
    Err(anyhow!("file clipboard is Windows-only"))
}

/// Registered clipboard formats that carry table/format fidelity. Excel
/// publishes all three when a cell range is copied; browsers and Word publish
/// at least one of HTML / RTF.
pub const RICH_FORMATS: [&str; 3] = ["HTML Format", "Rich Text Format", "XML Spreadsheet"];

pub fn is_rich_format(name: &str) -> bool {
    RICH_FORMATS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// True when the clipboard also carries a bitmap. Browser "copy image" puts
/// both HTML and the image on the clipboard; preferring the HTML would break
/// pasting into Paint, so those keep the image path.
#[cfg(windows)]
fn has_bitmap() -> bool {
    // CF_BITMAP, CF_DIB, CF_DIBV5.
    [2u32, 8, 17]
        .iter()
        .any(|format| unsafe { windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable(*format) } != 0)
}

#[cfg(not(windows))]
fn has_bitmap() -> bool {
    false
}

/// Reads every supported rich format plus the plain-text fallback in one
/// clipboard open. `None` when the clipboard has no rich format.
#[cfg(windows)]
pub fn read_rich() -> Option<(String, Vec<NamedFormat>)> {
    win_clipboard::read_rich()
}

#[cfg(not(windows))]
pub fn read_rich() -> Option<(String, Vec<NamedFormat>)> {
    None
}

/// Replaces the clipboard with the plain text plus the given rich formats, so
/// one paste can satisfy both plain-text and table-aware applications.
#[cfg(windows)]
pub fn write_rich(text: &str, formats: &[NamedFormat]) -> Result<()> {
    win_clipboard::write_rich(text, formats)
}

#[cfg(not(windows))]
pub fn write_rich(text: &str, _formats: &[NamedFormat]) -> Result<()> {
    write_text(text)
}

#[cfg(windows)]
mod win_clipboard {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;
    use std::thread;
    use std::time::Duration;

    use anyhow::{anyhow, bail, Result};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };

    use super::NamedFormat;

    // windows-sys 0.61 does not export GlobalFree, but a failed
    // SetClipboardData must release the block itself.
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalFree(handle: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    const CF_UNICODETEXT: u32 = 13;

    /// Clipboard owner for the current thread; closes it on drop so error paths
    /// cannot leave the clipboard locked.
    struct ClipboardGuard;

    impl ClipboardGuard {
        fn acquire() -> Result<Self> {
            for attempt in 0..10 {
                if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                    return Ok(Self);
                }
                if attempt < 9 {
                    thread::sleep(Duration::from_millis(25));
                }
            }
            Err(anyhow!("open clipboard: still held by another process"))
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }

    fn register(name: &str) -> u32 {
        let wide: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    }

    unsafe fn read_handle(format: u32) -> Option<Vec<u8>> {
        if IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        if size == 0 {
            return None;
        }
        let data = GlobalLock(handle) as *const u8;
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data, size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }

    fn decode_utf16le(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    fn encode_utf16le(text: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
        for unit in text.encode_utf16().chain(Some(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    /// GlobalSize includes allocator slack, so clip the payload at the point
    /// the owning document says it ends. Without this a table paste can carry
    /// trailing heap garbage.
    fn normalize(name: &str, mut data: Vec<u8>) -> Option<Vec<u8>> {
        while data.last() == Some(&0) {
            data.pop();
        }
        if data.is_empty() {
            return None;
        }
        match name {
            "HTML Format" => {
                if let Some(end) = cf_html_end(&data) {
                    data.truncate(end);
                }
            }
            "Rich Text Format" => truncate_after_last(&mut data, b'}'),
            "XML Spreadsheet" => truncate_after_last(&mut data, b'>'),
            _ => {}
        }
        if data.is_empty() {
            None
        } else {
            Some(data)
        }
    }

    fn truncate_after_last(data: &mut Vec<u8>, needle: u8) {
        if let Some(position) = data.iter().rposition(|byte| *byte == needle) {
            data.truncate(position + 1);
        }
    }

    /// `EndHTML` in the CF_HTML header is a byte offset to the end of the
    /// document, so it is authoritative even if the allocation has slack.
    fn cf_html_end(data: &[u8]) -> Option<usize> {
        // The header is ASCII and ends at the first '<' (the fragment comment).
        // Decoding the whole allocation would fail when the slack is not UTF-8.
        let limit = data.len().min(4096);
        let head_end = data[..limit]
            .iter()
            .position(|byte| *byte == b'<')
            .unwrap_or(limit);
        let head = std::str::from_utf8(&data[..head_end]).ok()?;
        let mut end = None;
        for line in head.lines() {
            if let Some(value) = line.strip_prefix("EndHTML:") {
                end = value.trim().parse::<usize>().ok();
            }
        }
        end.filter(|value| *value > 0 && *value <= data.len())
    }

    pub fn read_rich() -> Option<(String, Vec<NamedFormat>)> {
        let _guard = ClipboardGuard::acquire().ok()?;
        let mut formats = Vec::new();
        for name in super::RICH_FORMATS {
            let id = register(name);
            if id == 0 {
                continue;
            }
            let Some(data) = (unsafe { read_handle(id) }) else {
                continue;
            };
            let Some(data) = normalize(name, data) else {
                continue;
            };
            formats.push(NamedFormat {
                name: name.to_string(),
                data,
            });
        }
        if formats.is_empty() {
            return None;
        }
        let text = unsafe { read_handle(CF_UNICODETEXT) }
            .map(|bytes| decode_utf16le(&bytes))
            .unwrap_or_default();
        Some((text, formats))
    }

    unsafe fn set_bytes(format: u32, bytes: &[u8]) -> Result<()> {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if handle.is_null() {
            bail!("GlobalAlloc failed for clipboard format {format}");
        }
        let target = GlobalLock(handle) as *mut u8;
        if target.is_null() {
            GlobalFree(handle);
            bail!("GlobalLock failed for clipboard format {format}");
        }
        if bytes.is_empty() {
            *target = 0;
        } else {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        }
        GlobalUnlock(handle);
        // On success the system owns the handle and frees it with the clipboard.
        if SetClipboardData(format, handle).is_null() {
            GlobalFree(handle);
            bail!("SetClipboardData failed for clipboard format {format}");
        }
        Ok(())
    }

    pub fn write_rich(text: &str, formats: &[NamedFormat]) -> Result<()> {
        let _guard = ClipboardGuard::acquire()?;
        unsafe { EmptyClipboard() };
        unsafe { set_bytes(CF_UNICODETEXT, &encode_utf16le(text))? };

        let mut written = 0usize;
        let mut first_error = None;
        for format in formats {
            if !super::is_rich_format(&format.name) || format.data.is_empty() {
                continue;
            }
            let id = register(&format.name);
            if id == 0 {
                continue;
            }
            // Text formats are NUL terminated; the document parser uses the
            // bytes before it.
            let mut data = format.data.clone();
            data.push(0);
            match unsafe { set_bytes(id, &data) } {
                Ok(()) => written += 1,
                Err(err) => {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
            }
        }
        // The plain-text fallback is already in place, so a missing rich
        // format is not a hard failure; report only when nothing rich landed.
        if written == 0 {
            if let Some(err) = first_error {
                return Err(err);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn html_is_clipped_to_the_header_end() {
            let body = "<html><body>a\tb</body></html>";
            let header_len = "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000000\r\n".len();
            let header = format!(
                "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:{:010}\r\n",
                header_len + body.len()
            );
            assert_eq!(header.len(), header_len);
            let mut data = header.into_bytes();
            data.extend_from_slice(body.as_bytes());
            data.extend_from_slice(&[0, 0, 0xAB, 0xCD]); // allocator slack
            let out = normalize("HTML Format", data).unwrap();
            assert_eq!(out.len(), header_len + body.len());
            assert!(out.ends_with(body.as_bytes()));
        }

        #[test]
        fn rtf_and_xml_stop_at_the_last_delimiter() {
            let rtf = normalize("Rich Text Format", b"{\\rtf1 x}\0junk".to_vec()).unwrap();
            assert_eq!(rtf, b"{\\rtf1 x}");
            let xml = normalize("XML Spreadsheet", b"<Workbook/>\0junk".to_vec()).unwrap();
            assert_eq!(xml, b"<Workbook/>");
        }

        #[test]
        fn empty_payload_is_skipped() {
            assert!(normalize("HTML Format", vec![0, 0, 0]).is_none());
        }

        #[test]
        fn utf16_text_round_trip() {
            let text = "中文\tA1";
            assert_eq!(decode_utf16le(&encode_utf16le(text)), text);
        }
    }
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
        ClipboardContent::Rich { text, formats } => {
            let names: Vec<&str> = formats.iter().map(|f| f.name.as_str()).collect();
            format!(
                "rich ({} chars) [{}]",
                text.chars().count(),
                names.join(", ")
            )
        }
    }
}

pub fn read_supported() -> Result<ClipboardContent> {
    // Rich formats first: Excel/Word put plain text on the clipboard too, and
    // reading only the text is what flattens a copied table. Excel always
    // publishes RTF or XML Spreadsheet, so an HTML-only clipboard that also
    // has a bitmap (browser "copy image") keeps the image path.
    if let Some((text, formats)) = read_rich() {
        let structured = formats.iter().any(|format| format.name != "HTML Format");
        if structured || !has_bitmap() {
            return Ok(ClipboardContent::Rich { text, formats });
        }
    }
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
        ClipboardContent::Rich { text, formats } => write_rich(text, formats),
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
            ClipboardContent::Rich {
                text: "a\tb\r\nc\td".into(),
                formats: vec![
                    NamedFormat {
                        name: "HTML Format".into(),
                        data: b"Version:0.9\r\n<html><table><tr><td>a</td></tr></table></html>"
                            .to_vec(),
                    },
                    NamedFormat {
                        name: "Rich Text Format".into(),
                        data: b"{\\rtf1\\ansi a\\tab b}".to_vec(),
                    },
                ],
            },
        ];
        for item in items {
            let bytes = encode(&item).unwrap();
            assert_eq!(decode(&bytes).unwrap(), item);
        }
    }
}
