# 服務 + 登入畫面 PoC（Windows）

> 狀態：**Winlogon 注入 PASS（本機封包驗證）**；A→B 的 UDP 被端點防護/網路政策阻擋，尚未完成跨機登入畫面控制。  
> 這一步的目標：服務能在 active console session 以 SYSTEM 啟動 session agent，看到 `Default` ↔ `Winlogon`，並接收 P0 UDP 封包在目前桌面 `SendInput`。

## 實測結果（2026-10-21）

在 B 鎖定畫面時，用本機 PowerShell 送 P0 `Switch` + `Motion` 到 `127.0.0.1:45871`：

- `session.log`：`Winlogon: injector worker installed` / `injector active`
- 登入畫面的游標**實際往右移動**
- 沒有出現 `inject_fail` 增加

**結論：SYSTEM session agent 可以在 Winlogon 安全桌面 `SendInput`。**  
剩下的問題是 A→B 的 UDP 被 Apex One/EDR 或 VM 網路擋住；Windows 防火牆不是原因（A 已關閉、B 規則存在且允許）。

## 1. 已實作

| 元件 | 內容 |
|------|------|
| `lankvm-service.exe` | Windows Service（LocalSystem） |
| 服務 supervisor | 監控 active console session，變更時重啟 session agent |
| session launcher | 複製 `winlogon.exe` token、設 `TokenUIAccess`、`CreateProcessAsUserW` 到 active session |
| session agent | 每 250 ms 檢查 `OpenInputDesktop`，記錄 `Default` / `Winlogon` 切換 |
| hook worker | 每個桌面一個執行緒：`OpenInputDesktop` → `SetThreadDesktop` → `WH_*_LL` → message loop |
| 輸入計數 | 每 5 秒記錄 `keys=N mouse_moves=N`，**不記錄按鍵內容** |
| P0 UDP client | `config.txt` 設定 `client_port` 後，session agent 監聽並注入 `Motion/Button/Key/Wheel/Switch/Release` |
| client 模式 | **不安裝 hook**，只開 injector thread；避免 hook thread 與 `SendInput` 衝突，也降低 AV 敏感度 |
| 安裝/移除 | `install` / `uninstall` / `start` / `stop`（需管理員） |
| 日誌 | `C:\ProgramData\LAN-KVM\logs\service.log`、`session.log` |

**尚未做：** QUIC/TLS、配對、多螢幕、剪貼簿/檔案、UI。  
目前 hook **不封鎖**輸入，所以本機鍵鼠仍會送到 Windows；P0 只在測試機使用。P0 UDP **未加密、未配對**，只適合隔離網段。

## 2. 建置

Apex One 目前會刪除 release 與 dev-fast 版 service binary。**目前最可靠的是 debug 版**：

```powershell
cd D:\99_Temp\opencode\LAN-KVM
cargo build -p lankvm-service
```

產出：

```
target\debug\lankvm-service.exe
```

> 若 `target\dev-fast\lankvm-service.exe` 還在，也可以用；但它經常在 build 後立即被 Apex One 刪除。

debug build SHA-256（目前可用）：

```
E2815155B40B7AC7B83E08DB214ACAD20C0B0D38BC7D866AF2A6CEC76C552A18
```

dev-fast build SHA-256（若檔案還在）：

```
8862A96EBC1C6D215A61278CD5D0328C220E775DF6D08E8E8D59E357572DC638
```

debug 版未最佳化，但 service/injector 邏輯相同，適合功能測試。

> 正式版仍需要簽章 + 白名單；service 會以 LocalSystem 執行，這是高敏感行為。

## 3. 安裝（管理員 PowerShell）

```powershell
.\target\debug\lankvm-service.exe install
```

預期：

```
Installed and started LAN-KVM from D:\...\lankvm-service.exe
Logs: C:\ProgramData\LAN-KVM\logs
```

檢查服務：

```powershell
sc.exe query LAN-KVM
```

應該看到 `STATE : 4 RUNNING`。

## 4. 看日誌

```powershell
Get-Content C:\ProgramData\LAN-KVM\logs\service.log -Tail 30
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 30
```

預期 `service.log`：

```
service started
started session agent pid=... session=1
```

預期 `session.log`：

```
session agent started pid=...
pid=... session=1 console_session=1 elevated=true integrity=System desktop=Default ...
input desktop -> Default
```

## 4.5 啟用 P0 UDP client（選用）

在 B 建立設定檔：

```powershell
New-Item -ItemType Directory -Force C:\ProgramData\LAN-KVM | Out-Null
Set-Content C:\ProgramData\LAN-KVM\config.txt "client_port=45871"
```

重啟服務讓 session agent 帶上新參數：

```powershell
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe start
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 20
```

預期：

```
P0 UDP client listening on 0.0.0.0:45871 (unencrypted; test only)
```

### 與 A 的 devtools master 對測

先確定 B 上**沒有**舊的 devtools client 佔用 45871。A 執行：

```powershell
.\target\dev-fast\lankvm-devtools.exe poc --role master --peer <B_IP> --port 45871 --edge right --seconds 180
```

A 應該先看到 `[master] client online`。在 B 鎖定/登入畫面時，等 `session.log` 出現：

```
Winlogon: injector worker installed
Winlogon: injector active
```

