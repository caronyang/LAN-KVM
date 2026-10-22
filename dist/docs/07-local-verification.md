# v0.1-local：單機可驗證版本

> 日期：2026-10-21  
> 目的：把所有「不需要第二台電腦」或「可以用本機 loopback 模擬」的功能做完並驗證，讓明天回來可以直接測跨機/登入畫面。  
> 注意：這不是完整產品；跨機 UDP 目前被 Apex One/EDR 或 VM 網路阻擋。

## 1. 這個版本包含什麼

| 功能 | 狀態 | 驗證方式 |
|------|------|----------|
| `probe`：session/桌面/DPI/螢幕 | PASS | 實機輸出 |
| 相對游標注入 | PASS | `inject-test` |
| 絕對游標注入 | PASS | `abs-test`（delta 0 px） |
| 剪貼簿文字 round-trip | PASS | `clipboard-test` |
| 剪貼簿圖片 PNG round-trip | PASS | `clipboard-test`（16×16） |
| 剪貼簿檔案 CF_HDROP round-trip | PASS | `clipboard-test --files` |
| 剪貼簿變更監聽 | PASS | `clipboard-watch` |
| 剪貼簿 wire format（postcard） | PASS | text/image/files encode→decode |
| 本機檔案分塊複製 + BLAKE3 | PASS | `file-test`（3 MB） |
| P0 封包本機注入（滑鼠/鍵盤） | PASS | `local-winlogon-test`（Caps Lock LED 有切換） |
| Windows Service：LocalSystem + SYSTEM session agent | PASS | `install`、`session.log` |
| `Default` ↔ `Winlogon` 桌面切換 | PASS | 鎖定/UAC log |
| Winlogon hook | PASS | `Winlogon: hooks active` + counts |
| Winlogon `SendInput` | PASS | 本機封包讓登入畫面游標移動 |
| P0 UDP client（service） | 程式完成；跨機被擋 | `127.0.0.1` 可收，A→B 不行 |

## 2. 尚未包含

- 跨機 UDP 打通（Apex One/EDR/VM 網路政策）
- QUIC/TLS、mDNS 配對、群組信任
- 剪貼簿/檔案跨機同步
- 拖放
- Tauri UI / 安裝檔 / 自動更新
- 多螢幕/混合 DPI 實機測試
- macOS/Linux

## 3. 建置

Apex One 會刪除 release/dev-fast service；本版本使用 **debug**：

```powershell
cd D:\99_Temp\opencode\LAN-KVM
cargo build -p lankvm-devtools
cargo build -p lankvm-service
cargo build --release -p lankvm-agent
```

產出：

```
target\debug\lankvm-devtools.exe
target\debug\lankvm-service.exe
target\release\lankvm-agent.exe
```

## 4. 單機測試指令

### 4.1 系統資訊

```powershell
.\target\release\lankvm-agent.exe probe
```

### 4.2 絕對游標

```powershell
.\target\debug\lankvm-devtools.exe abs-test
```

預期：

```
target=(960, 540) got=(960, 540) delta=(0, 0)
...
absolute cursor test PASS
```

> 這個測試會被**實體滑鼠移動**或同時執行的 service 注入干擾；請在滑鼠靜止、service 停止時跑。程式已內建 3 次重試，但無法對抗持續的外部輸入。

### 4.3 剪貼簿

```powershell
# 文字 + 圖片
.\target\debug\lankvm-devtools.exe clipboard-test

# 加上 CF_HDROP 檔案
$tmp = Join-Path $env:TEMP 'lankvm-clip-test.txt'
Set-Content $tmp 'LAN-KVM file clipboard test'
.\target\debug\lankvm-devtools.exe clipboard-test --files $tmp
```

### 4.4 剪貼簿監聽

```powershell
# 執行後，手動複製一段文字/圖片/檔案
.\target\debug\lankvm-devtools.exe clipboard-watch --seconds 15
```

預期：

```
[clipboard] text (28 chars)
[clipboard] image 1920x1080 (... bytes PNG)
[clipboard] files (2)
```

### 4.5 本機檔案複製

```powershell
$src = Join-Path $env:TEMP 'lankvm-file-test.bin'
$dst = Join-Path $env:TEMP 'lankvm-file-out'
New-Item -ItemType Directory -Force $dst | Out-Null
$bytes = New-Object byte[] (3MB + 17)
[System.IO.File]::WriteAllBytes($src, $bytes)
.\target\debug\lankvm-devtools.exe file-test --source $src --dest $dst --overwrite
```

預期 `PASS` 與 BLAKE3 hash。

### 4.6 本機 P0 封包 → service 注入

先安裝/啟動 service（管理員）：

```powershell
.\target\debug\lankvm-service.exe install
```

