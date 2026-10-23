# LAN-KVM 架構決策與修訂（v0.2）

> 日期：2026-10-21　狀態：已確認方向，P0 PoC 進行中  
> 本文件補充並修訂 [`01-tech-selection.md`](01-tech-selection.md)。若兩份文件衝突，以本文件為準。  
> 進度：Rust workspace、`probe`、hook/injection 檢查、兩台 UDP 鍵鼠轉發 PoC 已完成；下一步是服務 + Winlogon 桌面切換。防毒/簽章策略見 [`04-av-and-signing.md`](04-av-and-signing.md)。

---

## 1. 已確認的決策

| 項目 | 決策 | 對架構的影響 |
|------|------|--------------|
| 平台 | 架構支援 Windows / macOS / Linux；目前只有 Windows 可實測 | 平台抽象層先做 Windows backend；macOS/Linux 留 trait 與 PoC 計畫 |
| 授權 | 開源 | 可參考 Deskflow 行為；但避免直接複製 GPL-2.0-only 程式碼，除非決定採用 GPL-2.0 |
| 登入畫面 | **必須可操作登入畫面** | 需要特權服務 + 系統層 session agent；一般使用者程式做不到 |
| 多螢幕 | 每台電腦可多螢幕；目前 A/B 各一顆 | 佈局與邊界要用 monitor 級資料，不能只存「左/中/右」 |
| 技術棧 | 以效率、順暢、能達標為主 | 最終建議 Rust + Tauri；以 P0 PoC 驗證登入畫面與延遲 |
| 品牌 | 名稱 LAN-KVM；極簡 Logo | Logo 已定案為 **Seam**（左右雙向箭頭各跨內側一半）；PNG/ICO/ICNS/托盤圖示已輸出 |

---

## 2. 最關鍵的變更：操作登入畫面

### 2.1 為什麼一般程式做不到

Windows 的登入畫面、鎖定畫面、UAC 同意視窗都在 **Winlogon 安全桌面**（secure desktop），不是一般使用者的 `Default` 桌面。一般使用者權限的程式：

- 無法 `OpenInputDesktop` 取得 Winlogon 桌面。
- 無法在該桌面安裝 `WH_*_LL` hook。
- 無法對該桌面 `SendInput`。
- 即使以管理員身分執行，沒有 SYSTEM 權限／UIAccess 仍可能被 UIPI 擋下。

因此「可操作登入畫面」不是加一個選項，而是**架構必須改變**。

### 2.2 三種可行做法

| 做法 | 原理 | 優點 | 缺點 | 建議 |
|------|------|------|------|------|
| A. 一般使用者程式 + SendInput | 在使用者桌面注入 | 簡單、安全 | **登入/UAC 完全做不到** | 不採用 |
| B. Windows 服務 + SYSTEM session agent + 桌面切換 | 服務以 LocalSystem 啟動；在作用中 session 以 winlogon token 建立 agent；agent 切換到目前 input desktop（Default / Winlogon）安裝 hook 與注入 | 不需驅動、可覆蓋登入/UAC/鎖定；Synergy/Deskflow 已證明可行 | 需要特權服務、桌面切換與 token 細節；SYSTEM 程式解析網路有安全風險 | **v1 採用** |
| C. 虛擬 HID 驅動 | 核心驅動建立虛擬鍵鼠 | 最強：BIOS 後、登入、UAC、遊戲、反作弊都可能支援 | 驅動開發、EV 簽章、Microsoft attestation、Blue Screen 風險、成本高 | 未來選項；v1 不做 |

> Deskflow 的 Windows daemon 正是做法 B：service 以 `winlogon.exe` token 建立 core process、設定 `TokenUIAccess`，再由 core 切換到目前 input desktop。我們會用**公開 API 重新實作**，不直接複製其 GPL 程式碼。

### 2.3 建議架構：服務 + 特權 core + 一般 UI