再把 A 的滑鼠推過邊界。B 的 Winlogon 游標應該被注入移動；鍵盤應該能輸入到登入畫面。

> client 模式刻意不安裝 low-level hook；`SendInput` 由另一個未掛 hook 的 injector thread 執行。這是實測後修正的：hook thread 直接 `SendInput` 會失敗。

> 這是目前最高風險的測試：服務以 SYSTEM 身分監聽未加密 UDP。只在隔離測試機/VM 使用，測試完執行 `uninstall`。

## 5. 測試桌面切換

### 5.0 確認 hook worker

`session.log` 應該有：

```
Default: hook worker installed (thread=...)
Default: hooks active
Default: input counts keys=... mouse_moves=... injected=...
```

這代表目前桌面已成功安裝 hook，且每 5 秒會回報輸入計數。

### 5.1 鎖定畫面

在 B 按 `Win+L`，然後看 `session.log`：

```
input desktop -> Winlogon
Winlogon: hook worker installed (thread=...)
Winlogon: hooks active
```

在鎖定畫面移動滑鼠/按鍵，應該出現：

```
Winlogon: input counts keys=... mouse_moves=... injected=...
```

解鎖後應該回到：

```
input desktop -> Default
Default: hook worker installed ...
```

### 5.2 UAC

觸發 UAC（例如執行一個需要管理員的程式），看 `session.log` 是否短暫出現 `Winlogon`，關閉 UAC 後回到 `Default`。

### 5.3 登入畫面

重新開機或登出，停在登入畫面時：

- `session.log` 應該顯示 `input desktop -> Winlogon`
- 登入後應該回到 `Default`

> 如果 log 顯示 `input desktop unavailable: ... Access is denied`，代表 session agent 沒有取得安全桌面存取權；需要檢查 token / UIAccess / 簽章。

## 6. 停止與移除

```powershell
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe uninstall
```

確認：

```powershell
sc.exe query LAN-KVM
```

應該看到服務不存在。

## 7. 失敗排查

| 現象 | 可能原因 | 下一步 |
|------|----------|--------|
| `install` access denied | 不是管理員 | 用管理員 PowerShell |
| `service.log` 沒有 `started session agent` | winlogon token / 權限 / CreateProcessAsUser 失敗 | 看 service.log 的錯誤；確認 winlogon.exe 在 active session |
| `winlogon.exe not found` | 服務在 winlogon 出現前啟動 | 等 5 秒；supervisor 會重試 |
| `session.log` 一直只有 Default | session agent 不在 active session，或 OpenInputDesktop 失敗 | 確認 `session=1 console_session=1`；看錯誤 |
| 鎖定後沒有 `Winlogon` | session agent 沒有安全桌面存取權 | 需要 UIAccess/簽章；下一步處理 |
| 有 `Winlogon` 但沒有 `injector active` | `OpenInputDesktop` / injector thread 建立失敗 | 看 log 的錯誤；可能需要 UIAccess/簽章 |
| 有 `hooks active` 但 counts 一直是 0 | hook 沒收到輸入，或桌面已切換 | 確認在該桌面實際移動滑鼠/按鍵；看 desktop 名稱 |
| `session.log` 沒有 `P0 UDP client listening` | 沒有 `config.txt` 或 port 被佔用 | 確認 `client_port=45871`；停止舊 devtools client；重啟服務 |
| A 有 `client online` 但 B 游標不動 | 封包到了，但 injector 還沒 active 或注入失敗 | 等 `Winlogon: injector active` 再推；看 `injected` / `inject_fail` |
| B 本機送包可動、A 送包不動 | A→B UDP 被端點防護/網路政策擋 | 不要再改防火牆；請 IT 放行 UDP 45871 或改用隔離 VM |
| Winlogon 游標會動但鍵盤無效 | `SendInput` scancode 或 UIPI/安全桌面限制 | 回報 `injected` / `inject_fail` 與 A/B log；下一步檢查注入權限 |
| 服務被防毒刪除 | Apex One 行為判定 | 用 dev-fast + 單檔白名單/隔離 VM；正式版簽章 |

## 8. 安全與限制

- 服務以 **LocalSystem** 執行，能建立 SYSTEM session agent；只在測試機安裝。
- 目前**不記錄按鍵內容**，只記錄桌面名稱。
- Winlogon 桌面的 hook 已實作；P0 UDP client 也已在目前桌面注入，待實機驗證登入畫面。`SendInput` 在安全桌面仍可能受 UIPI/UIAccess 限制，需實測。
- 若 `TokenUIAccess` 設定失敗，先看 SYSTEM 是否能 `OpenInputDesktop`；若不行，需要簽章程式或 UIAccess 設定。
- 不要用「關防毒」解決 service 被擋；請走簽章 + 白名單/隔離 VM。

## 9. 下一步

1. 實機測試鎖定/UAC/登入畫面的遠端滑鼠/鍵盤（P0 UDP client 已接上）。
2. 加入 release-all / 卡鍵保護、control lease、local takeover。
3. 把 P0 UDP 換成 QUIC/TLS + mDNS/配對；只在使用者主動開啟分享時才監聽/轉發。
4. 多螢幕/DPI、剪貼簿、檔案、Tauri UI。
5. 正式版：簽章、白名單、UIAccess 簽章程式、AV 測試。
