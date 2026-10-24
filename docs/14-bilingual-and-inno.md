# 中英文介面與 Inno Setup 安裝程式（v0.4）

> 日期：2026-10-24
> 版本：`0.4.0`
> 產物：`dist-app/LAN-KVM-Setup-0.4-inno.exe`
> SHA-256：`5143E1DA662B0D2041AF4F932BD5765C7CA3514A8BE579C9BF896B8FEAF8ECBA`

這一階段把交付方式收尾：介面中英文雙語，並改用 **Inno Setup** 產生的標準安裝程式
（取代原本自製的 self-extract setup）。

---

## 1. 中英文介面

### 1.1 瀏覽器 UI（`crates/lankvm-app/src/ui.html`）

- 內建 `zh`（繁體中文）與 `en` 兩份字典，所有靜態文字用 `data-i18n` 標記，
  動態文字（狀態、角色、提示、toast、配對 modal）統一走 `t("key")`。
- 語言選擇：
  1. 若 `localStorage["lankvm.lang"]` 有值，直接使用（手動切換會記住）。
  2. 否則看 `navigator.language`：`zh*` → 中文，其他 → English。
- 右上角 `<select id="lang">` 可隨時切換；`<html lang>` 會同步更新。
- 重新整理後仍維持同一語言（實測 `reload` 後 `lang=en` 保留）。

新增字串只要同時加到 `I18N.zh` 與 `I18N.en`，就不會出現只翻一半的畫面。

### 1.2 系統匣選單（`crates/lankvm-app/src/tray.rs`）

- 托盤提示與右鍵選單跟著 **Windows 顯示語言**（`GetUserDefaultUILanguage`，
  `0x04xx` 視為中文），不跟瀏覽器設定。
- 例：`LAN-KVM - 主控中 (192.168.68.69)` / `LAN-KVM - Controlling (192.168.68.69)`。

### 1.3 安裝程式

Inno Setup 精靈本身也是雙語（English + 繁體中文），見下一節。

---

## 2. Inno Setup 安裝程式

### 2.1 為什麼換掉自製 setup

自製 setup 是一支會自我解壓並寫檔的 Rust 程式，缺點是：

- 沒有「新增/移除程式」標準卸載流程（只能手動刪檔、刪服務）。
- 升級/修復/版本資訊/安裝記錄都要自己維護。
- 無法使用 Inno 的語言精靈、元件頁、`/SILENT`、`/LOG`、數位簽章流程。

Inno Setup 版本與自製 setup 做**完全相同**的事，另外多了標準卸載與升級。

### 2.2 檔案

```
installer/
  lankvm.iss                 # 安裝腳本
  ChineseTraditional.isl     # 繁體中文訊息（其餘未翻譯的罕用對話框會回退英文）
  config.txt                 # C:\ProgramData\LAN-KVM\config.txt（client_port=45872）
  build.cmd                  # 呼叫 ISCC 編譯
```

建置：

```powershell
cargo build --release -p lankvm-app -p lankvm-service
installer\build.cmd
# 產出 dist-app\LAN-KVM-Setup-0.4-inno.exe
```

`build.cmd` 會依序找：

1. `%ISCC%`（若有設定）
2. `%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe`（per-user 安裝）
3. `%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe`
4. `%ProgramFiles%\Inno Setup 6\ISCC.exe`

### 2.3 安裝流程（與自製 setup 對齊）

`PrepareToInstall`（檔案複製前）：

1. `taskkill /F /IM lankvm-app.exe /T`
2. `net stop LAN-KVM`（同步等待服務停止，避免 binary 被鎖）
3. `taskkill /F /IM lankvm-service.exe /T`（清掉殘留 session agent）

檔案：

- `{app}\lankvm-app.exe`、`{app}\lankvm-service.exe`
- `{commonappdata}\LAN-KVM\config.txt`（`onlyifdoesntexist`，升級不覆蓋使用者設定）

`ssPostInstall`（檔案複製後）：

1. `lankvm-service.exe install` → 建立或更新服務、AutoStart、啟動
2. 防火牆（先刪再建，避免重複規則）
   - UDP 45870 Discovery / 45871 Control / 45872 Data
   - TCP 45873 Logs / 45875 Clipboard
3. 排程
   - `LAN-KVM Boot Start`（ONSTART，SYSTEM 執行 `sc start LAN-KVM`）
   - `LAN-KVM Ensure Service`（每 5 分鐘，同樣動作）
4. `sc failure LAN-KVM reset= 86400 actions= restart/5000/restart/15000/restart/60000`
5. 安裝記錄（`Log()` 與 `%TEMP%\lankvm-setup.log`），後者會被「收集紀錄」打包成 `setup.log`

其他：

- 桌面捷徑（可選工作）、開始功能表捷徑、解除安裝捷徑
- `HKCU\...\Run\LAN-KVM = "<app>" --silent`（登入後常駐）
- 安裝完成可勾選「啟動 LAN-KVM」（`runasoriginaluser`，避免托盤程式以管理員權限執行）

### 2.4 解除安裝

`[UninstallRun]` 依序：

1. `taskkill /F /IM lankvm-app.exe /T`
2. `lankvm-service.exe uninstall`
3. 刪除兩個排程
4. 刪除五條防火牆規則
5. 移除 `HKCU\...\Run` 值（registry `uninsdeletevalue`）

保留：

- `%LOCALAPPDATA%\LAN-KVM\app.json`（配對與設定）
- `C:\ProgramData\LAN-KVM\config.txt` 與 `logs\`（診斷用）

### 2.5 已知注意事項

- **未簽章**：SmartScreen／AV 仍可能提示；正式版要簽章。
- **HKCU 與管理員安裝**：Inno 會警告「以 admin 安裝卻使用 per-user 區域」。
  正常情況（同一個使用者按 UAC）寫入的是該使用者的機碼；若是 IT 用其他管理員帳號
  替別人安裝，登入自動啟動要改成 `{commonstartup}` 捷徑或由 IT 另外部署。
- **升級**：直接執行新版安裝程式即可；服務會被停止、更新、重新啟動。
- **`/SILENT`**：不顯示精靈，仍會建立服務、防火牆、排程，並以原使用者啟動托盤程式。

### 2.6 驗收清單

1. 執行 `LAN-KVM-Setup-0.4-inno.exe`（UAC 按是），選「繁體中文」。
2. 安裝完成後：
   ```powershell
   sc query LAN-KVM
   schtasks /Query /TN "LAN-KVM Boot Start" /V /FO LIST
   schtasks /Query /TN "LAN-KVM Ensure Service" /V /FO LIST
   Get-NetFirewallRule -DisplayName "LAN-KVM*" | Select DisplayName,Enabled,Direction
   ```
3. 托盤圖示出現，UI 顯示 `0.4.0`；右上角切換 中文/English。
4. 重開機：服務與托盤程式自動起來，配對連線恢復。
5. 「新增或移除程式」可看到 LAN-KVM；解除安裝後服務、排程、防火牆規則都消失。

---

## 3. 這一版的其他變更

- workspace 版本 `0.1.0` → `0.4.0`（UI 顯示與 exe 版本資源一致）。
- `lankvm-devtools clipboard-files <path>`：診斷用，直接以 app 的 `write_files`
  設定 CF_HDROP，可驗證剪貼簿格式是否被清空。
