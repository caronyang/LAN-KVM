# P0 PoC 使用說明（Windows）

目前進度：**Rust workspace + Windows 診斷工具 + 低階 hook + 兩台 UDP 鍵鼠轉發 PoC**。  
尚未包含：服務/登入畫面、配對/加密、剪貼簿、檔案、UI。

> 重要：`lankvm-devtools.exe` 是**開發工具**，會安裝全域 hook、記錄按鍵、注入輸入、開未加密 UDP。**不要散布給使用者**。正式產品是 `lankvm-agent.exe`，不含這些開發指令。

> 這個 PoC 沒有加密、沒有配對、沒有權限控制。**只在可信任的區域網路與專用測試機測試**，不要暴露到公網。

## 建置

```powershell
cd D:\99_Temp\opencode\LAN-KVM
cargo build --release
```

產出：

```
target\release\lankvm-agent.exe      # 產品骨架（probe/version）
target\release\lankvm-devtools.exe   # 開發工具（hook/inject/poc）
```

> Apex One 目前會刪除 release 版 `lankvm-devtools.exe`。開發測試請用：

```powershell
cargo build -p lankvm-devtools
cargo build --profile dev-fast -p lankvm-devtools
```

- `target\debug\lankvm-devtools.exe`：未最佳化，功能測試用。
- `target\dev-fast\lankvm-devtools.exe`：`opt-level=2` + debug info，**建議用這個測 P0 延遲**。
- 以下指令以 `target\debug` 為例；若 dev-fast 可用，把路徑換成 `target\dev-fast` 即可。

需要 Rust stable（本機使用 1.86 / MSVC）。

---

## 1. 系統診斷：`probe`

在 **A、B 兩台電腦** 分別執行：

```powershell
.\target\release\lankvm-agent.exe probe
```

輸出範例：

```
LAN-KVM probe
  pid: 7520
  session: 1 (active console session: 1)
  elevation: false / integrity: Medium
  input desktop: Default
  virtual bounds: 1920x1080+0,0
  cursor: (680, 646)
  monitors:
    #0 \\.\DISPLAY1 1920x1080+0,0 dpi=96 primary
```

請把兩台輸出貼回來，我們需要確認：

- `session` / `active console session` 是否一致。
- `input desktop` 是否為 `Default`（登入後）；鎖定/UAC 時應為 `Winlogon`。
- `virtual bounds` 與 monitor 的 `rect`、`dpi`（多螢幕/混合 DPI 的基礎）。
- `elevation` / `integrity`（一般模式應為 Medium；進階模式應為 High/System）。

### 已知限制

- 一般權限執行時，`OpenInputDesktop` 只能看到目前使用者桌面；`Winlogon` 會失敗，這是正常的；登入畫面功能需要服務 + SYSTEM agent。
- 如果 `input desktop` 顯示 `unavailable (Access denied)`，代表目前桌面不是你的 session，或權限不足。

---

## 2. 游標注入檢查：`inject-test`

```powershell
.\target\release\lankvm-devtools.exe inject-test
```

會把游標往右（或左）移動 20 px 再移回來，並印出前後位置。用來確認 `SendInput` 沒有被 UIPI/安全軟體阻擋。

> 注意：相對移動會受 Windows「提高指標精確度」影響，實際位移可能不是剛好 20 px。這是正常的；v1 會改用絕對定位。

---

## 3. 輸入 hook 測試：`hook-test`

安裝 `WH_KEYBOARD_LL` / `WH_MOUSE_LL`，記錄按鍵與滑鼠按鈕事件，**不會**轉發或封鎖輸入（除非加 `--suppress`）。

```powershell
# 安全模式：只記錄，10 秒後自動停止
.\target\release\lankvm-devtools.exe hook-test --seconds 10
```

期間請按幾下鍵盤、點左右鍵、滾輪、移動滑鼠（只計數）。

輸出範例：

```
[key] DOWN vk=0x41 scan=0x1E ext=false injected=false extra=0x0
[mouse] LEFT DOWN at (800, 500) data=0x0 injected=false extra=0x0
...
Hooks removed. Mouse-move events observed: 123
```