```
┌──────────────────────────────────────────────────────────────────┐
│ lankvm-ui（一般使用者權限）                                       │
│  Tauri 2 + React/TS：托盤、設定、佈局、配對、診斷                  │
└───────────────────────────┬──────────────────────────────────────┘
                            │ Named Pipe（ACL 限制）／Unix socket
┌───────────────────────────▼──────────────────────────────────────┐
│ lankvm-service（Windows: LocalSystem；macOS: root LaunchDaemon；  │
│                  Linux: root systemd service）                    │
│  - 開機自動啟動、watchdog、session 監控                           │
│  - 以 winlogon token 在作用中 session 啟動／重啟 lankvm-core       │
│  - 不處理輸入事件，降低攻擊面                                     │
└───────────────────────────┬──────────────────────────────────────┘
                            │ CreateProcessAsUser（SYSTEM + UIAccess）
┌───────────────────────────▼──────────────────────────────────────┐
│ lankvm-core（作用中 session；Windows 為 SYSTEM）                  │
│  - 桌面管理器：Default ↔ Winlogon ↔ 鎖定/UAC                      │
│  - 輸入捕獲／注入、游標隱藏、邊界切換                             │
│  - QUIC / mDNS / 配對 / 剪貼簿 / 檔案                             │
│  - 無 GUI；只開一個本機 IPC server                                │
└──────────────────────────────────────────────────────────────────┘
```

**為什麼 core 跑 SYSTEM：** 只有 SYSTEM（或具備同等權限並能取得 winlogon token）的 process 能開啟 Winlogon 桌面、在該桌面安裝 hook 並注入。若 core 只跑使用者權限，登入畫面一定失效。

**安全取捨：** core 同時處理網路與輸入，是網路暴露面。v1 先用 Rust + TLS + 嚴格協定 + 最小依賴降低風險；v1.1 可把網路層拆到獨立 broker，core 只留輸入/剪貼簿，再用本機 IPC 溝通。這是刻意分階段，不是忽略。

### 2.4 Windows 桌面切換設計

這是登入畫面功能的技術核心，必須照下列方式做，否則會遇到「hook 裝了但收不到 Winlogon 輸入」或「切換桌面後 hook 失效」。

1. **服務啟動 core**：`WTSGetActiveConsoleSessionId()` 取得作用中 session；列舉該 session 的 `winlogon.exe`，`OpenProcessToken` + `DuplicateTokenEx`，`CreateProcessAsUserW` 建立 core；設定 `TokenUIAccess=1`（若簽章/路徑允許）。
2. **桌面管理器**：core 內一個專用執行緒定期 `OpenInputDesktop(DF_ALLOWOTHERACCOUNTHOOK, FALSE, ...)`，讀取桌面名稱（`GetUserObjectInformationW`）。
3. **每個桌面一個 worker thread**：桌面名稱改變時，建立新的 worker，於該執行緒 `SetThreadDesktop`，再安裝 `WH_MOUSE_LL` / `WH_KEYBOARD_LL`、建立 message-only window 與 Raw Input；舊 worker 停止、卸載 hook、關閉桌面。
   - 不可在已建立視窗/ hook 的執行緒上直接 `SetThreadDesktop`；必須用新執行緒。
4. **注入**：worker 在該桌面呼叫 `SendInput`；滑鼠用絕對定位 + `MOUSEEVENTF_VIRTUALDESK`。
5. **監控**：每隔 100–250 ms 檢查桌面是否改變；鎖定、UAC、登入、螢幕保護都視為不同桌面狀態。
6. **恢復**：回到 Default 桌面時重建 worker，重新同步修飾鍵狀態，避免卡鍵。

> 注意：`Ctrl+Alt+Del` 是 Secure Attention Sequence，不能直接 `SendInput`。若真的需要，只能用 `SendSAS` API，且需群組原則允許 `SoftwareSASGeneration`；預設不做，UI 要說明。

### 2.5 登入畫面的前置條件

- **網路必須在登入前可用**：有線最穩；Wi-Fi 若要在登入前連線，需要 machine-level 設定（電腦憑證/802.1X 或預先連線），一般家用 Wi-Fi 通常也可以。若網路要到使用者登入後才建立，遠端無法控制登入畫面。
- **BitLocker 預開機、BIOS、開機選單不支援**：那些階段在 Windows 之前。
- **Windows 更新重開機**：若停在登入畫面，core 仍可由服務啟動；但更新過程中的特殊畫面不保證。
- **多使用者/快速切換**：服務需監控 session 變更，將 core 綁到目前作用中的 console session；切換時重建 worker。
- **遠端桌面（RDP）**：RDP session 與 console session 不同；v1 先不保證，列為測試項。

