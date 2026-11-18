# LAN-KVM 1.1.15

> Released 2026-11-18 · English | [繁體中文](#繁體中文)

## English

### What's new

Copying a **whole folder** in Explorer and pasting it on the other machine now recreates the
folder tree: subfolders, empty folders and relative paths are preserved, and file contents arrive
intact. Previously a folder copy did nothing at all.

### Why a folder did nothing

Explorer puts the selected items on the clipboard as `CF_HDROP`. When the selection is a
directory, `CF_HDROP` contains a directory path. The frame builder only accepted `is_file()`
entries and logged:

```
clipboard files: skipping non-file D:\work\photos
```

so a folder-only selection ended as `no readable files` and nothing was sent.

### What changed

- New `Frame::Tree(Vec<WireEntry>)`; each entry is `{ path, data }`, where `path` is relative to
  the selected root (`/` separated) and `data = None` marks a directory. The frame is appended
  after the existing variants (tag 5), so old peers are unaffected apart from rejecting this one
  frame.
- The scan preserves subfolders, empty directories and relative layout. Two selected roots with
  the same name are disambiguated as `name` and `name (2)`.
- Directory symlinks / junctions (directory reparse points) are skipped so a link cannot loop the
  scan; cloud placeholder files (also reparse points) are still read. Limits: depth 32,
  5,000 entries, 128 MB; when a limit is hit the rest is skipped and logged.
- The receiver sanitises every path component, rejects `..`, drive prefixes and absolute roots,
  writes the bundle under `%LOCALAPPDATA%\LAN-KVM\clipboard\<time>\`, and puts the bundle roots on
  the clipboard as `CF_HDROP`. Explorer then creates the same tree on paste, including empty
  folders.
- File attributes, ACLs and timestamps are not preserved.

### Verified locally

Five new unit tests cover: nested folders + empty directory scanning, duplicate root names,
receiving a tree while rejecting `../evil.txt` and `C:/absolute.txt`, the `safe_relative` rules,
and the wire tag / fingerprint round-trip of the new frame.

### Files

```
LAN-KVM-Setup-1.1.15.exe
size   2,917,267 bytes
SHA256 527B08B0FBEEB623DB479CAD8583163A98D4A1525D9F376948050F4C68BCF89E
```

### How to test

1. Install 1.1.15 on **both** machines (folder copy needs the new frame on both ends).
2. On machine A, copy a folder that contains a subfolder, an empty folder and a few files.
3. Paste on machine B into Explorer (Desktop or a folder). The same tree should appear, with the
   empty folder and identical file contents.
4. Copy a mixed selection (a folder plus a loose file) and paste on B — both should arrive.
5. Copy several loose files (no folder) — the old flat file path must still work.
6. Repeat B → A. If nothing arrives, the log shows the skip reason; make sure both sides are
   1.1.15 or newer.

---

## 繁體中文

### 新功能

在 Explorer 複製**整個資料夾**，到另一台貼上會重建目錄樹：子資料夾、空目錄與相對路徑
都會保留，檔案內容也會完整送達。之前複製資料夾完全沒有反應。

### 為什麼複製資料夾沒反應

Explorer 會把選取的項目以 `CF_HDROP` 放進剪貼簿。若選取的是目錄，`CF_HDROP` 裡就是目錄
路徑。原本的 frame 建置只接受 `is_file()` 的項目，並寫下：

```
clipboard files: skipping non-file D:\work\photos
```

所以只選資料夾時最後以 `no readable files` 結束，什麼都沒送出。

### 修改內容

- 新增 `Frame::Tree(Vec<WireEntry>)`，每個項目是 `{ path, data }`；`path` 相對於選取根
  （以 `/` 分隔），`data = None` 代表目錄。新 frame 接在既有 variant 之後（tag 5），
  舊版對方只會無法套用這一個 frame，其他功能不受影響。
- 掃描保留子資料夾、空目錄與相對結構；兩個同名根目錄會變成 `name` 與 `name (2)`。
- 目錄 symlink／junction（目錄 reparse point）會跳過，避免連結造成無限遞迴；雲端檔案
  （同樣是 reparse point）仍會正常讀取。上限：深度 32、5,000 個項目、128 MB；
  超過上限會跳過其餘內容並寫 log。
- 接收端逐段 sanitize，拒絕 `..`、磁碟機前綴與絕對路徑，寫入
  `%LOCALAPPDATA%\LAN-KVM\clipboard\<時間>\`，再以 `CF_HDROP` 把 bundle 根項目放進剪貼簿。
  Explorer 貼上時就會建立同樣的目錄樹，包含空目錄。
- 不保留檔案屬性、ACL 與時間戳記。

### 本機驗證

新增 5 個單元測試涵蓋：巢狀資料夾＋空目錄掃描、同名根目錄、接收時拒絕
`../evil.txt` 與 `C:/absolute.txt`、`safe_relative` 規則，以及新 frame 的 wire tag 與
fingerprint round-trip。

### 檔案

```
dist-app\LAN-KVM-Setup-1.1.15.exe
size   2,917,267 bytes
SHA256 527B08B0FBEEB623DB479CAD8583163A98D4A1525D9F376948050F4C68BCF89E
```

### 測試方式

1. 兩台都安裝 1.1.15（目錄複製需要兩端都有新 frame）。
2. 在 A 機複製一個含子資料夾、空目錄與數個檔案的資料夾。
3. 到 B 機在 Explorer（桌面或資料夾）貼上，應出現同樣的目錄樹、空目錄與相同內容。
4. 複製「資料夾＋單一檔案」的混合選取，貼到 B，兩者都應出現。
5. 複製多個單一檔案（不含資料夾），原本的平面檔案路徑必須仍正常。
6. 反向 B → A 再測一次。若沒反應，log 會顯示跳過原因；並確認兩端都是 1.1.15 以上。
