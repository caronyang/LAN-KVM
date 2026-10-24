# 跨機剪貼簿與控制端游標隱藏（v0.3-clip）

> 日期：2026-10-24
> 產物：`dist-app/LAN-KVM-Setup-0.3-clip2.exe`
> SHA-256：`7FA43F902902DECA489AE2711D722692CAF1DB6DDC3130BE0072B7DF1C24503B`

這一階段處理兩件事：

1. **任一台複製（文字／圖片／檔案），另一台能貼上**。
2. **滑鼠切到另一台後，原電腦（master）的游標要消失**，而不是沿螢幕邊緣上下移動。

---

## 1. 跨機剪貼簿

### 1.1 為什麼是 TCP 而不是 P0 UDP

P0 的 UDP 封包是無連線、未加密、且沒有重傳；剪貼簿圖片與檔案動輒數 MB，
用 UDP 分片會讓錯誤處理變得複雜。這一版用一條 TCP 連線，等 QUIC/TLS 進場再替換。

| 項目 | 值 |
|------|----|
| 埠 | TCP **45875** |
| 誰聽 | 配對角色為 `client` 的那台（B） |
| 誰連 | 配對角色為 `master` 的那台（A），每 2 秒重試 |
| 方向 | 雙向（同一條連線） |
| 編碼 | 4-byte 長度前綴 + postcard frame |

### 1.2 連線與握手

1. 雙方各送 `Hello { magic, version, id, name }`。
2. 檢查 magic／version，並確認對方 `id` 在 `config.paired` 裡；未配對直接斷線。
3. 通過後 master 先送一次目前剪貼簿（讓「先複製、後連線」也能用）。
4. 之後任一邊剪貼簿序號變更就送一個 frame。

重複連線用一個 generation 計數器處理：新連線通過握手後，舊 session 會在 120 ms 內
自行結束並 shutdown socket，不會出現兩個 writer 同時送。

### 1.3 Frame 型別

```rust
enum Frame {
    Ping,
    Text(String),
    Image { width: u32, height: u32, png: Vec<u8> },
    Files(Vec<WireFile>),   // WireFile { name, data }
}
```

- 單一 frame 上限 **192 MB**。
- 檔案：單次最多 **200 個**、總量 **128 MB**；超過的檔案會跳過並寫 log。
- 檔案會在對方寫到：
  `%LOCALAPPDATA%\LAN-KVM\clipboard\<unix-ms>\<原檔名>`
  然後以 CF_HDROP（檔案清單）放進剪貼簿，貼上時就是一般檔案。
- 檔名會過濾 `<>:"/\|?*` 與控制字元，避免路徑穿越。
- 2 天前的暫存資料夾會自動刪除。
- log 會直接寫出檔名與落地資料夾，方便判斷「貼上的到底是哪一個檔案」：

```
clipboard sent: files (1) [TEST-file-3410.txt]
clipboard received: files (1) [TEST-file-3410.txt] -> C:\Users\...\LAN-KVM\clipboard\1792805341024
```

### 1.4 回音抑制（避免兩台互丟）

寫入本機剪貼簿會讓剪貼簿序號改變，若不處理就會形成「A 收到 → 寫入 → A 又送出」的迴圈。
這一版用三層，而且**不會吞掉使用者的複製動作**：

1. **精確序號抑制**：套用對方內容時記下「自己這次寫入」的剪貼簿序號
   （`SUPPRESSED_SEQ`），writer 只忽略這一個序號。使用者在貼上後立刻複製新內容，
   序號不同，照樣送出。（舊版用 1.5 秒時間窗，會把這段時間內的複製丟掉。）
2. **套用中旗標**：寫入期間（`APPLYING`）writer 不讀取序號，避免競態回送。
3. **短窗內容比對**：同一份內容（含檔案位元組）在 3 秒內再次出現，視為
   Windows／剪貼簿檢視器／對端的回音，不再送出；超過 3 秒後重新複製同一份內容仍會同步。

實測（同機兩個 profile 互連）文字、圖片、檔案都只會各送一次、各收一次，
且「複製檔案後立刻複製文字」也能正常送出。

### 1.5 已知的行為（不是 bug）

