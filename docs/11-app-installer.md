# v0.2 App：安裝版常駐程式（探索 / 配對 / 自動連線）

> 日期：2026-10-23  
> 版本：`v0.2-app`  
> 目的：把 P0 指令列測試包裝成可安裝的常駐程式，讓一般使用者不用開 PowerShell、不用手動加防火牆。

---

## 1. 這個版本包含什麼

| 功能 | 狀態 |
|------|------|
| 安裝程式 `setup.exe` | 完成 |
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
| Windows 登入畫面 / UAC | 尚未整合（需要 SYSTEM service） |

---

## 2. 架構

```
lankvm-app.exe
├─ HTTP UI 127.0.0.1:45880（內嵌 HTML，開瀏覽器）
├─ 探索 UDP 45870（multicast 239.255.42.99 + 255.255.255.255）
├─ 控制 UDP 45871（配對、session_start/stop、unpair）
├─ Supervisor：每 2 秒檢查配對與自動連線
└─ 子程序 lankvm-app.exe --engine master|client
      └─ 資料 UDP 45872（沿用 P0 28-byte 封包）
```

- **沒有 console**：`lankvm-app.exe` 與 `setup.exe` 都是 `windows_subsystem = "windows"`。
- **Engine 與 UI 分離**：engine 是另一個隱藏子程序，停止時用 stop-file 觸發 `poc::request_stop()`，master 會正常解除 `ClipCursor`／恢復游標，不會留下卡住狀態。
- **不需要管理員執行 App**：只有安裝程式需要 UAC（防火牆 + Program Files）。App 用一般權限在使用者 session 執行 hook/injection（Default 桌面）。
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

1. 兩台都解壓 `LAN-KVM-0.2-app.zip`，執行 `setup.exe`（UAC 按是）。
2. 安裝到 `C:\Program Files\LAN-KVM`，自動加防火牆、開機啟動、捷徑。
3. A 開 UI → 掃描 → 選 B → 配對。
4. B 會自動開 UI 顯示 3 位碼；A 輸入。
5. 配對成功後自動連線；之後開機自動連。
6. A 的 UI 設定 B 在左／右、切入推力、回切門檻。

移除：UI「結束程式」→ 刪除 `C:\Program Files\LAN-KVM` → 清 Run key／防火牆規則／捷徑（詳見 `dist-app/README.txt`）。

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
| Release build 重跑配對與自動連線 | PASS |

> 單機只驗證「探索／配對／自動連線／停止」流程；實際滑鼠跨邊界仍需兩台實機。

---

## 6. 產物

```
dist-app/
  lankvm-app.exe   D4CF45AF474A8D6E6627FB9481963590FBC22FD418BC0DBFC507E9954A3BD229
  setup.exe        DC9EC8B2F01850022A164C81606DFD9C8B047EB19B3C633AE02010AB2B57F3AA
  README.txt
LAN-KVM-0.2-app.zip  97C695AD397CBF9E351F65E1D0D0E022281411536141970A4B17529826A3396F
```

建置：

```powershell
cargo build --release -p lankvm-app -p lankvm-setup
```

---

## 7. 已知限制

1. **未加密**：資料仍是 P0 UDP；只可在信任網段。
2. **配對碼 3 位數**、無嘗試次數限制；正式版換 SPAKE2/憑證 pinning。
3. **只控制 Default 桌面**：Winlogon/UAC 尚未接進 App；舊的 SYSTEM service 在安裝時會被移除，因此這一版不控制登入畫面。
4. **未簽章**：McAfee/Apex One 可能隔離；需要 IT 白名單或簽章。
5. **沒有 tray icon**：常駐在背景，從捷徑開 UI；要結束從 UI 按。
6. **安裝器的 HKCU Run 寫入**：若使用者用「其他管理員帳號」提權，自動啟動會裝到該帳號；一般本機管理員不受影響。
7. **多網卡**：探索會記錄封包來源 IP；VirtualBox/Tailscale 多網卡時可能顯示非主要 IP，但通常仍可連。
8. **未實作多螢幕版面**：目前是單一左/右關係。

---

## 8. 下一步

1. 兩台實機安裝測試：探索、配對、開機自動連、來回切換、解除配對。
2. 把 SYSTEM service 接回來，讓 App 也能控制 Winlogon/UAC（需要 engine 以 SYSTEM 身分啟動 session agent）。
3. QUIC/TLS + SPAKE2/憑證，取代 UDP + 3 位碼。
4. 程式簽章與 AV 白名單。
5. Tray icon、開機靜默、UI 佈局畫布（多螢幕）。
6. 連線健康度：engine 崩潰自動恢復、RTT/狀態顯示。