### 2.6 macOS / Linux 的登入畫面

| 平台 | 可行性 | 做法 | 風險 |
|------|--------|------|------|
| Windows | **高** | 服務 + SYSTEM agent + 桌面切換 | token/桌面切換細節；已有多個產品證明 |
| macOS | 中 | root LaunchDaemon + loginwindow agent；`CGEventPost` 或 `IOHIDUserDevice` 虛擬 HID | TCC/Accessibility、簽章/公證、loginwindow session 限制；需 PoC |
| Linux X11 | 高 | root systemd service；XRecord + XTest；顯示管理器登入畫面也是 X11 | 不同 DM 的啟動時機 |
| Linux Wayland | 低～中 | 使用者 session 用 libei/portal；登入畫面需 uinput/evdev 或 compositor 支援 | GDM 登入畫面、權限、游標定位；只能 best-effort |

**結論：** Windows 的登入畫面列為 v1 必須達標；macOS/Linux 先完成一般桌面，登入畫面列為平台 PoC，不承諾與 Windows 同等級。這點要寫進產品說明，避免跨平台期待落差。

---

## 3. 跨平台架構

### 3.1 行程/服務對照

| 元件 | Windows | macOS | Linux |
|------|---------|-------|-------|
| 特權 supervisor | Windows Service（LocalSystem） | LaunchDaemon（root） | systemd system service（root） |
| 特權 core | SYSTEM process in active session | root loginwindow/session agent | root X11 agent / uinput agent |
| 使用者 UI | Tauri tray app | Tauri menu bar app | Tauri tray app |
| IPC | Named Pipe + ACL | Unix domain socket + code signing check | Unix domain socket + SO_PEERCRED |
| 開機自啟 | Service auto-start | LaunchDaemon `RunAtLoad` | systemd `enable` |
| 登入畫面 | Winlogon desktop | loginwindow agent | X11 DM / uinput |

### 3.2 平台抽象層

```rust
pub trait InputBackend {
    fn start(&self, tx: Sender<InputEvent>) -> Result<()>;
    fn stop(&self) -> Result<()>;
    fn set_suppressed(&self, suppressed: bool) -> Result<()>;
    fn inject(&self, events: &[InputEvent]) -> Result<()>;
    fn warp_cursor(&self, x: i32, y: i32) -> Result<()>;
    fn set_cursor_visible(&self, visible: bool) -> Result<()>;
}

pub trait DesktopBackend {
    fn active_desktop(&self) -> DesktopKind; // Default / Secure / Locked / Unknown
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn virtual_bounds(&self) -> Rect;
    fn cursor_pos(&self) -> Point;
}

pub trait ClipboardBackend { /* read/write text, html, image, files */ }
pub trait FileDropBackend { /* OLE / NSFilePromise / XDND */ }
```

- `DesktopBackend` 在 Windows 由桌面管理器實作；macOS/Linux 先回傳 Default，之後補上 loginwindow/X11。
- UI 只依賴 `lankvm-core` 的 IPC API，不直接碰平台 API。

### 3.3 建置順序

1. **Windows PoC**：輸入、邊界、游標、登入/UAC、延遲。
2. **Windows 核心 v1**：服務、core、探索/配對、佈局、剪貼簿、UI、安裝。
3. **Windows 檔案 v1**：複製貼上、圖片、進度。
4. **macOS 一般桌面**：CGEventTap/Post、NSPasteboard、TCC 引導。
5. **Linux X11**：XRecord/XTest、X11 selection。
6. **登入畫面跨平台 PoC**：macOS LaunchDaemon/loginwindow、Linux X11 DM/uinput。
7. **Linux Wayland**：libei/portal best-effort。

---

## 4. 多螢幕設計

目前 A/B 各一顆，但資料模型從第一天就要支援多螢幕，否則之後要重寫佈局與邊界。

