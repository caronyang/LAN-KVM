# v0.2 App：安裝版常駐程式（探索 / 配對 / 自動連線）

> 日期：2026-10-23  
> 版本：`v0.2-app`  
> 目的：把 P0 指令列測試包裝成可安裝的常駐程式，讓一般使用者不用開 PowerShell、不用手動加防火牆。

---

## 1. 這個版本包含什麼

| 功能 | 狀態 |
|------|------|
| 安裝程式 | 單一 `LAN-KVM-Setup-0.2.exe`（內嵌 App 與服務） |
| 系統匣常駐 | 完成：左鍵開 UI、右鍵選單（連線/掃描/收集紀錄/結束） |
| 不顯示 console | session agent 以 `CREATE_NO_WINDOW` 啟動 |
| A 收集時一併抓 B 的 log | 完成：TCP 45873 回傳，存成 `peer-<名稱>/` |
| 自動防火牆規則（UDP 45870/45871/45872） | 完成（安裝時） |
| 開機自動啟動（HKCU Run，`--silent`） | 完成 |
| 桌面／開始功能表捷徑 | 完成 |
| 本機圖形介面（瀏覽器，`127.0.0.1:45880`） | 完成 |
| 區網自動探索（multicast + broadcast） | 完成，單機雙 instance 驗證 |
| 3 位數配對碼 | 完成 |
| 配對後開機自動連線 | 完成，單機驗證 |
| 解除配對後停止自動連線 | 完成 |
| 左／右版面與切換門檻設定 | 完成 |
| 滑鼠來回切換（A→B→A） | 沿用 P0，已於前一階段驗證 |
| Windows 登入畫面 / UAC | 已整合 SYSTEM 服務（待兩台實機驗收） |
| 一鍵收集測試 log（兩台） | 完成：UI「收集紀錄」 |

---

## 2. 架構

```
LAN-KVM-Setup-0.2.exe（單一檔案）
└─ 解壓到 C:\Program Files\LAN-KVM
   ├─ lankvm-app.exe（常駐 + 系統匣 + 瀏覽器 UI + master engine）
   └─ lankvm-service.exe（SYSTEM client 注入，Winlogon/UAC）

lankvm-app.exe
├─ HTTP UI 127.0.0.1:45880（內嵌 HTML）
├─ 系統匣圖示：左鍵開 UI、右鍵選單
├─ 探索 UDP 45870（multicast 239.255.42.99 + 255.255.255.255）
├─ 控制 UDP 45871（配對、session_start/stop、unpair）
├─ Supervisor：每 2 秒檢查配對與自動連線、每 5 秒探測 client 引擎
└─ lankvm-app.exe --engine master（只有 master 端是隱藏子程序）
      └─ 資料 UDP 45872（沿用 P0 28-byte 封包）

LAN-KVM SYSTEM service（client 端）
├─ 常駐 LocalSystem，開機自動啟動
├─ session agent 以 CREATE_NO_WINDOW 啟動（不顯示黑色指令視窗）
├─ session agent 在 Default / Winlogon 桌面上注入
└─ 監聽 UDP 45872；App 用 Ping/Pong（d=1）探測是否就緒

Log 回傳
└─ TCP 45873：A 收集紀錄時，B 把打包日誌 POST 回 A
```

- **沒有 console**：`lankvm-app.exe`、`lankvm-setup.exe`、`lankvm-service.exe` 都不開視窗。
- **Master / client 分離**：master 端是 App 的隱藏 engine 子程序；client 端是 SYSTEM 服務。因此 B 鎖定或跳 UAC 時，注入仍由服務完成。
- **Engine 停止**：master 用 stop-file 觸發 `poc::request_stop()`，會正常解除 `ClipCursor`／恢復游標；App 被強制結束時，engine 的 parent watchdog 也會接手。
- **不需要管理員執行 App**：只有安裝程式需要 UAC（防火牆、Program Files、服務）。App 用一般權限在使用者 session 執行。
- **配對角色**：發起配對的那台是 **master**（有鍵鼠），另一台是 **client**。配對資料會記住角色，開機後 master 自動連線。

