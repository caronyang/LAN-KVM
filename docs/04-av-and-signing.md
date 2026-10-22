# 防毒／EDR 與程式簽章策略

> 這份文件回答一個必須先講清楚的問題：**「不能被防毒軟體阻擋」不是靠寫程式技巧保證的，而是靠「產品行為 + 簽章 + 廠商白名單 + 企業政策」共同達成。**  
> 任何會全域攔截鍵盤/滑鼠、注入輸入、常駐、開網路埠的軟體，在未簽章、未白名單的情況下，被 AV/EDR 攔截是正常結果。

---

## 1. 目前遇到的狀況

本機環境：

- Windows Defender 即時防護關閉（`AMServiceEnabled = False`）。
- 安裝了 **Trend Micro Apex One / OfficeScan**（企業端點防護）。
- `target\release\lankvm-agent.exe` 在執行/建置後消失，`target\release\deps\lankvm_agent.exe` 仍在，且**未簽章、沒有任何版本資訊**。

因此最可能的原因是 Apex One 的行為/信譽判定，而不是 Windows Defender。要確認必須取得：

1. 防毒軟體的**偵測名稱**（例如 `TROJ_GEN...`、`Behavior:...`、`HackTool...`）。
2. 被攔截的**完整檔案路徑**與 **SHA-256**。
3. 動作（已封鎖/已隔離/已刪除）。
4. 是「執行時」還是「掃描時」被擋。

> 目前的 P0 二進位沒有簽章、沒有版本資源，還包含 `hook-test` 的按鍵記錄與未加密 UDP；被歸類為 keylogger/hacktool 的機率很高。這不是產品該有的樣子。

### 實測（2026-10-21）

- `lankvm-agent.exe probe` 可正常執行（產品骨架，無 hook/keylog/UDP）。
- `lankvm-devtools.exe` 被 Apex One **立即刪除**：從 `target\release\deps` 複製回 `target\release` 後，執行 `help` 時檔案已消失。這不是建置錯誤，是端點防護的即時掃描/隔離。
- v0.1-local 測試包雜湊請見 [`SHA256SUMS.txt`](../SHA256SUMS.txt)。
- 目前 service（debug）：`E2815155B40B7AC7B83E08DB214ACAD20C0B0D38BC7D866AF2A6CEC76C552A18`
- 目前 devtools（debug）：`D50CB2D920E0F0FB665B4D3BDDAE1CAB1787A547B1CF4B0F8AA6E794D507E3F5`
- 目前 agent（debug）：`379AC4E60E2A4B0CC6F9835CBC092C10110555375018296546888ED8BC1F7302`

**處置：** 不要把 devtools 放到正式機器；請 IT 對測試機做單檔 hash 白名單，或用沒有 Apex One 的隔離 VM。正式產品仍需簽章 + 白名單。

---

## 2. 已經做的結構調整

### 2.1 開發工具與產品分離

| 二進位 | 內容 | 是否散布 |
|--------|------|----------|
| `lankvm-agent.exe` | 產品骨架：`probe`、`version`；後續 tray/service/UI | **是** |
| `lankvm-devtools.exe` | `hook-test`（會記錄按鍵）、`inject-test`、`poc`（未加密 UDP） | **否**，只給開發/測試機 |

這個拆分的重點不是「躲防毒」，而是**不要讓產品包含鍵盤記錄、除錯與未加密網路程式碼**。正式產品仍會有輸入 hook/injection，因為那是核心功能；但它們必須以可簽章、可稽核、使用者主動啟用的方式存在。

### 2.2 尚未完成、但出貨前必須做

- [x] 加上 Windows 版本資源（ProductName、CompanyName、FileDescription、OriginalFilename、Icon、版權）與 manifest（asInvoker、PerMonitorV2、longPathAware）。
- [x] 開發工具與產品分離：`lankvm-devtools.exe` 不散布，`lankvm-agent.exe` 不含 hook/inject/poc。
- [ ] 簽署 `lankvm-agent.exe`、service、installer、uninstaller、更新檔。
- [ ] 建立可重現建置與 SHA-256 清單。
- [ ] 把 hook/injection 改成「使用者按下開始分享後才安裝」，不要一開機就掛 hook。
- [ ] 正式網路層改 QUIC + TLS + 配對；拿掉未加密 UDP。
- [ ] 正式版不記錄按鍵內容、不記錄剪貼簿內容；只記 metadata（類型/大小/錯誤碼）。
- [ ] 服務預設不安裝；只有使用者要「登入畫面控制」時才安裝，且 UI 明確說明權限。
- [ ] 隱私政策、EULA、資安說明、雜湊與原始碼連結。