建立 client 設定並重啟 service：

```powershell
New-Item -ItemType Directory -Force C:\ProgramData\LAN-KVM | Out-Null
Set-Content C:\ProgramData\LAN-KVM\config.txt "client_port=45871"
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe start
```

確認 `session.log`：

```powershell
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 20
# P0 UDP client listening on 0.0.0.0:45871
# Default: injector active
```

在 Default 桌面測試（會移動游標、切換 Caps Lock）：

```powershell
.\target\debug\lankvm-devtools.exe local-winlogon-test --delay 2 --motion 150
```

### 4.7 Winlogon 本機測試

```powershell
.\target\debug\lankvm-devtools.exe local-winlogon-test --delay 15 --motion 300
```

1. 執行後 15 秒內按 `Win+L`。
2. 登入畫面游標應往右移動；Caps Lock LED 應切換。
3. 看 `session.log`：

```
Winlogon: injector active
Winlogon: input counts ... injected=... inject_fail=0
```

> 如果 Caps Lock 最後是開著的，請再按一次 Caps Lock 或執行 `WScript.Shell.SendKeys("{CAPSLOCK}")` 還原。

## 5. 封裝好的測試包

`dist/` 與 `LAN-KVM-v0.1-local.zip` 已包含：

```
lankvm-agent.exe
lankvm-devtools.exe
lankvm-service.exe
config.txt
install-service.ps1
uninstall-service.ps1
run-local-tests.ps1
docs/07-local-verification.md
README.txt
```

`run-local-tests.ps1` 會跑 probe、clipboard（text/image/file）、本機檔案複製；`abs-test` 因容易被實體滑鼠干擾，改成手動指令。

> ZIP 的 SHA-256 會隨封裝內容變動；請以 release 目錄外層的 `SHA256SUMS.txt` 或最後一次 build 輸出為準。

## 6. 服務安裝/移除

```powershell
# 管理員
.\target\debug\lankvm-service.exe install
sc.exe query LAN-KVM
.\target\debug\lankvm-service.exe stop
.\target\debug\lankvm-service.exe uninstall
```

## 7. 已知問題

1. **A→B UDP 不通**：A 的 Windows 防火牆關閉、B 的 netsh 規則存在，但封包沒到 B。手動 PowerShell Hello 也失敗，故為 Apex One/EDR 或 VM 網路政策。需要 IT 放行 UDP 45871，或隔離 VM。
2. **release/dev-fast service 被 Apex One 刪除**：目前只能用 debug。
3. **P0 未加密/未配對**：只可在隔離測試機使用。
4. **相對注入**：service client 仍用 `Motion dx/dy`；絕對定位已在 `abs-test` 驗證，但尚未接上 P0 協定。
5. **UI/剪貼簿/檔案跨機同步**：尚未接上。

## 8. 明天回來後的測試順序

1. 跑 `docs/07` 第 4 節所有本機測試。
2. 用管理員 PowerShell 在 B 安裝 debug service + `client_port=45871`。
3. 鎖定 B，跑 `local-winlogon-test`，確認滑鼠/Caps Lock。
4. 若 A→B 網路已由 IT 放行：A 跑 devtools master，B 鎖定，等 `Winlogon: injector active` 後推邊界。
5. 若仍未放行：改用兩台乾淨 VM（無 Apex One）測試 A→B。

## 9. VM 測試建議

1. 兩台 VM 使用 **Bridged** 或同一個 **Internal Switch**，確認互相 ping 得到。
2. 兩台都解壓 `LAN-KVM-v0.1-local.zip`。
3. B 的管理員 PowerShell：

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\install-service.ps1 -ClientPort 45871
Get-Content C:\ProgramData\LAN-KVM\logs\session.log -Tail 20
```

4. B 的防火牆（若 VM 有開）：

```powershell
New-NetFirewallRule -DisplayName "LAN-KVM P0 UDP 45871" `
  -Direction Inbound -Protocol UDP -LocalPort 45871 -Action Allow -Profile Any
```

5. A：

```powershell
.\lankvm-devtools.exe poc --role master --peer <B_IP> --port 45871 --edge right --seconds 180
```

A 必須先看到 `[master] client online`。  
6. B 按 `Win+L`，等 `session.log`：

```
input desktop -> Winlogon
Winlogon: injector worker installed
Winlogon: injector active
```

7. 推 A 的滑鼠過右邊界。B 的登入畫面游標應移動；Caps Lock 應可切換。
8. 看 B log 的 `injected` / `inject_fail`：

- `injected` 增加、`inject_fail=0`：成功。
- `inject_fail>0`：Winlogon `SendInput` 被安全政策擋，需要 UIAccess/簽章。

9. 測完：

```powershell
.\uninstall-service.ps1
```
