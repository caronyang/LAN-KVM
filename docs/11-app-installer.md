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

---

## 6. 產物

```
dist-app/
  LAN-KVM-Setup-0.2.exe   5E3AD323EF178F690ABB9FF391A5C22BDE651A5A7F728E59023960B3E4F5652A
  README.txt
```

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
