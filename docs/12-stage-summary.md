# v0.2 階段成果：兩台實機驗收完成

> 日期：2026-10-23（測試）／2026-10-24（整理）  
> 版本：`v0.2-app` + `-login3` 測試包  
> 狀態：**兩台實機驗收完成**（登入頁、UAC 機制、輸入、開機自動連線、log 收集）

---

## 1. 測試環境

| 角色 | 主機 | IP | 設定 |
|------|------|----|------|
| A（master，有實體鍵鼠） | `LAPTOP-0MC99DAQ` | `192.168.68.68` | 已登入，App 常駐系統匣 |
| B（client） | `DESKTOP-MJ6411M` | `192.168.68.69` | 安裝 `-login3`，可停在登入頁 |

- Windows 11 x64、同一個 Wi-Fi／網段
- 傳輸：P0 UDP 45870（探索）／45871（控制）／45872（輸入）／45873（log）
- A 的 master RTT：**4–13 ms**（Wi-Fi）

---

## 2. 驗收結果

| 項目 | 結果 | 備註 |
|------|------|------|
| 區網探索、3 位數配對 | PASS | B 顯示配對碼、A 輸入 |
| 配對後自動連線 | PASS | 開機／App 重啟後自動連 |
| A → B 滑鼠、鍵盤 | PASS | 邊界切換、Raw Input、ClipCursor 正常 |
| B → A 回切（游標推到進入邊界） | PASS | 邊界 2px 內立即回切，不再需要 30px |
| 滑鼠滾輪、左右中鍵 | PASS |  |
| 複製貼上 | PASS | 目前是系統剪貼簿層級；跨機同步尚未接傳輸層 |
| Num Lock／數字鍵盤 | PASS | 改用 virtual key 注入 toggle key |
| B 鎖定畫面 | PASS | `input desktop -> Winlogon`、`winjector active` |
| **登入頁點密碼欄位、輸入密碼** | **PASS** | 由 Winlogon helper process 注入；`inject_fail=0` |
| UAC（同一 secure desktop 機制） | PASS（機制相同） | 需在 B 跳 UAC 時由 A 操作 |
| B 先開機、停在登入頁，A 操作 B 登入 | **PASS** | `peers=1`、B online、RTT 4–13 ms |
| B 換 IP／ID 廣播 | PASS | 服務以持久化 id 廣播 + unicast 到 A |
| 一鍵收集兩台 log | PASS | 修正 10035 後，A 會收到 `peer-<B>/` |
| 無 console／系統匣常駐 | PASS | session agent 使用 `CREATE_NO_WINDOW` |
| 開機服務自動啟動 | PASS（B，-login3） | AutoStart + ONSTART + ensure + health probe |
| 無限次來回切換 | PASS | 修正 EdgeDetector 未 reset |

---

## 3. 這階段修掉的關鍵問題

1. **只能切一輪**：`do_release` 沒有重設 `EdgeDetector.fired`。
2. **回切要推 30px**：改成游標實際到進入邊界、越界 2px 即 Release；並用實際游標位置避免誤判。
3. **鎖定畫面點密碼欄位後卡死**：
   - 一開始以為是 thread attach；log 顯示 `attach=true` 但 `SendInput err=5`。
   - 最終改用 **獨立 `winjector` process**（`lpDesktop = winsta0\Winlogon`）注入。
4. **黑窗**：session agent 改成 `CREATE_NO_WINDOW`；setup 會清舊 agent。
5. **Num Lock／Caps／Scroll**：scan code 不會更新 toggle state，改用 `keybd_event` virtual key。
6. **開機服務沒起來、重裝才好的**：
   - 安裝改成 idempotent（AutoStart、更新 path、確保啟動）
   - 服務每 5 秒探測 agent 45872，失敗自動重啟
   - ONSTART `LAN-KVM Boot Start` + 每 5 分鐘 `LAN-KVM Ensure Service`
   - 服務失敗自動重啟
7. **B 先開機、App 沒啟動時找不到**：
   - 服務／agent 讀取 App 的持久化 `id`，廣播 + unicast 到配對的 A IP
   - A 用 id 比對、更新 IP，必要時重啟 master
8. **UI 紅 banner 把右欄推歪**：banner 移到跨兩欄的最上方；空 banner 不佔高度。
9. **一鍵 log 10035**：接受的 TCP socket 要切回 blocking。

---

## 4. 最終測試產物

```
dist-app/LAN-KVM-Setup-0.2-login3.exe
SHA-256 8A622E5A9739429E6A77CD59916CB8FF57E7534C9690DEEF13C6C24B59D3EAD3
```

- 單一安裝檔，內嵌 `lankvm-app.exe` 與 `lankvm-service.exe`
- 安裝後：防火牆、AutoStart、ONSTART/ensure 排程、桌面/開始功能表捷徑、系統匣
- 服務 hash（E649 後未變）：`51063357C60DE4090534589091E286AB299921D0A61F6F6D066966A2552B1B15`

> 歷史測試檔 `LAN-KVM-Setup-0.2.exe`、`-login.exe` 保留在 `dist-app/`，以 `-login3` 為準。

---

## 5. 已知限制

- **未簽章**：McAfee／Apex One／Smart App Control 可能隔離；正式版需簽章＋白名單。
- **P0 UDP 未加密、配對碼 3 位數**：只可在信任網段；正式版 QUIC/TLS + SPAKE2。
- **B 的登入頁網路**：Wi-Fi profile 必須是 all-user 或使用 Ethernet；per-user Wi-Fi 在登入頁可能沒連線。
- **只支援 Windows**；macOS/Linux 尚未實作。
- **跨機剪貼簿／檔案同步**尚未接上傳輸層（本機引擎已完成）。
- **絕對定位游標**尚未接進 P0（`abs-test` 已驗證 API）。
- **多螢幕／混合 DPI** 尚未實機驗收。
- **安裝器**仍是自製 self-extract；尚無正式卸載/修復流程（建議 Inno Setup）。
- 系統匣圖示在 Windows 11 可能收在 `^`，需手動拖出。

---

## 6. 下一步建議

1. **正式安裝器**：Inno Setup（或 MSI）＋程式/服務簽章、卸載、升級、修復。
2. **QUIC/TLS + mDNS/SPAKE2 配對**：取代 P0 UDP 與 3 位數碼。
3. **跨機剪貼簿／檔案**：接到 QUIC 串流。
4. **絕對定位游標**：改善手感與多螢幕。
5. **多螢幕版面 UI**：左右/上下、混合 DPI。
6. **24 小時 soak／斷線恢復／多網卡**。
7. macOS/Linux 平台層。

---

## 7. 如何重現這次驗收

1. 兩台安裝 `LAN-KVM-Setup-0.2-login3.exe`。
2. A 開啟 UI → 掃描 → 選 B → 配對（B 顯示 3 位碼）。
3. 確認 B 的 `sc query LAN-KVM` 是 RUNNING、兩個排程存在。
4. B 重開機，停在登入頁（不要登入）。
5. A 等 20–30 秒，看掃描列表出現 B；推邊界即可操作 B 登入頁。
6. 在 A 按「收集紀錄」；應包含 `peer-DESKTOP-MJ6411M/`。