---

## 3. 通訊協定

探索（UDP 45870，JSON）：

```json
{"type":"discover","id":"...","name":"...","control_port":45871,"data_port":45872}
{"type":"peer","id":"...","name":"...","control_port":45871,"data_port":45872}
```

配對（UDP 45871）：

```json
{"type":"pair_request", ...}
{"type":"pair_challenge"}
{"type":"pair_proof","code":"482", ...}
{"type":"pair_ok", ...}
{"type":"pair_fail"}
```

連線：

```json
{"type":"session_start","edge":"right", ...}
{"type":"session_stop"}
{"type":"unpair"}
```

配對碼只顯示在 host（被選擇的那台），guest 輸入；host 不會把碼傳給 guest。

---

## 4. 安裝與使用

1. 兩台都執行 `LAN-KVM-Setup-0.2.exe`（單一檔案，UAC 按是）。
2. 安裝到 `C:\Program Files\LAN-KVM`，自動加防火牆、開機啟動、捷徑、SYSTEM 服務。
3. 安裝後 App 常駐系統匣；從匣圖示或桌面捷徑開 UI。
4. A 開 UI → 掃描 → 選 B → 配對。
5. B 會自動開 UI 顯示 3 位碼；A 輸入。
6. 配對成功後自動連線；之後開機自動連（系統匣）。
7. A 的 UI 設定 B 在左／右、切入推力、回切門檻。

移除：UI「結束程式」→ 以管理員執行 `lankvm-service.exe uninstall` → 刪除 `C:\Program Files\LAN-KVM` → 清 Run key／防火牆規則／捷徑（詳見 `dist-app/README.txt`）。

### 4.1 收集測試紀錄

兩台測完，任一台按 UI 右上「收集紀錄」或系統匣右鍵「收集紀錄」。它會：

1. 收集本機的 app/engine/service/session/system/setup 日誌。
2. 透過 TCP 45873 向**已配對且在線**的另一台要 log（對方會打包回傳）。
3. 在桌面（或 OneDrive 桌面／使用者目錄）建立：

```
LAN-KVM-logs-<電腦名>-<時間>
  app.log engine.log app.json state.json
  service.log session.log
  system.txt
  setup.log
  peer-<對方名稱>/     ← 對方回傳的完整日誌
  README.txt
```

把整個資料夾交給開發者即可，不需要手動去另一台找 log。

> 需要防火牆允許 TCP 45873；安裝程式會自動加 `LAN-KVM Logs` 規則。

---

## 5. 單機模擬驗證（2026-10-23）

同一台電腦跑兩個 instance（不同 HTTP/control/data port、同一探索埠）：

| 項目 | 結果 |
|------|------|
| 兩 instance 互相探索 | PASS（各自看到對方） |
| A 發起配對、B 顯示 805 | PASS |
| A 輸入配對碼 | PASS |
| 配對後自動建立 session（A=master、B=client） | PASS |
| 停止 session | PASS（`[master] stopped`，engine 正常退出） |
| 解除配對 | PASS（兩邊都移除、session 停止） |
| 先開 B 再開 A（模擬開機順序） | PASS（A 自動連上 B） |
| 強制結束 A 的 App（模擬崩潰） | PASS（A 的 engine 由 parent watchdog 正常停止；B client 保留） |
| 重啟 A 的 App | PASS（自動重連，沿用 B 既有的 client） |
| client 改走 SYSTEM 服務（session agent 模擬） | PASS（B session `uses_service=true`、master 正常連線） |
| 系統匣圖示 | PASS（`tray: icon installed`，exe icon 為 Seam） |
| 單一 setup 內嵌 App/服務 | PASS（`--extract` 出來的 hash 與 build 完全一致） |
| A 收集時抓 B 的 log | PASS（`peer-<名稱>/`） |
| session agent 無 console | 已改 `CREATE_NO_WINDOW`（待實機確認） |
| 服務重啟後 master 自動重置 | 已加 `session_reset`（待實機確認） |
| UI「收集紀錄」 | PASS（含 app/engine/service/session/system/setup 日誌） |
| Release build 重跑配對與自動連線 | PASS |