### 4.1 資料模型

```rust
struct MonitorInfo {
    id: u32,
    name: String,
    rect: Rect,        // 實體像素，虛擬桌面座標（可為負）
    work_area: Rect,
    dpi: u32,
    primary: bool,
}

struct NodeLayout {
    node_id: NodeId,
    monitors: Vec<MonitorInfo>,
    canvas_rect: Rect, // 邏輯畫布中的位置；由 UI 拖曳設定
}
```

- 座標一律用**實體像素**，避免混合 DPI 時比例錯誤。
- Windows 使用 per-monitor DPI awareness v2；`GetSystemMetricsForDpi` / `GetDpiForMonitor`。
- 佈局畫布用邏輯單位顯示，儲存時保留實體尺寸與 DPI。

### 4.2 邊界判定（monitor 級）
不要只用 node 的 bounding box，因為 monitor 可能有空洞（例如上下排列、左右錯開）。

1. 對每個 monitor 的四條邊，扣掉與同一 node 其他 monitor 相鄰的區段。
2. 剩下的區段就是「外部邊界」。
3. 外部邊界依佈局鄰接關係連到另一個 node 的外部邊界。
4. 游標在外部邊界 + 外向位移 → 觸發切換；在內部邊界（同一台電腦的兩個螢幕之間）不觸發。
5. 進入目標 node 時，依 Y 找垂直範圍包含映射點的 monitor；若沒有，找最近者，再 warp 到該 monitor 的邊。

### 4.3 解析度/DPI 不同

```text
t = (cursor_y - source_segment.top) / source_segment.height
target_y = target_monitor.top + clamp(t, 0, 1) * target_monitor.height
```

- 用「邊段」比例，不是整台電腦的比例；多螢幕時才不會跳到錯的螢幕。
- 目標螢幕比來源窄/寬時，水平位置只影響進入點，不影響後續相對移動（後續用絕對座標）。
- 混合 DPI：注入用實體像素；UI 顯示用邏輯尺寸，避免 150% 時看起來大小不一。

### 4.4 測試

- 單螢幕：目前 A/B。
- 雙螢幕：左右、上下、主螢幕在右、負座標。
- 混合 DPI：100% + 150%、100% + 200%。
- 虛擬顯示器：Windows 可用虛擬顯示驅動或外接螢幕；沒有實機時用 DisplayFusion/虛擬螢幕測試。
- 邊界：四邊、角落、內部邊界、空洞。

### 4.5 節點左右關係（必要設定）

「哪一台在左、哪一台在右」不能寫死，必須由使用者設定：

- **P0**：master 用 `--edge right|left` 指定自己面向 peer 的那一側；被控端的進入邊由 `Switch` 封包帶入（相反側）。
- **v1 UI**：在佈局畫布上把節點拖到左／右（或上／下），存進 `NodeLayout.canvas_rect`；邊界與進入邊都由佈局推導，不再有 CLI 參數。
- 設定要能保存（每台機器記住 peer 方位），換場域或換座位不用改程式。
- 多螢幕時以「外部邊段」為單位設定，不是整台電腦的 bounding box。

> 2026-10-23 測試需求：A 在左、B 在右時 A 用 `--edge right`；若實際座位相反，A 改成 `--edge left`，不需重啟 B。

---

## 5. 技術棧最終建議

### 5.1 建議：Rust + Tauri 2（從零開始，不 fork）

| 構面 | 評估 |
|------|------|
| 執行效率 | Rust 原生、無 GC；core 常駐記憶體目標 < 40 MB（不含 WebView） |
| 輸入控制 | `windows` crate 直接呼叫 Win32；hook/注入/桌面切換完整可控 |
| 網路 | tokio + quinn + rustls；datagram/stream 分離，適合低延遲與大檔案 |
| UI | Tauri 2 + React/TS；版面編輯器、配對精靈、i18n 開發快；安裝檔小 |
| 安全 | 記憶體安全；網路協定可用 Rust 生態的成熟 TLS/QUIC；適合 SYSTEM 常駐 |
| 跨平台 | platform trait；macOS/Linux backend 可替換 |
| 授權 | 可選 GPL-3.0 或 MIT/Apache；不直接複製 Deskflow 程式碼 |
| 風險 | 登入畫面/桌面切換需自寫；Windows 輸入調校需 PoC |