- 檔名相同、大小相同、內容相同的檔案，在另一端貼上後看起來會**一模一樣**。
  例如兩台都有 `LAN-KVM-Setup-0.2.exe`，從 B 複製它到 A，A 貼上的當然還是同一個安裝檔。
  請用**名稱不同**的檔案驗證（例如在 B 建 `B-測試-123.txt`）。
- 兩台的系統時鐘如果差很多（例如差 12 分鐘），log 時間會對不起來，但功能不受影響。

### 1.6 限制

- 剪貼簿是**使用者工作階段**資源：兩台都必須登入 Windows。
  B 停在登入頁時，鍵鼠仍可用，但剪貼簿同步要等 B 登入後才會建立。
- 只有「第一個」配對角色會被使用（單一 peer）；多 peer 之後再做。
- 未加密：只在信任網段使用。

---

## 2. 控制端游標隱藏

### 2.1 問題

舊版用 `ShowCursor(false)` 加上把游標 `ClipCursor` 在整個螢幕邊緣的長條。
`ShowCursor` 只影響呼叫執行緒的顯示計數，游標移到別的視窗時會被該視窗的
`WM_SETCURSOR` 設回來；而全高的 clip 讓真實游標仍沿邊緣上下滑動。

### 2.2 現在的做法

1. **全螢幕透明 overlay**（`windows.rs::create_cursor_hider`）：
   - `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`、`WS_POPUP`
   - `SetLayeredWindowAttributes(alpha=1)`：視覺上完全透明，但仍接收滑鼠
   - window class 的 `hCursor = NULL`，且 `WM_SETCURSOR` 直接 `SetCursor(NULL)` 回 1
   - 因為 overlay 在游標底下，`WM_SETCURSOR` 一定是由我們的視窗處理，游標就不會回來
2. **ClipCursor 改成邊緣 1×1**：即使某些情況游標仍顯示，也只會固定在一個點，
   不會沿螢幕邊緣上下跑。
3. 回切（`do_release`）或程式結束（`CursorGuard::drop`）時：
   `DestroyWindow` + `ShowCursor(true)`，游標立即恢復。
4. 建立失敗時 log 會顯示 `cursor hider failed`，仍保有 1×1 clip 的退路。

緊急取回仍是 `Ctrl+Alt+Shift+Q`（全域 LL hook，overlay 不影響）。

### 2.3 預期 log

```
[master] remote control ON (Right edge, entry ratio 32768, cursor hider on)
[master] remote control OFF (edge)
```

---

## 3. 如何驗收

兩台都安裝 `LAN-KVM-Setup-0.3-clip2.exe`，確認配對與自動連線正常，然後：

**剪貼簿**

1. A 複製一段文字 → B 貼上（記事本）。
2. B 複製一段文字 → A 貼上。
3. A 複製一張圖片（例如截圖工具）→ B 貼到小畫家。
4. **在 B 建立一個唯一檔名的檔案**（例如 `B-測試-123.txt`），複製後到 A 貼上，
   確認檔名與內容都是 B 的那一份。
5. 檔案貼上後**立刻**再複製一段文字，確認文字也會同步（舊版的 1.5 秒窗會漏掉）。
6. A 複製大檔案（> 128 MB）→ log 應出現 `clipboard files: skipping … (size limit)`，
   且不影響後續同步。

**游標**

1. A 推過邊界控制 B：A 的游標應消失，且不再沿邊緣上下移動。
2. 把 B 的游標推回進入邊界：控制權回 A，游標立即出現。
3. 在 B 上按 `Ctrl+Alt+Shift+Q`（從 A 的鍵盤）可強制取回。

**收集紀錄**後應包含兩台的 `app.log`，搜尋 `clipboard` 可看到上述事件。

---

## 4. 已知問題／下一步

- 剪貼簿連線目前與 P0 session 無關（只要配對就同步）；之後應與 QUIC 串流整合。
- 檔案是先整包讀進記憶體再送，超大檔案會吃 RAM；下一版改成分塊串流。
- 尚未支援「遠端剪貼簿格式化文字（HTML/RTF）」與「拖放」。
- 游標 overlay 會擋住 master 本機點擊（控制期間本來就該如此）；尚未做「保留本機點擊」模式。