> 單機只驗證「探索／配對／自動連線／停止」流程；實際滑鼠跨邊界仍需兩台實機。

### 5.1 Winlogon 密碼欄位卡死（2026-10-23 實機）

證據（B 的 `session.log`）：

```
input desktop -> Winlogon
Winlogon: injector active
Winlogon: input counts ... injected=183 inject_fail=183
```

Default 桌面 `inject_fail=0`；只有點擊密碼欄位、切到 Winlogon secure desktop 後全部失敗。A 的 master 仍停在 remote，游標被釘住、鍵盤被攔，所以兩台都不能操作。

第二輪驗證（thread attach）：

```
Winlogon: injector desktop attach=true err=0
TokenUIAccess=true err=0
Winlogon: inject failed kind=Motion streak=1 err=存取被拒 (os error 5)
```

結論：**把 injector thread attach 到 Winlogon、UIAccess=true 都還不夠**；`SendInput` 在 secure desktop 仍被拒。自動 Release 會讓登入畫面完全無法測試，已移除。

目前修正（helper process）：
- 在 session agent 啟動時就預先建立 `lankvm-service.exe winjector --port 45874`，以 `lpDesktop = winsta0\Winlogon` 啟動，**process 一开始就在 secure desktop**，且**保持常駐**。
- 輸入事件透過 loopback TCP 45874 轉給 helper；Default 時只 disable 路由，不重建 process。
- 鎖定時只 enable 路由，避免 1–2 秒的 process 建立/連線空窗。
- Default 桌面仍走原本的 injector（正常）。
- helper 的 log：`winjector: listening`、`winjector: agent connected`、`winjector: enabled (Winlogon)`、`winjector: SendInput failed kind=... err=...`。

> 2026-10-23 晚實機：密碼欄位已可點擊並解鎖，但切換時有 1–2 秒卡頓；本版已改預啟動/常駐連線。

### 5.2 Num Lock / 數字鍵盤

- Num Lock、Caps Lock、Scroll Lock 是 toggle key；用 scan code 的 `SendInput` **不會更新 toggle 狀態**。
- 已改為 `keybd_event` 的 virtual key 路徑（Num=0x90、Caps=0x14、Scroll=0x91），scan 仍保留給一般鍵。
- 本機以服務 agent + Switch 封包驗證 Num Lock 會切換；數字鍵盤需 Num Lock 開啟後才有數字。

### 5.3 B 先開機、停在登入頁（ID 廣播 + 換 IP）

問題：B 的 App 要登入後才啟動，登入頁只有 SYSTEM 服務；A 不能用「上次配對的 IP」保證找到 B（DHCP 可能換 IP）。

做法：
- App 首次啟動就產生持久化 `id`（存在 `app.json`），配對時記錄雙方 `id`、`name`、IP、port。
- **服務本身（session 0）與 session agent 都會**在背景讀取 `C:\Users\*\AppData\Local\LAN-KVM\app.json`，每 3 秒廣播：
  ```json
  {"type":"peer","id":"...","name":"...","control_port":45871,"data_port":45872,"service":true}
  ```
  同時**unicast 到配對時記住的對方 IP:45870**，所以即使廣播被 AP/防火牆擋掉，對方換 IP 時仍能被找到。
- A 的 App 收到後用 **id** 比對已配對資料；IP 或 port 變更就更新 `app.json`。
- 若 A 已有 master 連到舊 IP，會 graceful stop 後由 supervisor 用新 IP 重連。
- B 的 App 登入後也會廣播；同一 id，不會重複配對。

Log：
- B：`identity announce id=... name=... control=45871 data=45872`
- A：`peer <id> IP updated old -> new`、`peer address changed; master stopped for a clean reconnect`

