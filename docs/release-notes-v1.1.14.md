# LAN-KVM 1.1.14

> Released 2026-11-16 · English | [繁體中文](#繁體中文)

## English

### What's new

Copying a range in Excel and pasting it on the other machine now produces a **table**, the same
way a hardware USB KVM does. Borders, fills, bold text, number formats and merged cells are
preserved; you no longer get a block of tab-separated plain text.

### Why it was flat text

The clipboard crate read `CF_UNICODETEXT` first and stopped there. When Excel copies a range it
publishes several formats at once:

| Format | What it holds |
|---|---|
| `CF_UNICODETEXT` | tab/newline separated text (no formatting) |
| `HTML Format` | an HTML `<table>` with inline styles |
| `Rich Text Format` | the range as an RTF table |
| `XML Spreadsheet` | Excel's own 2003 XML, full cell styling |

Only the first one was transferred, so the peer could only paste flat text.

### What changed

- `lankvm-clipboard` now reads the named formats (`HTML Format`, `Rich Text Format`,
  `XML Spreadsheet`) and their raw bytes, normalising the payload against the document end so
  allocator slack is not sent.
- The clipboard protocol gained a `Rich` frame that carries the plain text **and** the rich
  formats. Pasting into Notepad still uses the text; Excel and Word prefer the rich data.
- The receiving side writes every format in a single clipboard update (`EmptyClipboard` then
  `SetClipboardData`), so one copy satisfies both plain-text and table-aware applications.
- Browser "copy image" also publishes `HTML Format`; when a bitmap is present and there is no
  RTF/Excel XML, the image path is kept so pasting into Paint still works.
- Rich payloads are capped at 48 MB per format / 64 MB total; anything larger falls back to plain
  text. The existing 192 MB frame cap is unchanged.

Wire compatibility: the new frame is appended after the existing variants, so the magic and
protocol version stay the same. An older peer fails to decode that one frame (and logs it) while
continuing to exchange text, image and file frames. **Use 1.1.14 on both machines** to get tables.

### Verified locally

Excel 16 was driven through COM: a 3×3 range with borders, bold header, a fill colour and a
`0.00` number format was copied, run through `read_supported` → `write_supported`, and pasted into
a second worksheet. The paste kept the values, `0.00` format, bold, fill colour, borders and the
3×3 shape.

### Files

```
LAN-KVM-Setup-1.1.14.exe
size   2,905,681 bytes
SHA256 55FBEEDD920AD94DA8539E97CFF73C042D63217B921E29F2DD85DBC2C2FD6E95
```

### How to test

1. Install 1.1.14 on **both** machines (versions must match for rich formats).
2. In Excel on machine A, select a formatted range (borders, a merged cell, a number format,
   a fill colour) and press Ctrl+C.
3. On machine B, press Ctrl+V in Excel. The cells should arrive as a table with the same styling.
4. Paste the same clipboard into Notepad — it should still be plain text.
5. Copy a screenshot or image and paste it into Paint on the peer — the image path must still work.
6. If a paste is wrong, collect a log bundle; the line `clipboard sent: rich (...)` lists which
   formats were transferred.

---

## 繁體中文

### 新功能

在 Excel 框選範圍複製、到另一台貼上，現在會得到**表格**，和硬體 USB KVM 的行為一致。
框線、填色、粗體、數字格式與合併儲存格都會保留，不再變成一堆以 Tab 分隔的純文字。

### 為什麼原本只有純文字

剪貼簿 crate 原本先讀 `CF_UNICODETEXT` 就回傳。Excel 複製範圍時會同時放出多種格式：

| 格式 | 內容 |
|---|---|
| `CF_UNICODETEXT` | 以 Tab／換行分隔的文字（沒有格式） |
| `HTML Format` | 帶 inline style 的 HTML `<table>` |
| `Rich Text Format` | 以 RTF 表示的表格 |
| `XML Spreadsheet` | Excel 自己的 2003 XML，含完整儲存格樣式 |

原本只傳第一種，所以對方只能貼成純文字。

### 修改內容

- `lankvm-clipboard` 改為讀取具名格式（`HTML Format`、`Rich Text Format`、`XML Spreadsheet`）
  的原始 bytes，並依文件結尾修剪，避免把配置器的殘留位元組一起送出。
- 剪貼簿協定新增 `Rich` frame，同時攜帶純文字與富格式。貼到記事本仍使用文字；
  Excel、Word 會優先使用富格式。
- 接收端在一次剪貼簿更新中寫入所有格式（`EmptyClipboard` 後逐一 `SetClipboardData`），
  因此同一次複製可同時滿足純文字與表格應用程式。
- 瀏覽器「複製圖片」也會放 `HTML Format`；若同時有點陣圖、且沒有 RTF／Excel XML，則維持
  圖片路徑，貼到小畫家仍正常。
- 富格式上限為每種 48 MB、合計 64 MB，超過就退回純文字。原有 192 MB frame 上限不變。

連線相容性：新 frame 接在既有 variant 之後，magic 與協定版本不變。舊版對方解不開那一個
frame（會寫入 log），但文字、圖片、檔案仍可正常交換。**兩台都要 1.1.14** 才會有表格。

### 本機驗證

以 COM 驅動 Excel 16：建立 3×3 範圍，含框線、粗體標題、填色與 `0.00` 數字格式，複製後
經過 `read_supported` → `write_supported`，再貼到第二個工作表。貼上後數值、`0.00` 格式、
粗體、填色、框線與 3×3 範圍全部保留。

### 檔案

```
dist-app\LAN-KVM-Setup-1.1.14.exe
size   2,905,681 bytes
SHA256 55FBEEDD920AD94DA8539E97CFF73C042D63217B921E29F2DD85DBC2C2FD6E95
```

### 測試方式

1. 兩台都安裝 1.1.14（要有富格式，版本必須一致）。
2. 在 A 機的 Excel 框選有格式的範圍（框線、合併儲存格、數字格式、填色），按 Ctrl+C。
3. 在 B 機的 Excel 按 Ctrl+V，應貼成同樣格式的表格。
4. 同一份剪貼簿貼到記事本，仍應是純文字。
5. 複製截圖或圖片，貼到對方的小畫家，圖片路徑必須仍正常。
6. 若貼上結果不對，收集 log bundle；`clipboard sent: rich (...)` 會列出實際傳送的格式。