### 封鎖測試（謹慎）

```powershell
.\target\release\lankvm-devtools.exe hook-test --seconds 5 --suppress --yes
```

- 只封鎖鍵盤與滑鼠按鈕/滾輪，**不封鎖滑鼠移動**。
- 5 秒後自動解除；不要在遠端連線或沒有實體鍵盤的機器上測試。

### 這個測試要驗證什麼

- hook 是否能在目前桌面安裝成功。
- callback 是否能在 1 ms 內完成（目前只列印按鍵/按鈕，不處理每個 move）。
- 注入事件是否帶 `LLKHF_INJECTED` / `LLMHF_INJECTED`。
- `dwExtraInfo` 是否保留（我們用 `0x4C4B564D` 作為 magic）。

---

## 4. 兩台鍵鼠轉發 PoC：`poc`

### 4.1 網路設定

兩台電腦必須在同一個網段、可以互相 UDP 連線。

1. Windows 防火牆若跳出詢問，允許 **私人網路** 的 `lankvm-devtools.exe`。
2. 預設 UDP port：`45871`。若被佔用，用 `--port` 改兩邊一致的 port。
3. 先確認兩台 `probe` 的 `virtual bounds`，以及 A 的右邊界就是 B 的左邊界（以 layout 來說）。

### 4.2 啟動順序

在 **B（被控端）** 先啟動：

```powershell
.\target\release\lankvm-devtools.exe poc --role client --port 45871 --seconds 60
```

在 **A（主控端）** 啟動，`--peer` 填 B 的 IP：

```powershell
.\target\release\lankvm-devtools.exe poc --role master --peer 192.168.1.20 --port 45871 --edge right --seconds 60
```

- `--edge right`：A 的右邊界連到 B 的左邊。
- `--seconds 60`：60 秒後自動停止；**第一次測試務必加**，避免游標卡住。
- `Ctrl+Alt+Shift+Q`：主控端強制取回控制。
- B 的游標碰到左邊界（往左推約 30 counts）會自動把控制權還給 A。

### 4.3 測試步驟

0. **先確認 B 與 A 都在同一個網段，且 B 的 IP 是 B 上 `ipconfig` 看到的位址。** 兩邊的 `--seconds` 要夠長（建議 120），並且在 B 啟動後 10 秒內啟動 A。
1. A 啟動後應先看到：

   ```
   [master] client online
   [latency] rtt=... ms
   ```

   如果沒有 `client online`，代表 A 的封包沒有到 B（IP 錯、防火牆、不同網段、B 已超時停止）；此時**不要**推邊界，先修網路。
2. 在 A 把游標推到螢幕**最右邊**，再往右推一點。
3. A 應該印出：

   ```
   [master] remote control ON (Right edge, entry ratio ...)
   ```

4. B 應該印出：

   ```
   [client] controlled; entry at (0, ...) from left
   ```

5. B 的游標會出現在左邊界、與 A 離開時相同高度。
6. 移動 A 的滑鼠 → B 的游標應該跟著動；鍵盤與滑鼠按鈕也應該在 B 生效。
7. 把 B 的游標往左邊界推 → B 印出 `[client] released at edge`，A 印出 `[master] remote control OFF (edge)`。
8. 觀察 A 的 log：

   ```
   [master] remote motion 123/s
   [latency] rtt=1 ms
   ```

### 4.4 判讀失敗

