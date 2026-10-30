# LAN-KVM 1.1.8

> Released 2026-10-29 · English | [繁體中文](#繁體中文)

## English

### Summary

1.1.8 gives every log file a size cap, keeps only the tail of each file in log bundles,
streams bundles instead of buffering them in memory, and adds log sizes plus a **Clear logs**
button to the UI. No protocol, input-path or behaviour changes.

### Problem

Before 1.1.8 every log was plain append-only with no upper bound:

- `session.log` writes an input counter every 5 seconds — about **1.6 MB/day** when idle and
  up to **4 MB/day** while controlling a peer
- `app.log` and `service.log` grow with events (slower)
- `engine.log` was only deleted at the next engine start, and only if it exceeded 8 MB

Growth is slow (typically 10–50 MB/year), but log collection (`/api/logs`) read every file
fully into memory, concatenated them into one buffer, and the receiving side buffered the
whole body before parsing. At hundreds of megabytes that becomes slow and memory-hungry, and
can exceed the 15 s write timeout — exactly when logs are needed for diagnosis.

### Fix

#### Rotation (new `lankvm-platform::logfile`)

| File | Limit | Generations | Max on disk |
|------|-------|-------------|-------------|
| `session.log` | 5 MB | 3 (`.1`–`.3`) | 20 MB |
| `service.log` | 5 MB | 3 | 20 MB |
| `app.log` | 2 MB | 3 | 8 MB |
| `engine.log` | 8 MB | 1 (previous run) | 16 MB |

- The file size is checked **once per MiB written**, so the hot path stays a single `write_all`
- Handles are closed before renaming; a failed rename is retried at the next probe
- `engine.log` rotates at every engine start (it writes straight to the file, no pipe, so
  rotating mid-session would touch the input path)

#### Bundles

- Only the **last 2 MB** of each file; truncation is recorded in `README.txt` as
  `truncated, kept last N of M bytes`
- Packing and unpacking are now streamed end to end; the bundle is never held in memory
- Hard cap of 64 MB per bundle

#### UI / API

- The log card shows each file's size and the total
- **Clear logs** button (`POST /api/logs/clear`) removes rotated generations too;
  `engine.log` is kept while a session is running
- `/api/state` adds `log_files` and `log_total_bytes`
- The UI reads only the last 64 KB of `engine.log` instead of the whole file every second

### Files

```
LAN-KVM-Setup-1.1.8.exe
size   2,887,574 bytes
SHA256 4FB80C5237989700E527C283F30C90D821D0FE70F92DC5582885775C765457FA
```

### Verification

- [ ] Log sizes are shown in the UI after install or upgrade
- [ ] `session.log` over 5 MB rotates to `session.log.1` and keeps only 3 generations
- [ ] `app.log` over 2 MB rotates to `app.log.1`
- [ ] `engine.log` over 8 MB becomes `engine.log.1` at the next engine start
- [ ] A bundle contains at most 2 MB per file and `README.txt` notes the truncation
- [ ] Clear logs removes local and ProgramData `*.log*`; the service recreates its files
- [ ] Clearing during a session keeps `engine.log` and says so
- [ ] Edge switching, sign-in screen, clipboard and Num Lock behave like 1.1.7

### Notes

- No protocol or input-path changes.
- ProgramData logs are created by SYSTEM; on some machines clearing them needs administrator
  rights and the UI shows the Windows error if it fails.
- The 1.1.7 data-port fix (default 45872, legacy `client_port=45871` migration, installer
  overwriting the config on upgrade) is retained.

---

## 繁體中文

> 重點：紀錄檔加上大小上限與輪替、log bundle 改為串流與尾端截斷、UI 顯示大小並可清除紀錄。
> 功能行為與 1.1.7 相同，沒有協議或連線流程變更。

### 1. 問題

1.1.7 以前所有紀錄檔都是 append-only：

- `session.log` 每 5 秒一行 input counts，閒置 24/7 約 1.6 MB/天
- `app.log`、`service.log` 事件驅動，成長較慢
- `engine.log` 只在下次啟動且超過 8 MB 時刪除

成長速度不快（一般使用約 10–50 MB/年），但 `/api/logs` 收集紀錄時會：

1. 把整個檔案 `fs::read` 進記憶體
2. 把所有檔案串成一個大 `Vec<u8>`
3. 對端也先把整個 body 讀完才解析

log 累積到數百 MB 後，收集會變慢、記憶體暴增，甚至超過 15 秒 write timeout 而失敗。

### 2. 修正內容

#### 輪替（新 `lankvm-platform::logfile`）

| 檔案 | 上限 | 保留 |
|------|------|------|
| `session.log` | 5 MB | 3 代 |
| `service.log` | 5 MB | 3 代 |
| `app.log` | 2 MB | 3 代 |
| `engine.log` | 8 MB | 1 代（上一次執行） |

- 每寫 1 MiB 才檢查一次大小，熱路徑不多做 syscall
- 輪替前先關閉 handle；rename 失敗（其他 process 正在寫）會在下次重試
- `engine.log` 於每次 engine 啟動時輪替，保留上一次 hang 掉的紀錄

#### 收集紀錄

- 每個檔案只保留 **最後 2 MB**，截斷時在 `README.txt` 註明
- 打包與接收都改為串流，不再把整包放在記憶體
- 整包上限 64 MB

#### UI / API

- 紀錄卡片顯示各檔案大小與合計
- 新增「清除紀錄」按鈕（含 rotate 過的世代；連線中保留 `engine.log`）
- 新增 `POST /api/logs/clear`
- `/api/state` 新增 `log_files`、`log_total_bytes`
- UI 讀取 `engine.log` 改為只讀最後 64 KB，不再每秒讀整個檔案

### 3. 檔案

```
dist-app\LAN-KVM-Setup-1.1.8.exe
size   2,887,574 bytes
SHA256 4FB80C5237989700E527C283F30C90D821D0FE70F92DC5582885775C765457FA
```

建置：

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd
```

### 4. 驗收清單

- [ ] 乾淨安裝／從 1.1.7 升級後，UI 顯示紀錄檔大小
- [ ] `session.log` 超過 5 MB 後出現 `session.log.1`，且只保留 3 代
- [ ] `app.log` 超過 2 MB 後出現 `app.log.1`
- [ ] `engine.log` 超過 8 MB 後，下次連線啟動時變成 `engine.log.1`
- [ ] 收集紀錄：bundle 內每個檔案 ≤ 2 MB、`README.txt` 有截斷說明
- [ ] 清除紀錄：本機與 ProgramData 的 `*.log*` 都被刪除，服務數秒內重建
- [ ] 連線中按清除：`engine.log` 被保留並顯示提示
- [ ] 邊界切換、登入畫面、剪貼簿行為與 1.1.7 相同

### 5. 備註

- 這一版沒有改動 P0 協定、連線流程或輸入路徑。
- `ProgramData` 的紀錄檔由 SYSTEM 建立；部分機器上清除可能需要系統管理員權限，
  失敗時 UI 會顯示 Windows 的錯誤訊息。
- 1.1.7 的 data-port 修正（預設 45872、舊 `client_port=45871` 遷移、安裝程式覆寫 config）
  全部保留。