> 這樣 B 換 IP、或 B 先開機停在登入頁，A 都能用 ID 找到它；不需要手動改 IP。

### 5.4 服務自我修復（開機/登入頁）

- 服務安裝改成 **idempotent**：已存在也會更新為 `AutoStart`、更新 binary path，並確保啟動；不會再只印 “already installed”。
- 服務每 5 秒對 agent 的 UDP 45872 做 P0 Ping/Pong（要求 `d=1`）：
  - 連續 3 次失敗 → 停止並重啟 session agent。
  - log：`client health probe failed (n/3)`、`restarting session agent: UDP client not responding`。
- agent 的 UDP client bind 失敗時**直接退出**，讓服務重拉，不再留下沒有 listener 的半死程序。
- log：`active session -> N`、`launch_session ... desktop=Default failed ... trying Winlogon`、`launch_session ... OK`、`session agent ... exited code=...`。
- 安裝程式額外建立：
  - **ONSTART 排程 `LAN-KVM Boot Start`**（SYSTEM 執行 `sc start LAN-KVM`），避免 SCM 沒自動起服務。
  - 服務失敗自動重啟（`sc failure ... restart/5000/restart/15000/restart/60000`）。
  - 安裝記錄 `%TEMP%\lankvm-setup.log` 會寫 `boot task -> true/false`。
- UI：紅色警告 banner 移到頁面最上方（跨兩欄），不再把右欄卡片往下推。

---

## 6. 產物

```
dist-app/
  LAN-KVM-Setup-0.2-login3.exe   1AB6DE4A80BC37B5A97864A8560BA958C0CBF9B74AF8BDBA7E386C24B6A56FA2
  README.txt
```

> `LAN-KVM-Setup-0.2.exe`（5680）是上一個測試版；`-login` 版多了登入頁 session/桌面 fallback 與啟動診斷。若安裝程式正在執行，請先完成或關閉，再用 `-login` 版。

建置（順序很重要，setup 會 `include_bytes!` 前一步的產物）：

```powershell
cargo build --release -p lankvm-app -p lankvm-service
cargo build --release -p lankvm-setup
```

驗證單一安裝檔內容：

```powershell
.\LAN-KVM-Setup-0.2.exe --extract $env:TEMP\lk-extract
Get-FileHash $env:TEMP\lk-extract\*.exe
```

`--extract` 不需要管理員權限，只用來檢查內嵌檔案。

---

## 7. 已知限制

1. **未加密**：資料仍是 P0 UDP；只可在信任網段。
2. **配對碼 3 位數**、無嘗試次數限制；正式版換 SPAKE2/憑證 pinning。
3. **client 注入依賴 SYSTEM 服務**：若服務被防毒隔離或未啟動，UI 會顯示未就緒，且無法控制 Winlogon/UAC。安裝記錄在 `%TEMP%\lankvm-setup.log`。
4. **未簽章**：McAfee/Apex One 可能隔離；需要 IT 白名單或簽章。
5. **沒有 tray icon**：常駐在背景，從捷徑開 UI；要結束從 UI 按。
6. **安裝器的 HKCU Run 寫入**：若使用者用「其他管理員帳號」提權，自動啟動會裝到該帳號；一般本機管理員不受影響。
7. **多網卡**：探索會記錄封包來源 IP；VirtualBox/Tailscale 多網卡時可能顯示非主要 IP，但通常仍可連。
8. **未實作多螢幕版面**：目前是單一左/右關係。
9. **Windows 11 系統匣**：新圖示預設收在「^」；第一次要手動拖到工作列固定。這是 OS 行為，程式無法強制。

---

## 8. 下一步

1. 兩台實機安裝測試：探索、配對、開機自動連、來回切換、**Winlogon/UAC**、解除配對、收集 log。
2. QUIC/TLS + SPAKE2/憑證，取代 UDP + 3 位碼。
3. 程式簽章與 AV 白名單。
4. Tray icon、UI 佈局畫布（多螢幕）。
5. 連線健康度：RTT/狀態顯示、傳輸加密後的連線恢復。