| 現象 | 可能原因 | 下一步 |
|------|----------|--------|
| A 沒有 `client online` | IP 錯、不同網段、B 防火牆、B 已停止 | 在 B 用 `ipconfig` 確認 IP；從 A `ping B_IP`；加 UDP 45871 防火牆規則；兩邊 `--seconds 120` 並在 10 秒內啟動 |
| A 顯示 `client not reachable` | 沒有最近的 PONG（client 離線/被封鎖） | 修網路；這是刻意保護，不會在此狀態隱藏游標 |
| A 沒有 `remote control ON` | Raw Input 沒收到、游標沒到邊界、左鍵按著 | 確認 `probe` 的 virtual bounds；放開左鍵；移動實體滑鼠 |
| A 顯示 `remote motion 0/s` | `ClipCursor` 後 Raw Input 停止，或 hook 沒收到 | 這是關鍵 PoC 問題；回報，我們改抓取方式 |
| A 顯示 `remote motion 56/s` 但 B 沒反應 | A 本地 hook/Raw Input 正常，問題在網路或 client 注入 | 看 B 是否有 `controlled`；沒有的話先修 `client online` |
| B 沒有 `controlled` | UDP 沒通、port/IP 錯、防火牆 | `ping`、確認兩邊 `--port`、允許防火牆 |
| B 有 `controlled` 但游標不動 | 注入被阻擋、B 在更高權限視窗 | 先測 `inject-test`；確認 B 是一般桌面 |
| 鍵盤卡鍵 | 斷線/結束時 key-up 沒送到 | 按一下該鍵；我們會加 release-all 保護 |
| 延遲很高 | Wi-Fi、AP isolation、背景大流量 | 改有線；看 `rtt` |

### 4.5 這個 PoC 的已知限制

- **未加密、未配對**：任何知道 port 的人都能送封包。只用在封閉測試網段。
- **只有單螢幕、右/左單一方向**：多螢幕與上下方向還沒接上。
- **相對注入**：B 的游標移動受該機的滑鼠加速影響，A/B 靈敏度可能不同。v1 會改絕對定位 + 自訂游標模型。
- **不支援管理員視窗/登入畫面**：目前是使用者權限；下一階段才做服務 + SYSTEM agent。
- **沒有剪貼簿/檔案**。
- **Raw Input + ClipCursor 需實機驗證**：這是 P0 最重要的未知數。

---

## 5. 如果 `lankvm-devtools.exe` 被防毒刪除

目前實測：**release 版 devtools 會被 Apex One 立即刪除；debug 版可執行**（`inject-test`、`hook-test`、`poc client` 都成功）。

```powershell
cargo build -p lankvm-devtools
.\target\debug\lankvm-devtools.exe help
```

- debug 版未最佳化，延遲數字會比正式版差；先用它驗證功能與邊界，不要拿來當正式效能基準。
- 這不是長期解法；正式產品仍要簽章 + 白名單。
- 若 debug 版也被刪，請改用隔離 VM 或請 IT 對單檔 SHA-256 加白名單。

## 6. 防毒軟體注意事項

`lankvm-devtools.exe` 的行為（全域 hook、鍵盤記錄、SendInput、未加密 UDP）**本來就會被防毒/EDR 視為高風險**，尤其是在企業環境（例如 Trend Micro Apex One）。這是開發工具，不是產品；不要把「讓防毒不擋」寄託在改名、加殼或關閉防毒。

- 開發測試機：請 IT/使用者把 **專案建置輸出資料夾**（例如 `D:\99_Temp\opencode\LAN-KVM\target`）加入排除，或只允許 `lankvm-devtools.exe` 的雜湊。**不要全域關閉防毒**。
- 正式產品：`lankvm-agent.exe` 不包含 hook 測試/鍵盤記錄/未加密 UDP；仍需要 **程式碼簽章 + 廠商白名單 + 使用者同意**，詳見 [`04-av-and-signing.md`](04-av-and-signing.md)。
- 若 Apex One 由公司集中管理，通常只有 IT 能加白名單；請提供偵測名稱、檔案 SHA-256、簽章者給 IT。

---

## 7. 下一步

1. 兩台實機跑 `probe`，回報輸出。
2. 兩台實機跑 `poc`，回報 A/B log 與主觀延遲。
3. 若 `remote motion 0/s` 或邊界不順，先修 Raw Input/ClipCursor。
4. 通過後做：服務 + SYSTEM agent + Winlogon 桌面切換（登入畫面）。
5. 再做：QUIC + mDNS/配對、剪貼簿、檔案、Tauri UI。

## 安全提醒

- `--suppress` 會暫時封鎖輸入；務必設定 `--seconds`。
- `poc` 會隱藏/釘住主控端游標；程式正常結束會還原。若當掉，登出/重新開機可恢復。
- 不要把 `target/`、log 或含個人資訊的截圖提交到 repo。