---

## 3. 降低誤判的產品設計原則

### 3.1 行為要「像一個正常工具」

- **可見的托盤圖示**、明確的開/關、連線狀態；不要隱藏視窗、不要靜默執行。
- **不要在開機時就安裝全域 hook**。先啟動托盤，等使用者按「開始分享」才安裝 hook；停止分享就卸載。
- **不要記錄鍵盤輸入內容**。正式版只轉發，不寫入 log；`hook-test` 留在 devtools。
- **不要做 process injection / DLL injection / shellcode / 記憶體 patch**。我們只需要 `SendInput` 與 low-level hook；這兩者雖然敏感，但比 injection 正常得多。
- **不要加殼/混淆/UPX**。這只會提高防毒評分。
- **不要用隱藏的自啟動**。用標準 Startup/Task Scheduler，UI 可關閉，卸載可移除。
- **不要背景掃描使用者輸入或檔案**。只在連線時處理。

### 3.2 網路行為要可驗證

- 正式版**先配對，再開服務**；未配對時不接受輸入封包。
- 使用 **QUIC + TLS 1.3**，憑證 pinning；不要用未加密 UDP。
- 只綁定需要的介面/埠；連線結束就關閉；不要長期監聽 0.0.0.0。
- 檔案傳輸要由使用者動作觸發，顯示進度與來源/目的地；不要背景抓檔案。
- 剪貼簿同步要可關閉、可暫停，尊重密碼管理器的排除格式。

### 3.3 權限要最小化

| 模式 | 權限 | 使用情境 | 備註 |
|------|------|----------|------|
| 標準模式 | 一般使用者 | 一般桌面分享 | 預設；不能控制管理員視窗 |
| 登入畫面模式 | Windows Service（LocalSystem）+ SYSTEM session agent | 登入/UAC/鎖定畫面 | 選項；安裝時 UAC 一次，UI 說清楚 |
| 驅動模式 | 核心驅動 | 遊戲/反作弊/BIOS 後 | **不建議**；簽章與相容成本極高 |

「登入畫面」是使用者明確要求，所以服務是必要之惡；但**不要讓網路層跑在 SYSTEM agent 裡**。正式架構建議：

```
使用者權限：UI + 網路/配對/剪貼簿/檔案
SYSTEM：只做輸入捕捉/注入 + 桌面切換（透過本機 IPC）
```

這樣即使網路層有漏洞，也比較難直接變成 SYSTEM RCE。這也是 AV/EDR 比較能接受的設計。

---

## 4. 簽章：從「可疑」變成「可識別」

### 4.1 憑證選項

| 方案 | 成本 | 優點 | 注意 |
|------|------|------|------|
| **Azure Trusted Signing** | 低（月費） | 微軟託管、CI 可簽、適合開源/小型團隊 | 需身分驗證與資格審核 |
| **SignPath.io（OSS 免費）** | 免費 for OSS | 專為開源專案、CI 整合 | 需公開原始碼與審核 |
| OV Code Signing | 中 | 一般商業軟體可用 | SmartScreen 信譽要累積 |
| EV Code Signing | 高 | SmartScreen 信譽較快、企業信任度高 | 硬體 token/HSM、流程較重 |
| 自簽憑證 | 免費 | 內部測試 | **對外沒有幫助**，甚至更可疑 |

**建議：**

- 開源 Release：先申請 **SignPath.io OSS** 或 **Azure Trusted Signing**。
- 商業/企業散布：OV 或 EV；EV 對 SmartScreen 與 IT 白名單較有利。
- 不論哪種，** installer、agent、service、uninstaller、更新檔都要簽**，並加時間戳。

### 4.2 簽章檢查

```powershell
Get-AuthenticodeSignature .\lankvm-agent.exe | Format-List
Get-FileHash .\lankvm-agent.exe -Algorithm SHA256
```

預期：

```
Status : Valid
SignerCertificate : CN=...
```

若 `Status` 是 `NotSigned` 或 `HashMismatch`，不要發布。

### 4.3 SmartScreen / 信譽