### 5.2 為什麼不 fork Deskflow

Deskflow 很成熟，且已解決 Windows 登入/UAC，但：

- **授權**：GPL-2.0-only + OpenSSL exception；本專案若要 MIT/Apache 就不能用；若用 GPL-2.0，整體授權與相依套件要審查。
- **技術債**：C++/Qt、舊 Synergy 協定、TCP、手動 IP 設定；要加 mDNS/配對/群組/檔案/拖放，等於在舊架構上大幅改造。
- **UI/品牌**：Qt Widgets 要改成極簡跨平台 UI，成本不低；Tauri 的版面編輯器與 i18n 更快。
- **功能缺口**：Deskflow 沒有我們要的現代配對、檔案複製/拖放；這些正是工作量最大的部分。
- **維護**：fork 30k stars 的 C++ 專案，長期合併 upstream 與除錯成本高。

**但 Deskflow 是重要的行為參考與測試對照**：Windows service/winlogon token/桌面切換、游標隱藏、邊界行為都可觀察。我們用公開 API 重新實作，不複製程式碼。

### 5.3 決策門檻（fallback）

P0 PoC 若出現以下任一情況，暫停並重新評估是否 fork Deskflow：

- 無法穩定在 Winlogon 桌面安裝 hook / 注入（登入畫面失敗）。
- 有線 p95 延遲 > 25 ms，且調校後仍無法達到順暢。
- 游標隱藏在主要應用（瀏覽器、Office、Explorer、遊戲）大量失效。
- 桌面切換造成卡鍵、hook 遺失、藍屏/當機。

若 PoC 成功，維持 Rust/Tauri 從零開發。

---

## 6. 授權建議

| 選擇 | 適用 | 注意 |
|------|------|------|
| **GPL-3.0-or-later**（建議） | 開源、避免專有 fork、與 Apache-2.0 相依相容 | 不能直接複製 Deskflow（GPL-2.0-only）程式碼 |
| GPL-2.0-only | 想直接移植 Deskflow 程式碼 | 需審查所有相依套件（Apache-2.0-only 不相容） |
| MIT / Apache-2.0 | 最大採用率、可閉源衍生 | 不能使用 GPL 程式碼；社群貢獻可能被商業 fork |

**建議先採 GPL-3.0-or-later，clean-room 實作。** 若 PoC 後決定大量移植 Deskflow，再改為 GPL-2.0-only 並做授權審查。

---

## 7. 修訂估時

| 階段 | 內容 | 估時（1 位資深） |
|------|------|------------------|
| P0 | 2 台 Windows PoC：輸入/邊界/游標/**登入畫面**/延遲 | 3–4 週 |
| P1 | 服務 + core + UI 骨架、mDNS/配對、佈局、鍵盤、文字剪貼簿、安裝 | 8–10 週 |
| P2 | 圖片剪貼簿、檔案複製貼上、進度/續傳、權限 | 4–5 週 |
| P3 | 拖放、HUD、診斷、簽章、自動更新、打磨 | 3–4 週 |
| **Windows v1 合計** | 不含 macOS/Linux | **18–23 週**（約 4.5–5.5 個月） |
| P4 | macOS 一般桌面 + 登入畫面 PoC | 4–6 週 |
| P5 | Linux X11 + 登入畫面；Wayland best-effort | 4–6 週 |

> 比 v0.1 增加，主要因為登入畫面/服務架構、跨平台抽象、多螢幕資料模型與安全設計。若只做 Windows 且登入畫面可接受「有線 + 已配對」限制，P0–P3 仍是 18–23 週。

---

## 8. P0 PoC 計畫（Windows）

### 8.1 目標

用兩台 Windows 實機（或一台實機 + 一台 VM，但 VM 的輸入/桌面切換不完全代表實機）驗證：

