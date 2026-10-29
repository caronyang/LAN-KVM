# 紀錄輪替與收集（v1.1.8）

> 適用版本：`1.1.8` 以上
> 相關程式：`crates/lankvm-platform/src/logfile.rs`、`crates/lankvm-service/src/logger.rs`、
> `crates/lankvm-app/src/{state,engine,logs,http}.rs`

## 1. 為什麼要改

1.1.7 以前所有紀錄檔都是單純的 append：

- `session.log`：session agent 每 5 秒寫一行 input counts，閒置 24/7 約 **1.6 MB/天**，
  實際受控時可到 **4 MB/天**
- `app.log`、`service.log`：事件驅動，成長較慢
- `engine.log`：每次 engine 啟動 append，只有「下次啟動且超過 8 MB」才刪除

成長本身很慢（一般使用約 10–50 MB/年），但 **收集紀錄** 的流程會把整個檔案讀進記憶體再整包送出。
log 累積到數百 MB 之後，收集會變慢、記憶體暴增，甚至超過 15 秒 write timeout 而失敗——
偏偏那正是需要 log 來診斷的時候。

## 2. 上限政策

| 檔案 | 位置 | 單檔上限 | 保留世代 | 最大佔用 |
|------|------|----------|----------|----------|
| `session.log` | `C:\ProgramData\LAN-KVM\logs` | 5 MB | 3（`.1`–`.3`） | 20 MB |
| `service.log` | `C:\ProgramData\LAN-KVM\logs` | 5 MB | 3 | 20 MB |
| `app.log` | `%LOCALAPPDATA%\LAN-KVM` | 2 MB | 3 | 8 MB |
| `engine.log` | `%LOCALAPPDATA%\LAN-KVM` | 8 MB | 1（上一次執行） | 16 MB |

合計上限約 **64 MB**，不會再無限制成長。

## 3. 輪替機制

`lankvm_platform::logfile`：

- 每個 writer 只累計自己寫入的 bytes，**每 1 MiB 才檢查一次實際檔案大小**，
  所以熱路徑仍然只有一次 `write_all`，不會每行都 `stat`
- 超過上限時：刪除最舊的 `.N` → `.N-1` 改名為 `.N` → … → 目前檔案改名為 `.1`
- 檔案handle 會在輪替前關閉（Windows 不能 rename 仍開啟的檔案）
- 多個 process 同時寫同一個檔案時，rename 可能失敗；失敗會被忽略，下一個 1 MiB 再重試

`engine.log` 只在 **engine 啟動時** 輪替（8 MB）。engine 的 stdout/stderr 直接寫入檔案、
不經過 pipe，這是為了避免 log I/O 卡住輸入路徑；因此單次連線中不會中途輪替，
但每次啟動都會把上一次的結果保留為 `engine.log.1`。

## 4. 收集紀錄（bundle）

- 每個檔案只帶 **最後 2 MB**；有截斷時會在 bundle 的 `README.txt` 註明
  `truncated, kept last N of M bytes`
- 打包改為 **串流**：邊讀檔邊寫 socket，不再把整包放進 `Vec<u8>`
- 接收端也改為串流：邊讀 socket 邊寫檔，只保留 headers 的少量緩衝
- 整包上限 64 MB；超過會直接拒絕，避免對端送來超大內容

## 5. UI

「紀錄」卡片新增：

- 目前各紀錄檔大小與合計（每秒随 `/api/state` 更新）
- **清除紀錄** 按鈕：刪除所有世代（`.1`–`.3`）
  - 連線中會保留 `engine.log`，避免丟掉進行中的輸出
  - `ProgramData` 的檔案由 SYSTEM 建立，部分機器需要系統管理員權限；
    失敗時會顯示 Windows 的錯誤訊息

API：

```
POST /api/logs/clear  ->  {"ok":true,"cleared":[...],"skipped":[...]}
GET  /api/state       ->  ... "log_files":[{"name","bytes"}], "log_total_bytes":N
```

## 6. 驗證

```powershell
cargo test -p lankvm-platform      # rotate / tail_text
cargo test -p lankvm-app           # bundle 串流 round-trip / 截斷拒絕 / copy_tail
cargo clippy --workspace --all-targets -- -D warnings
```

手動：

1. 讓某個 log 超過上限（例如暫時把 `ROTATION` 調小），確認出現 `xxx.log.1`
2. 按「收集紀錄」，確認 bundle 內每個檔案 ≤ 2 MB，`README.txt` 有截斷說明
3. 按「清除紀錄」，確認 `C:\ProgramData\LAN-KVM\logs` 與 `%LOCALAPPDATA%\LAN-KVM` 的
   `*.log*` 都被刪除，且服務在數秒內重建 `session.log`