- 新憑證 / 新檔案第一次下載仍可能出現 SmartScreen 警告；EV 或累積下載量會改善。
- 提供官方下載頁、SHA-256、簽章者名稱，讓使用者可驗證。
- 不要用「先下載 ZIP 再解壓縮」規避 Mark-of-the-Web；這反而增加 suspicion。

---

## 5. 防毒廠商白名單 / 誤判提交流程

### 5.1 需要準備的資料

- 產品名稱、版本、官方下載 URL、原始碼 URL。
- 檔案 SHA-256、簽章者、檔案大小。
- 偵測名稱、AV 產品/版本、作業系統版本。
- 行為說明（不是鍵盤記錄器；使用者主動啟用；資料不離開 LAN；不寫入按鍵內容）。
- 安裝/卸載步驟、隱私政策。

### 5.2 Microsoft

- Microsoft Defender / SmartScreen 誤判：<https://www.microsoft.com/en-us/wdsi/filesubmission>
- 選擇「Software developer」提交，附上簽章與說明。
- 若是 SmartScreen 信譽問題，簽章 + 下載量 + 時間會改善。

### 5.3 Trend Micro Apex One / OfficeScan

企業環境通常由 IT 集中管理，**只有 IT 能加白名單**。可請 IT 使用：

- Apex One 的 **Trusted Program List / Approved List**（依 publisher 或 SHA-256）。
- 若是行為監控誤判，提交樣本給 Trend Micro（Support Portal / Threat Defense）。
- **不要**要求 IT 關閉即時防護或整台機器排除；用「單一程式 + 雜湊」白名單。

### 5.4 其他 EDR

CrowdStrike、SentinelOne、Kaspersky、Sophos 等都需要類似的廠商/IT 流程。請預期：

- 未簽章 → 大概率被擋。
- 已簽章但新產品 → 可能被行為規則擋，需要廠商調規則。
- 企業政策禁止輸入注入 → 任何軟體 KVM 都過不了，只能改用硬體 KVM。

---

## 6. 測試與驗收

### 6.1 乾淨 VM 測試矩陣

| 環境 | 測試 |
|------|------|
| Windows 11 + Defender 即時防護 | 下載、掃描、安裝、執行、卸載 |
| Windows 11 + Apex One | 同上；取得偵測名稱/動作 |
| Windows 10 22H2 + Defender | 同上 |
| 企業 GPO/AppLocker | 未簽章/已簽章安裝行為 |

### 6.2 每次 Release 的檢查清單

- [ ] 所有 PE 檔案已簽章且時間戳有效。
- [ ] 版本資源正確（ProductName/CompanyName/Version）。
- [ ] 不含 `hook-test` / `inject-test` / `poc` / 除錯符號（目前已分離到 devtools）。
- [ ] 不含未加密 UDP、硬編碼 IP、測試憑證。
- [ ] 安裝/卸載會正確新增/移除防火牆規則、服務、自啟動。
- [ ] VirusTotal 掃描（注意會公開樣本；商業版可先用廠商通道）。
- [ ] Defender/Apex One 乾淨 VM 掃描與執行測試。
- [ ] 公布 SHA-256 與 SBOM。

---

## 7. 目前的實務建議

1. **不要**在正式機器上關閉防毒來跑 P0。
2. 找一台**專用測試機/VM**，只把 `D:\99_Temp\opencode\LAN-KVM\target` 或 `lankvm-devtools.exe` 的雜湊加入排除；測試完移除排除。
3. 若 Apex One 是公司管理，請 IT 協助：
   - 提供 `lankvm-devtools.exe` 的 SHA-256 與用途說明，申請測試白名單。
   - 或使用隔離的測試機/VM，不接公司正式網段。
4. 正式產品照第 3～6 節做；**先簽章，再談白名單**。
5. 若使用者/IT 仍不接受輸入注入行為，**硬體 KVM 是唯一能保證不被 AV 干預的方案**。

---

## 8. 需要你提供的資訊

請從 Apex One / Security Agent 的日誌或 IT 主控台取得：

- 偵測名稱（Threat Name）
- 檔案路徑與 SHA-256
- 動作（block/quarantine/delete）
- 是執行時還是掃描時
- Apex One 版本 / pattern 版本

有了這些，我們才能判斷是「信譽不足」還是「行為規則命中」，並對症下藥；在那之前，任何「保證不被擋」的說法都不可信。