1. 主控端 low-level hook + Raw Input 邊界方向判定。
2. 被控端 `SendInput` 絕對定位 + 游標隱藏/釘住。
3. 左右切換、回切、卡鍵/斷線恢復。
4. **登入畫面**：client 鎖定/登出/重開機到登入畫面，master 可移動滑鼠、點擊、輸入密碼。
5. **UAC 安全桌面**：master 可操作 UAC 同意視窗。
6. 有線 1GbE 端到端延遲 p95 < 15 ms；Wi-Fi 記錄 p95。
7. 24 小時 soak：記憶體/CPU/handle 穩定，hook 不失效。

### 8.2 技術驗證清單

- [ ] 服務以 LocalSystem 安裝/啟動/停止/自動重啟。
- [ ] 以 winlogon token 在 active session 建立 core。
- [ ] `OpenInputDesktop` 偵測 Default/Winlogon 切換。
- [ ] 每個桌面 worker thread 安裝 hook、卸載、重建。
- [ ] Winlogon 桌面 `SendInput` 絕對定位/鍵盤。
- [ ] `TokenUIAccess` 設定成功/失敗的行為（有簽章 vs 無簽章）。
- [ ] 鎖定/解鎖、UAC、登入、快速使用者切換。
- [ ] hook callback 時間 < 1 ms；無 hook timeout。
- [ ] 注入事件過濾（`LLMHF_INJECTED` + magic）。
- [ ] 螢幕休眠/喚醒、網路斷線重連。
- [ ] 防毒/SmartScreen 對服務與 hook 的反應。

### 8.3 量測

- 在 hook 與注入端打 `QueryPerformanceCounter`，記錄 p50/p95/p99。
- 用 1000 Hz 滑鼠、144 Hz 螢幕、有線 1GbE。
- 封包遺失/延遲模擬（可選 tc/netem 或 Windows 工具）。
- 主觀測試：快速甩動、慢推、斜向、角落、邊界按鈕。

### 8.4 Gate

- 登入畫面與 UAC 必須成功；否則回到技術選型（考慮 fork Deskflow 或驅動）。
- 延遲 p95 < 15 ms 有線；> 25 ms 視為不可接受。
- 100 次切換無卡鍵、無游標殘留、無 hook 遺失。

---

## 9. 品牌：LAN-KVM 極簡 Logo（已定案）

**定案：01 Seam** — 兩個螢幕輪廓，中間是左右雙向箭頭；兩個箭頭各跨過左右螢幕內側的一半，表示雙向切換。

| # | 名稱 | 概念 | 檔案 |
|---|------|------|------|
| 1 | **Seam（定案）** | 兩個螢幕 + 各跨內側一半的左右雙向箭頭 | `logo-01-seam.svg` / `logo.svg` |
| 2 | K | 以螢幕邊框構成的 K 字 | `logo-02-k.svg` |
| 3 | Nodes | 左/中/右三個螢幕，中間高亮 | `logo-03-nodes.svg` |
| 4 | Bridge | 兩個螢幕以弧線橋接 | `logo-04-bridge.svg` |
| 5 | Switch | 極簡 toggle 開關 | `logo-05-switch.svg` |

- 風格：幾何、粗線條、深藍底 + 青色重點；16 px 仍可辨識。
- 已輸出：SVG 主檔、透明 mark、單色黑/白、16–1024 px PNG、Windows ICO、macOS ICNS、系統匣 template icon、深/淺色系統匣圖示。
- 路徑：`assets/branding/`；完整預覽：`assets/branding/gallery.html`；使用規範：`assets/branding/README.md`。

---

## 10. 下一步

1. 建立 Rust workspace + 5 個 crate 骨架（core/platform/net/input/agent）。
2. 實作 Windows P0 PoC：服務 + core + hook + 注入 + 桌面切換 + 延遲 HUD。
3. 兩台實機測試；通過 Gate 後才做 UI 與檔案功能。
4. Logo 已定案（Seam）；圖示包已輸出，套用到安裝檔、托盤與 UI。
5. 補齊協定規格（v0.1 附錄 A 為起點）與威脅模型。

> 目前不建議先做漂亮的 UI 或完整檔案功能；登入畫面與游標體驗是專案成敗點，必須先驗證。
