# LAN-KVM 技術選型評估

> 版本：v0.1　日期：2026-10-21　狀態：待決策  
> 範圍：常駐式軟體 KVM（鍵盤/滑鼠共享、剪貼簿、檔案傳輸）
>
> **更新（v0.2）：** 需求方已確認「開源、需操作登入畫面、跨平台架構、多螢幕、名稱 LAN-KVM」。登入畫面會改變行程/權限架構，請優先閱讀 [`02-decisions-and-architecture.md`](02-decisions-and-architecture.md)。

---

## 0. 結論（TL;DR）

**建議採用：Rust + Tauri 2，Windows 10/11 優先，架構預留 macOS / Linux。**

主要原因：

1. 本專案的核心是**全域輸入攔截、輸入注入、游標控制、剪貼簿/OLE、低延遲網路**，全部都需要直接呼叫作業系統 API。Rust 的 `windows` crate 可完整覆蓋 Win32，且記憶體安全適合長期常駐、會接收網路連線的程式。
2. Tauri 2 提供系統匣、安裝檔、自動更新與 Web UI；版面編輯器、配對精靈、多語系用 Web 技術開發速度最快。安裝檔約 5–15 MB，未開視窗時常駐記憶體約 30–60 MB。
3. 網路層用 `tokio` + `quinn`（QUIC）：滑鼠移動走 unreliable datagram 求最低延遲；鍵盤、剪貼簿、檔案走 reliable stream；內建 TLS 1.3，不需要自己實作加密與重傳。
4. 跨平台路徑明確：Windows 先做；macOS 使用 CGEventTap / CGEventPost；Linux X11 使用 XRecord / XTest；Wayland 以 libei + portal 做 best-effort。

**不建議直接 fork Deskflow**：它是 GPL-2.0，且配對、UI、檔案拖放都要大改。若本專案要閉源商業化，授權會是問題。Deskflow 適合當行為參考與測試對照，不適合當產品底座。

**三個最大技術風險（必須先 PoC）：**

- 主控端游標「全域隱藏 + 釘在邊界」在各種應用程式下的穩定性。
- 被抓取端的 1:1 游標移動：絕對定位、螢幕加速、混合 DPI、多螢幕。
- 權限：提高權限視窗（UAC）、鎖定畫面、BIOS/開機前無法覆蓋；以及 Windows 對注入的種種限制。

**估時：** Windows v1（含複製貼上檔案、不含原生跨機拖放）約 11–15 人週；加上拖放與打磨約 14–19 人週。單人全職約 3–4.5 個月可到 Windows 可用版；macOS / Linux 另計 6–10 人週。

---

## 1. 需求拆解與技術含義

| # | 需求 | 技術含義 | 難度 |
|---|------|----------|------|
| 1 | 同一套軟體安裝在多台電腦 | 單一安裝檔、系統匣常駐、開機自啟、單一實例、自動更新、x64/ARM64 | 低 |
| 2 | 同網段掃描、認出對方、配對 | mDNS-SD + UDP broadcast + 手動 IP/QR；TLS 憑證指紋 + PAKE/SAS；防火牆規則 | 中 |
| 3 | 設定任一電腦為主控端，2–3 台共用鍵鼠 | 全域低階 hook、SendInput、權限與注入、控制權租約（同一時間只有一個控制者） | 高 |
| 4 | 電腦位置左 / 中 / 右 | 共用螢幕佈局模型、多螢幕、DPI、邊界鄰接圖 | 中 |
| 5 | 邊界偵測 + 方向判定 + 絲滑切換 | Raw Input 位移、游標隱藏/釘住、遠端絕對定位、低延遲傳輸、狀態機 | 高 |
| 6 | 英文（預設）/ 中文 | i18n（en、zh-TW，可加 zh-CN）、字型與排版 | 低 |
| 7 | 數個 Logo 供選擇 | SVG 品牌套件、ICO/ICNS、托盤單色圖示、深/淺色 | 低 |
| 8 | 剪貼簿分享（文字/圖片） | CF_UNICODETEXT/HTML/RTF/DIB、剪貼簿監聽、延遲傳輸、隱私控制 | 中 |
| 9 | 檔案複製：拖放 + 複製貼上 | QUIC 串流、分塊/雜湊/續傳、CF_HDROP、OLE 虛擬檔、拖放偵測 | 高 |

**一句話總結：** 這不是單純的「滑鼠鍵盤轉發器」，而是「輸入裝置虛擬化 + 分散式游標 + 跨機剪貼簿 + 跨機檔案系統橋接」的組合。網路與 UI 不是最難的，**作業系統層的輸入與游標才是**。

---

## 2. 平台範圍建議

### 2.1 建議順序

| 階段 | 平台 | 理由 |
|------|------|------|
| v1 | Windows 10 22H2（已 EOL，仍可支援）/ 11（x64，ARM64 後補） | 需求最大、Win32 API 最完整、同網段 PC 最常見 |
| v1.1 | macOS 13+（Apple Silicon / Intel） | CGEventTap 可行，但需要 Accessibility / Input Monitoring 權限與公證 |
| v1.2 | Linux X11 | XRecord / XTest 成熟 |
| v1.3 | Linux Wayland | 依 compositor 支援 libei/portal，只能 best-effort |

不建議一開始三平台並行。三個平台的輸入 backend 差異很大，會拖慢最關鍵的「絲滑度」調校。

### 2.2 平台 API 對照

| 功能 | Windows | macOS | Linux X11 | Wayland |
|------|---------|-------|-----------|---------|
| 全域監聽 | WH_MOUSE_LL / WH_KEYBOARD_LL + Raw Input | CGEventTap | XRecord | libei / InputCapture portal |
| 輸入注入 | SendInput | CGEventPost | XTest | libei |
| 隱藏游標 | ShowCursor / ClipCursor / SetSystemCursor | CGDisplayHideCursor | XFixes | compositor 相依 |
| 剪貼簿 | Win32 Clipboard / COM | NSPasteboard | X11 selection | wl-clipboard / portal |
| 跨機拖放 | OLE IDataObject / CF_HDROP | NSFilePromiseProvider | XDND | portal（受限） |
| 權限 | 管理員 / UIAccess | Accessibility、Input Monitoring、Screen Recording | X11 存取 | portal 使用者同意 |
| 開機前/鎖定畫面 | 不支援（除非驅動/UIAccess） | 不支援 | 不支援 | 不支援 |

> 重要期望管理：軟體式 KVM **無法**覆蓋 BIOS、開機選單、Windows 鎖定畫面、UAC 安全桌面、Ctrl+Alt+Del。這些情境需要硬體 KVM 或簽章驅動/UIAccess，成本完全不同。

---

## 3. 技術棧候選比較

### 3.1 候選方案

| 方案 | 核心優勢 | 主要弱點 | 適合情境 |
|------|----------|----------|----------|
| **Rust + Tauri 2** | Win32 控制完整、記憶體安全、體積小、QUIC/async 生態成熟、跨平台 | Rust 學習曲線、UI 需要 Web 技術、各平台仍需原生碼 | 長期常駐、重視品質與跨平台 |
| **C# / .NET 10 + WinUI 3 / WPF** | Windows 整合最好、P/Invoke 直接、OLE/剪貼簿容易、開發快 | Windows-only（跨平台需 Avalonia）、體積/記憶體較大、注入仍需原生 API | 確定 Windows-only、團隊熟 C# |
| **C++ / Qt（fork Deskflow）** | 已有輸入/剪貼簿/多螢幕基礎、活躍社群 | GPL-2.0、Qt 授權、舊架構、配對/拖放仍需重寫、建置複雜 | 開源專案、可接受 GPL |
| **Go + Wails/Fyne** | 網路與單一執行檔方便 | Win32 hook/OLE 需 cgo，GUI 生態弱，低延遲控制較差 | 網路服務為主、非本專案 |
| **Electron + native addon** | UI 開發快 | 體積/記憶體過大、native addon 仍要 C++/Rust | 不建議用於常駐 KVM |

### 3.2 加權評分（主觀，滿分 5）

| 指標 | 權重 | Rust+Tauri | C#/.NET | C++/Qt | Go | Electron |
|------|-----:|-----------:|--------:|-------:|---:|---------:|
| Win32/系統控制能力 | 30% | 5 | 5 | 5 | 2 | 2 |
| 低延遲網路 | 15% | 4 | 4 | 4 | 5 | 3 |
| UI / i18n / 版面編輯器 | 15% | 4 | 4 | 2 | 3 | 5 |
| 常駐資源與安裝體積 | 10% | 5 | 3 | 4 | 4 | 1 |
| 跨平台潛力 | 10% | 4 | 2 | 4 | 4 | 4 |
| 開發速度 | 10% | 3 | 5 | 2 | 4 | 5 |
| 授權與長期維護 | 10% | 5 | 4 | 2 | 5 | 4 |
| **加權總分** | 100% | **4.40** | **4.10** | **3.60** | **3.50** | **3.20** |

> 分數不是絕對；若「Windows-only + 團隊只會 C#」則 C# 會是更務實的選擇。

### 3.3 為什麼推薦 Rust + Tauri

- 低階輸入需要大量 Win32 結構與指標操作，`windows` crate 提供型別安全的 FFI；不需要 C++/Qt 的建置負擔。
- `tokio` + `quinn` + `rustls` 可以在同一個 process 內同時處理輸入 datagram、檔案串流與 TLS，不需要額外服務。
- Tauri 的托盤、安裝檔、更新器、單一實例、自動啟動都有現成插件；UI 用 React/Svelte 開發版面編輯器與配對精靈很快。
- 記憶體安全對「會接收網路輸入注入指令」的常駐程式特別重要。
- 之後要加 macOS/Linux，只需替換 platform backend，核心狀態機/協定/UI 可重用。

**建議版本（2026-10 可用）：**

| 用途 | Crate | 版本 |
|------|-------|------|
| Win32 API | `windows` | 0.62 |
| 非同步執行期 | `tokio` | 1.54 |
| QUIC | `quinn` | 0.11 |
| TLS | `rustls` | 0.23 |
| mDNS | `mdns-sd` | 0.21 |
| 剪貼簿 | `arboard`（文字/圖片）+ 自寫 CF_HDROP/OLE | 3.7 |
| 憑證 | `rcgen` | 0.14 |
| 配對 PAKE | `spake2` | 0.4 |
| 圖片 | `image` | 0.25 |
| UI | `tauri` | 2.12（Tauri 3 還在 alpha，暫不使用） |
| 輸入注入（跨平台備援） | `enigo` | 0.6（Windows 主要仍用自寫 SendInput） |

### 3.4 什麼情況該選 C# / .NET

- 明確只做 Windows，且團隊 C# 經驗遠大於 Rust。
- 需要大量 Windows 原生 UI、OLE 拖放、Shell 整合，且希望用現成 `System.Windows.Clipboard` / WPF DragDrop。
- 可接受自包含發佈約 60–150 MB、常駐記憶體約 100 MB 以上。
- 網路可用 `System.Net.Quic`（MsQuic）或 TCP；但底層輸入 hook/注入仍要 P/Invoke，並沒有比較簡單。

### 3.5 是否 fork Deskflow / Input Leap / Barrier

| 專案 | 狀態 | 授權 | 評估 |
|------|------|------|------|
| Deskflow | 活躍（2026-10 仍有 commit，約 3 萬 stars） | GPL-2.0 | 輸入/剪貼簿/多螢幕基礎完整；但配對、UI、檔案拖放、i18n 需大改；GPL 不適合閉源產品 |
| Input Leap | 已 archive（2025-12） | 多為 GPL-2.0 | 不建議 |
| Barrier | 2024 後停滯 | GPL-2.0 | 不建議 |

**結論：** 若是開源專案，可以考慮在 Deskflow 上貢獻；若是商業/閉源產品，從零開始、只參考其行為與測試案例，會比對抗 GPL 與舊架構便宜。

---

## 4. 推薦架構

### 4.1 高階架構

```
┌────────────────────── PC A（主控端 / Controller）──────────────────────┐
│  Tauri UI（托盤、設定、配對、版面編輯）                                  │
│        │ IPC（僅本機）                                                   │
│        ▼                                                                 │
│  Agent（Rust，常駐）                                                     │
│   ├─ Input Capture：低階 hook + Raw Input                               │
│   ├─ Cursor/Layout State Machine：邊界、鄰接、控制權租約                 │
│   ├─ Input Injector：SendInput（被控制時）                               │
│   ├─ Clipboard Engine：監聽、格式轉換、延遲傳輸                          │
│   ├─ File Engine：QUIC 串流、分塊、雜湊、續傳                            │
│   └─ Net：mDNS 探索、配對、QUIC 連線、頻道管理                           │
└──────────────────────────────┬──────────────────────────────────────────┘
                               │  LAN（TLS 1.3 / QUIC）
┌──────────────────────────────▼──────────────────────────────────────────┐
│  PC B（被控端 / Controlled）                                             │
│  Agent（Rust）                                                           │
│   ├─ Input Hook（僅用來過濾自己的注入事件、偵測本地取回控制）             │
│   ├─ Input Injector：SendInput 絕對定位                                  │
│   ├─ Clipboard Engine                                                    │
│   └─ File Receiver                                                       │
└──────────────────────────────────────────────────────────────────────────┘
```

### 4.2 行程與執行緒模型

- 每個使用者 session 一個 `lankvm-agent`（系統匣常駐）；單一實例用 named mutex。
- **Hook 執行緒**：安裝 WH_MOUSE_LL / WH_KEYBOARD_LL，必須有 message loop。callback 內只做「讀取 + 放進 lock-free/bounded queue + 必要時 return 1」，**絕不做網路 I/O、鎖等待、配置**。Windows 的 `LowLevelHooksTimeout` 預設 300 ms，超時會被系統移除 hook。
- **Raw Input 執行緒**：message-only window + `RIDEV_INPUTSINK`，取得未經滑鼠加速的相對位移；邊界方向判定用這個。
- **Tokio runtime**：探索、QUIC、剪貼簿、檔案。
- **注入執行緒**：高優先權，`timeBeginPeriod(1)`，批次送 SendInput。
- **UI**：Tauri WebView 只在開啟視窗時建立；關閉視窗後只留托盤，降低常駐記憶體。

### 4.3 權限模型

| 模式 | 執行身分 | 可控制範圍 | 建議 |
|------|----------|------------|------|
| 標準模式 | 一般使用者 | 一般應用程式；**無法**注入提高權限視窗 | v1 預設，安全性較好 |
| 進階模式 | 工作排程器「最高權限」啟動 | 可控制管理員視窗（仍無法覆蓋 UAC 安全桌面） | 選項，首次設定時 UAC 一次 |
| 服務模式 | SYSTEM + 使用者 session agent | 全使用者、鎖定畫面（仍需 UIAccess/驅動） | 不建議 v1，複雜度高 |

> 建議 v1 先做標準模式；進階模式用「工作排程器 + 最高權限」而非 Windows Service，因為 Session 0 的服務無法直接 SendInput 到使用者桌面。

### 4.4 程式碼結構（Rust workspace）

```
LAN-KVM/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ lankvm-core/            # 型別、設定、佈局、狀態機、協定
│  ├─ lankvm-platform/        # trait + windows/ macos/ linux/ 實作
│  ├─ lankvm-net/             # mDNS、配對、QUIC、頻道
│  ├─ lankvm-input/           # capture、edge detector、injector、cursor
│  ├─ lankvm-clipboard/       # 格式轉換、監聽、CF_HDROP
│  ├─ lankvm-files/           # 傳輸引擎、續傳、hash
│  └─ lankvm-agent/           # 二進位：托盤、組裝、生命週期
├─ apps/desktop/              # Tauri 2（src-tauri + React/TS UI）
├─ docs/
└─ assets/branding/
```

Platform trait 範例：

```rust
pub trait InputBackend {
    fn start_capture(&self, tx: Sender<InputEvent>) -> Result<()>;
    fn set_suppressed(&self, suppressed: bool) -> Result<()>;
    fn inject(&self, events: &[InputEvent]) -> Result<()>;
    fn warp_cursor(&self, x: i32, y: i32) -> Result<()>;
    fn set_cursor_visible(&self, visible: bool) -> Result<()>;
}

pub trait ScreenBackend {
    fn monitors(&self) -> Vec<MonitorInfo>;   // 實體像素 + DPI + 虛擬桌面座標
    fn cursor_pos(&self) -> (i32, i32);
    fn virtual_bounds(&self) -> Rect;
}
```

---

## 5. 關鍵設計

### 5.1 輸入捕獲與注入（Windows）

#### 捕獲

- **低階 hook**：`WH_MOUSE_LL`、`WH_KEYBOARD_LL`，全域攔截；用 `SetWindowsHookExW` + message loop。
- **Raw Input**：`RegisterRawInputDevices`（`RIDEV_INPUTSINK`），取得 `WM_INPUT` 的 `lLastX/lLastY`。這是**未加速的裝置位移**，也是游標被螢幕邊界夾住後仍能判斷「繼續往外推」的關鍵。
- **注入事件過濾**：`MSLLHOOKSTRUCT.flags & LLMHF_INJECTED`、`KBDLLHOOKSTRUCT.flags & LLKHF_INJECTED`，再加上 `dwExtraInfo` 的 magic value（例如 `0x4C4B564D`）。沒有這層過濾，遠端注入會被自己的 hook 再抓一次，形成迴圈。
- **Hook watchdog**：定期檢查 hook 是否還有效（例如送一個已知事件或檢查 hook handle），失效就重裝；程式退出、鎖定、切換使用者時要卸載。

#### 注入

- **滑鼠移動：絕對定位優先。**
  ```c
  INPUT in = {0};
  in.type = INPUT_MOUSE;
  in.mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
  in.mi.dx = (LONG)(x * 65535.0 / (virtual_w - 1) + 0.5);
  in.mi.dy = (LONG)(y * 65535.0 / (virtual_h - 1) + 0.5);
  SendInput(1, &in, sizeof(in));
  ```
  - `MOUSEEVENTF_VIRTUALDESK` 讓座標映射到整個虛擬桌面（含負座標、多螢幕），不是只有主螢幕。
  - 絕對定位**不受「提高指標精確度」影響**，不會有相對位移的加速曲線問題，也不會累積漂移。
  - 解析度：65535 對 4K 寬度約 0.06 px/step，足夠。
  - 多螢幕有「空洞」時（例如上下排列但左右錯開），映射到空洞會被 OS 夾到最近螢幕；需要按 monitor rect 選最近的有效點。

- **滑鼠移動：相對定位模式（遊戲/全螢幕備援）。**
  - 使用 `MOUSEEVENTF_MOVE` 相對位移。
  - 相對位移會受滑鼠加速影響；Deskflow 的做法是**暫時**用 `SystemParametersInfo(SPI_SETMOUSE/SPI_SETMOUSESPEED)` 關閉加速、送出、再還原。這會動到全域設定，其他程式可能受影響，且時序有風險。
  - 較好的做法：若遊戲支援 raw input，改用 absolute 模式；真的不行才提供「相對模式」選項，並在 UI 明講限制。

- **鍵盤：掃描碼（scan code）優先。**
  - `KEYEVENTF_SCANCODE` + `MapVirtualKey`，支援不同鍵盤佈局與多數遊戲。
  - 處理 extended key（E0）、AltGr（Ctrl+Alt）、媒體鍵、Win 鍵。
  - 鎖定鍵（Caps/Num/Scroll）狀態同步：切換時查詢 `GetKeyState`，必要時補送 toggle。
  - **按鍵重複**：本地 OS 會產生重複 keydown；遠端注入若已按住，遠端 OS 也會重複，容易變成雙重節奏。需要在協定中標記 `repeat`，遠端只注入一次 keydown，或過濾遠端自動重複。此項要實測。
  - **修飾鍵狀態同步**：切換瞬間把目前按住的 Shift/Ctrl/Alt/Win 狀態帶到新機器；離開時對舊機器送 key-up，避免卡鍵。
  - **IME/CJK**：主控端進入遠端模式時，把本地 IME 切到英數/關閉（`ImmSetOpenStatus`），否則本地 IME 會吃掉按鍵、顯示候選字；遠端 IME 才能正常組字。鍵盤佈局不同時要警告，或提供「Unicode 文字模式」（會失去快捷鍵/IME 行為）。

#### 權限與限制

| 情境 | 可否 | 說明 |
|------|------|------|
| 一般應用程式 | 可 | 標準模式 |
| 管理員視窗（工作管理員、安裝程式） | 需進階模式 | 注入端與目標需同/更高完整性（UIPI） |
| UAC 安全桌面 | 不可 | 需 UIAccess 簽章程式或驅動 |
| 鎖定畫面 / 登入畫面 | 不可 | 需 credential provider / 驅動 |
| Ctrl+Alt+Del | 不可 | Secure Attention Sequence，OS 保留 |
| 遊戲 / 反作弊 | 多數不可 | SendInput 常被忽略或判定異常；需虛擬 HID 驅動（簽章成本高） |
| 遠端桌面（RDP）內 | 部分 | 取決於 session 與 hook；需實測 |

> 這些限制要在 UI/FAQ 寫清楚，不要讓使用者以為是萬能 KVM。

### 5.2 邊界切換與「絲滑」游標

#### 佈局模型

不要用「左/中/右」三個字串直接寫死，而是建立一個**共用佈局畫布**：

```json
{
  "canvas": { "width": 5760, "height": 2160 },
  "nodes": [
    { "id": "pc-a", "name": "DESKTOP-A", "x": 0,    "y": 0, "w": 1920, "h": 1080, "monitors": [...] },
    { "id": "pc-b", "name": "DESKTOP-B", "x": 1920, "y": 0, "w": 1920, "h": 1080, "primary": true },
    { "id": "pc-c", "name": "DESKTOP-C", "x": 3840, "y": 0, "w": 1920, "h": 1080, "monitors": [...] }
  ]
}
```

- 每個 node 上報自己的 monitor rect（實體像素 + DPI + 虛擬桌面原點）。
- 佈局以「邏輯像素」儲存，跨機器解析度不同時按比例映射；不要假設大家都是 1920×1080。
- 主控端（master）持有佈局；變更時同步給所有節點，並用版本號避免不同步。
- 多螢幕：node 的邊界是「所有 monitor 的聯集邊界」；進入/離開時依 Y 找最近的有效 monitor 邊緣，避免掉進螢幕間的空洞。

#### 邊界偵測

```text
條件：
1. 本地游標在 node 的外邊界 1 px 內（GetCursorPos + 邊界 rect）
2. Raw Input 累積的外向位移 >= threshold（預設 20 px，可調 10–50）
3. 在時間窗內（預設 200 ms）
4. 沒有按下滑鼠按鍵（除非是檔案拖放模式）
5. 通過 hysteresis：離開後 300 ms 內不再觸發，避免彈回
6. 可選：需要按住熱鍵（遊戲/精細操作模式）
```

- 一定要用 Raw Input 的位移，不能只看 `WM_MOUSEMOVE`：游標被邊界夾住後座標不變，鉤子拿不到「繼續往外」的資訊。
- 方向：`dx > 0` 且貼右邊 → 往右切；`dx < 0` 且貼左邊 → 往左切；上下同理（若支援垂直佈局）。
- 可加「邊緣阻力」：需要累積一定位移才切換，避免拉捲軸/點邊界按鈕時誤切。
- 切換前送 `SWITCH_REQ`，目標回 `SWITCH_ACK` 才真正切；逾時就留在本地並顯示提示。避免封包遺失造成「本地已放開、遠端沒收到」的失控狀態。

#### 游標隱藏與釘住

這是「絲滑」的關鍵，也是最需要實測的部分。

- **隱藏**：`ShowCursor(FALSE)` 迴圈直到 display counter < 0（Deskflow 也是這樣做），離開遠端模式時 `ShowCursor(TRUE)` 還原。
- **釘住**：用 `ClipCursor` 把游標限制在離開邊界的 1×1 區域，避免本地游標跑到別的螢幕或觸發 hover。某些應用會重設 clip，需要定時重套。
- **備援**：若 `ShowCursor` 在某些應用無效，使用 `SetSystemCursor` 換成透明游標，退出時 `SystemParametersInfo(SPI_SETCURSORS)` 還原；必須有 watchdog，避免程式崩潰後系統游標一直空白。
- **不要**修改使用者的滑鼠加速設定；用絕對定位繞過加速。
- 進入遠端時，把本地游標停在邊界；遠端游標在對應邊界的相同 Y 比例出現。因為本地游標已隱藏，視覺上就是「從這邊滑過去」。

#### 進入點映射

```text
y_ratio = (local_y - node.rect.top) / node.rect.height
remote_y = remote.rect.top + clamp(y_ratio, 0, 1) * remote.rect.height
remote_x = (方向 == 右) ? remote.rect.left : remote.rect.right - 1
```

- 不同解析度/高度時按比例，游標不會突然跳到頂端或底部。
- 若遠端該 Y 落在螢幕間空洞，往最近的 monitor 邊緣靠。
- 進入後的第一個 motion 事件要用絕對座標，避免「先跳到 (0,0) 再修正」的閃動。

#### 狀態機

```text
Local ──(edge + ACK)──► Controlling ──(return edge / hotkey / peer request)──► Returning ──► Local
                             │
                             └──(peer 本地輸入 / 斷線 / 鎖定 / 使用者按 Esc)──► Returning

Controlled ──(release)──► Local
```

- **控制權租約**：同一時間只有一個 controller。`lease_id` + `seq`，舊的 lease 事件直接丟棄。
- **斷線處理**：controller 端若 200 ms 內沒收到 heartbeat，立刻恢復本地輸入、顯示游標、釋放所有按鍵；controlled 端也釋放所有按鍵，避免卡鍵。
- **本地取回**：被控端收到「非注入」的本地鍵鼠事件時，可設定為「本地優先」自動取回控制（例如使用者動了 B 的實體滑鼠）。這比只能靠熱鍵更直覺。
- **熱鍵**：預設 `Ctrl + Alt + Shift + Q` 或「連按兩次 Ctrl」回到主控端；熱鍵必須在 hook 內比對，且不能轉發到遠端。熱鍵可設定。

#### 延遲目標

| 情境 | 目標 p95 | 說明 |
|------|----------|------|
| 有線 1GbE | < 15 ms | 捕獲→編碼→網路→解碼→注入→顯示 |
| Wi-Fi 5/6 良好訊號 | < 30 ms | 需關閉省電、避免 2.4 GHz 擁塞 |
| 可接受上限 | 50 ms | 超過就會明顯「拖」 |

- 滑鼠移動封包 250–500 Hz（每包 8–16 bytes），不要用 1000 Hz 直送，浪費 CPU/網路。
- 有線不啟用 jitter buffer；Wi-Fi 可啟用 0–8 ms 自適應 buffer，但要以實測為準。
- `timeBeginPeriod(1)`、注入執行緒提高優先權、關閉 Wi-Fi power save。
- 內建 HUD：RTT、jitter、loss、目前 Hz、目前模式，方便現場調校。

#### 什麼做不到

- 無法把「同一個視窗」拖過機器；視窗屬於原機器。跨機拖放只支援**檔案**（見 5.6）。
- 無法在 BIOS/鎖定畫面/UAC 安全桌面使用。
- 若遠端套用會主動設定游標位置的程式（部分遊戲、繪圖板驅動），可能與絕對定位打架；需要 per-app 測試。

### 5.3 網路與傳輸

#### 為什麼用 QUIC

| 方案 | 優點 | 缺點 | 結論 |
|------|------|------|------|
| TCP 自訂 framing | 簡單、可靠 | 檔案傳輸會 head-of-line block 輸入；加密/重傳要自己做 | 不建議 |
| 原始 UDP + 自訂可靠層 | 最低延遲 | 加密、重傳、擁塞控制全自寫；易錯 | 不建議 |
| **QUIC（quinn）** | TLS 1.3、多 stream、datagram、擁塞控制、成熟 | 少量額外開銷（LAN 可忽略） | **推薦** |
| WebRTC DataChannel | 瀏覽器級 NAT 穿透 | 對 LAN 過重、jitter buffer 較大 | 不需要 |
| SMB / HTTP | 現成 | 認證/防火牆/進度整合差，難做即時 | 只當備援 |

#### 頻道設計

| 頻道 | 型態 | 內容 | 可靠性 |
|------|------|------|--------|
| `control` | QUIC stream | HELLO、配對、佈局、SWITCH_REQ/ACK、RELEASE、心跳 | 可靠、有序 |
| `motion` | QUIC datagram | `{seq, x, y, buttons, wheel}` 絕對座標 | 不可靠；舊 seq 直接丟棄 |
| `input` | QUIC stream | 按鍵、滑鼠鍵、滾輪 | 可靠、有序 |
| `clipboard` | QUIC stream | 文字/HTML/圖片/檔案清單 | 可靠 |
| `files` | 獨立 QUIC 連線或多 stream | 檔案分塊 | 可靠；**建議獨立連線**避免 bulk 影響輸入 |

> 檔案傳輸建議用**另一條 QUIC 連線**（或至少獨立 congestion context），避免大檔案把 input 的 congestion window 吃掉。若共用連線，QUIC datagram 仍受 congestion control 影響，不保證即時。

#### 訊息編碼

- `postcard` + serde（緊湊、no_std 友善）或 CBOR（可除錯）。建議 postcard，另提供 debug 文字日誌。
- 每個訊息有 `{version, type, seq, len}`；控制訊息上限 1 MB，檔案 chunk 上限 4 MB。
- 未知 type/version 要優雅拒絕，不能 panic。
- 所有長度/陣列先驗證再配置，防 DoS。

#### 探索

1. **mDNS/DNS-SD（主要）**：advertise `_lankvm._udp.local`，TXT 放 `id`、`name`、`ver`、`port`、`fp`（憑證指紋短雜湊）、`role`。`mdns-sd` crate 持續維護。
2. **UDP broadcast（備援）**：`255.255.255.255:port` 每 2 秒廣播一次；適合 mDNS 被防火牆/AP 阻擋的網路。
3. **手動 IP / QR（最後手段）**：輸入 IP 或掃 QR（QR 內容含 IP、port、一次性配對 token）。
4. **IPv6 link-local**：可選，企業網路若關 IPv4 multicast 可用。

> 很多 Wi-Fi 熱點/訪客網路會啟用 **AP isolation**，同網段但彼此不通。安裝時要偵測並明確提示「請改用有線或關閉 AP isolation」，不要讓使用者以為程式壞了。

#### 配對與信任

輸入注入等同遠端控制權，**絕對不能「同網段就信任」**。

建議流程：

1. 每個節點首次啟動產生 Ed25519 keypair + 自簽憑證（`rcgen`），device id = 公鑰雜湊。
2. 配對時用 **SPAKE2**（RFC 9382）搭配 6 位數代碼，或 TLS 握手後顯示 **SAS**（兩邊比對相同 6 位數）。
3. 握手完成後交換憑證指紋並**pin**；之後重連只接受 pinned key（TOFU + pin）。
4. 憑證/金鑰存在 OS 憑證存放區（Windows Credential Manager / DPAPI），不要明文放設定檔。
5. 支援「群組」：第一台建立群組，其他台用邀請碼加入；群組成員之間以群組 CA 簽發的成員憑證互信，之後**任一台上線都能當主控端**，不需要重新配對。
6. 每條連線雙向認證；未配對的 peer 只能看到「可配對」狀態，不能送任何輸入/剪貼簿/檔案。

> 如果 v1 想簡化：採星狀（所有流量經 master 轉發），配對只跟 master 做。缺點是換 master 要重新配對；若需求是「可設定任一電腦為主控端」，建議一開始就用群組 + 全互連（3–6 台規模很小，成本不高）。

#### 防火牆與 QoS

- 安裝檔加入 Windows Firewall inbound rule（UDP + TCP 自訂 port，例如 45871）；mDNS 用 5353。
- 只在需要時開 Port；配對視窗結束後可關閉「接受新連線」，既有 peer 不受影響。
- 滑鼠 motion 可標 DSCP EF / Wi-Fi WMM，但家用 AP 不一定理會；別過度期待。
- 顯示 RTT/jitter/loss；建議有線優先。

### 5.4 剪貼簿

#### 監聽

- Windows：`AddClipboardFormatListener` + `WM_CLIPBOARDUPDATE`，在專用 message-loop 執行緒。
- 去抖：一次複製可能觸發多個格式更新；用 sequence number / 50–100 ms debounce。
- 忽略自己寫入造成的更新（寫入前記錄 hash/sequence）。

#### 格式

| 類型 | Windows 格式 | 傳輸 | 備註 |
|------|--------------|------|------|
| 純文字 | CF_UNICODETEXT | UTF-8 | 最常用；限制 256 KB 可即時推送 |
| HTML | CF_HTML | UTF-8 + header | 保留連結/格式；需處理 CF_HTML header offset |
| RTF | CF_RTF | UTF-8 | 可選 |
| 圖片 | CF_DIB / CF_DIBV5 | PNG | 保留 alpha；4K 截圖約 10–30 MB，極限 64 MB |
| 檔案清單 | CF_HDROP | 檔案傳輸流程 | 見 5.6 |

#### 同步策略

需求是「A 複製，滑鼠移到 B，B 可以貼上」。有三種做法：

1. **立即推送**（最簡單）：A 一複製就送給所有 peer。缺點：隱私（密碼、公司文件）與頻寬（大圖）。
2. **切換時推送**（推薦 v1）：A 複製時只記住內容/雜湊；滑鼠切到 B 時，把「目前剪貼簿」推送給 B；B 貼上即可。符合使用情境，也減少外洩。
3. **延遲拉取**（最省）：只送 offer（類型/大小/hash），B 真正按貼上或切換時才拉。實作較多，但最安全。

建議：**文字小於 256 KB 可立即推送（可關閉）；圖片/檔案只送 offer，切換或貼上時才拉。** 另提供「剪貼簿同步」開關與「暫停 5 分鐘」。

#### 隱私與安全

- 尊重 Windows 的 `ExcludeClipboardContentFromMonitorProcessing`、`CanIncludeInClipboardHistory` 等格式（密碼管理器常用）。
- 不要記錄剪貼簿內容到 log；只記類型/大小/hash。
- 每個 peer 可個別設定「允許剪貼簿」；預設只允許已配對裝置。
- 可設定「只在切換時同步」，避免背景一直傳。
- 圖片先壓縮（PNG/WebP），超過上限顯示「內容過大，未同步」。

#### 寫入遠端剪貼簿

- `OpenClipboard` 可能被其他程式佔用；用重試 + backoff（例如 10 次 × 20 ms）。
- `GlobalAlloc(GMEM_MOVEABLE)` + `SetClipboardData`；所有權轉移後不要自行 free。
- 寫入後可 `CloseClipboard`；必要時用 `EmptyClipboard` 清舊格式。
- 檔案貼上：v1 先下載到暫存資料夾，設定 CF_HDROP，再注入 Ctrl+V；v1.1 再做 OLE 虛擬檔（Explorer 顯示原生進度）。

### 5.5 檔案複製 / 貼上

#### 流程（Ctrl+C → Ctrl+V）

```text
A: 使用者在 Explorer 選檔按 Ctrl+C
   → A agent 偵測 CF_HDROP（或由 Explorer 選取範圍取得清單）
   → 送 FILE_OFFER {files:[name,size,hash], total} 給 B
B: 使用者在目標資料夾按 Ctrl+V
   → B agent 在 hook 看到 Ctrl+V，且剪貼簿是「遠端檔案 offer」
   → 送 FILE_ACCEPT；顯示進度
   → A 用 QUIC stream 分塊傳送；B 寫入 *.part
   → 完成後驗證 hash、atomic rename、設定 CF_HDROP
   → B 注入 Ctrl+V 給前景應用 / Explorer
```

- 為什麼要攔 Ctrl+V：Windows 的 CF_HDROP 只能指向**本機真實路徑**；檔案還沒下載完之前，不能讓 Explorer 直接貼上。先下載再貼最穩定。
- 大檔案：顯示進度、可取消、支援續傳（以 hash/offset 續傳）。
- 檔名衝突：自動改名 `file (1).txt`，或詢問覆蓋。
- 完成後可選「開啟資料夾」。
- 若使用者複製後直接關機/斷線，offer 失效，B 顯示「來源離線」。

#### 流程（拖放）

**v1（heuristic，建議先做）**

- 偵測「左鍵按下 + 移動超過閾值 + 前景視窗是 Explorer + 有選取檔案」→ 視為檔案拖曳。
- 游標碰到邊界 → 切換到 B，顯示半透明「放開以複製到 B」覆蓋層。
- 在 B 放開 → 解析落點（桌面 / 開著的 Explorer 資料夾），傳輸完成後放到該資料夾；若無法解析，放桌面或預設資料夾。
- 限制：只支援 Explorer 檔案拖曳；從 Outlook/瀏覽器拖出的附件不支援（走複製貼上）。

**v1.1（原生 OLE 虛擬檔，較完整）**

- 在 B 建立 `IDataObject`，提供 `CFSTR_FILEDESCRIPTORW` + `CFSTR_FILECONTENTS`，以 `IStream` 按需從 A 拉資料。
- Explorer 會顯示原生複製進度，可以直接拖進任何支援 OLE drop 的應用。
- 難點：`IStream` 非同步、Explorer 的 timeout/重試、檔案大小未知時的處理、跨行程 COM 記憶體管理。需要單獨 PoC。
- macOS 對應 `NSFilePromiseProvider`；Linux XDND 可先只做「下載到資料夾」。

> 不建議 v1 就做原生 OLE 拖放；先用 heuristic 把使用情境跑通，再決定是否投資。

#### 傳輸引擎

- Chunk 1–4 MB；BLAKE3 hash；可續傳；平行多檔案（2–4 streams）。
- 可壓縮類型用 zstd（文字/log），已壓縮類型（jpg/mp4/zip）跳過。
- 保留 mtime；長路徑用 `\\?\`；>4 GB 用 u64。
- 接收端先寫 `.part`，完成後 atomic rename；磁碟空間不足要先檢查。
- 檔名 sanitize（路徑穿越、保留字、Unicode normalization）。
- 可選 Mark-of-the-Web（Zone.Identifier），避免下載的執行檔被當成「本機產生」。
- 安全：只接受已配對 peer；每 peer 可關閉檔案傳輸；大小/數量上限；不自動執行。

### 5.6 UI / UX / i18n

#### 系統匣

- 狀態：Local / Controlling B / Controlled by A / Offline。
- 快速動作：暫停控制、切換主控端、剪貼簿開關、開啟設定、結束。
- 圖示：正常/連線中/被控/錯誤四種狀態；macOS 用 template icon。

#### 主視窗

1. **裝置**：線上/離線、名稱、IP、延遲、權限、配對/解除。
2. **佈局**：可拖曳的螢幕畫布；把每台電腦的 monitor 群組放到左/中/右；顯示解析度與 DPI；「測試切換」按鈕。
3. **配對**：6 位數代碼 + 兩邊比對；顯示指紋；群組邀請。
4. **設定**：語言、開機自啟、邊界靈敏度、熱鍵、剪貼簿策略、檔案上限、進階模式、更新。
5. **診斷**：RTT/jitter/loss、目前 Hz、hook 狀態、防火牆狀態、匯出 log（去識別化）。

#### 邊界回饋

- 游標接近邊界時，邊緣出現 1–2 px 的淡光（可關）。
- 切換成功：短暫 toast「Control → DESKTOP-B」。
- 切換失敗：toast「B 無回應，已留在本地」。
- 第一次使用：提示「把游標推到螢幕邊緣即可切換；按 Ctrl 兩次可回來」。

#### i18n

- 預設英文；繁中（zh-TW）完整翻譯；可加 zh-CN。
- 使用 JSON 資源（`locales/en.json`、`zh-TW.json`）或 Fluent；所有 UI 字串不得寫死。
- 日期/數字/檔案大小用 locale 格式化；中文檔名/路徑要測。
- 說明文件/FAQ 也分語言；錯誤碼集中管理，log 用英文。

### 5.7 安裝、更新、簽章

- **安裝檔**：Tauri NSIS/MSI；per-machine 安裝（UAC 一次）以寫入防火牆規則與 Program Files；WebView2 evergreen bootstrapper（Windows 10 可能沒有）。
- **開機自啟**：標準模式用 Run key / Startup；進階模式用 Task Scheduler「最高權限」。
- **單一實例**：named mutex；第二次啟動只開設定視窗。
- **自動更新**：Tauri updater + 簽章 manifest；可延後/略過；更新失敗可 rollback。
- **Windows 簽章**：OV/EV code signing certificate；沒有簽章會被 SmartScreen 擋，且防毒容易誤判。EV 可較快建立信譽。
- **防毒**：低階 hook + 網路 + 自啟是敏感組合；簽章、送 Microsoft/防毒廠商白名單、不要在背景做可疑行為、提供清楚隱私政策。
- **macOS**：Developer ID + notarization；Accessibility/Input Monitoring 需要 TCC 引導。
- **隱私**：不收集剪貼簿/檔案內容；診斷資料 opt-in；log 去識別化。

---

## 6. 風險與限制

| 風險 | 影響 | 機率 | 對策 | 階段 |
|------|------|------|------|------|
| 游標隱藏/釘住在某些應用失效 | 高（體驗崩壞） | 中 | 先 PoC；ShowCursor + ClipCursor + 透明系統游標備援；建立 app 測試矩陣 | P0 |
| 注入被 UAC/安全桌面/遊戲阻擋 | 高（功能缺口） | 高 | v1 標準模式；進階模式；FAQ 說清楚；驅動列為未來選項 | P0/P1 |
| 相對位移加速/漂移 | 高 | 中 | 預設絕對定位；相對模式僅備援；實測 1000 Hz 滑鼠 | P0 |
| 多螢幕/混合 DPI 映射錯誤 | 中 | 中 | per-monitor v2；邏輯畫布；4K/150%/200% 測試 | P1 |
| Wi-Fi 延遲/jitter/AP isolation | 中 | 高 | 有線建議；HUD；自適應 buffer；偵測 AP isolation | P1 |
| 跨機拖放 OLE 複雜 | 高（時程） | 高 | v1 heuristic；v1.1 才做 OLE 虛擬檔 | P3 |
| 防毒/SmartScreen 誤判 | 高（無法安裝） | 中 | 簽章、白名單、乾淨安裝檔、少用 packer | P1 |
| 剪貼簿隱私外洩 | 高 | 中 | 預設切換時同步；密碼管理器格式排除；peer 權限；不記錄內容 | P2 |
| 未配對裝置注入（RCE 級） | 極高 | 低 | TLS + pin + SPAKE2；未配對只允許配對；rate limit；fuzz | P1 |
| macOS TCC / Wayland 限制 | 中 | 高 | 延後；best-effort；UI 明確標示 | P4 |
| GPL / 第三方授權 | 高（商業） | 低 | 從零開發；license audit；避免 GPL 程式碼 | P0 |

---

## 7. 開發階段與估時

| 階段 | 內容 | 驗收 / Gate | 估時（1 位資深） |
|------|------|-------------|------------------|
| **P0 PoC** | 2 台 Windows：hook、抑制、Raw Input、絕對注入、游標隱藏、邊界切換、UDP/QUIC motion、延遲量測 | 有線 p95 < 15 ms；100 次切換不卡鍵；游標不閃現；可回切 | 2–3 週 |
| **P1 核心 v1** | mDNS/broadcast 探索、配對、群組/信任、佈局、3 台、鍵盤/修飾鍵、熱鍵、托盤、i18n、安裝/自啟 | 3 台可穩定互相切換；重開機自動連線；英文/繁中 | 6–8 週 |
| **P2 剪貼簿/檔案** | 文字/HTML/圖片、CF_HDROP、QUIC 檔案引擎、複製貼上、進度/取消/續傳 | 20 MB 圖片 < 2 s；1 GB 檔 > 80 MB/s 有線；衝突/斷線可恢復 | 3–4 週 |
| **P3 拖放/打磨** | heuristic 拖放、OLE 虛擬檔（選）、HUD、診斷、簽章、自動更新、FAQ | 拖檔到 B 桌面成功；安裝無 SmartScreen 警告（簽章後） | 3–4 週 |
| **P4 跨平台** | macOS、Linux X11、Wayland best-effort | macOS 基本鍵鼠/剪貼簿/檔案；Linux X11 可用 | 6–10 週 |

**Windows v1.0 合計：約 14–19 人週**（不含 P4）。若只做「最小可用」：P0 + P1 + 文字剪貼簿，約 8–11 週；再加上複製貼上檔案約 11–15 週。

> 估時假設：需求不再膨脹、有 2 台實機可測、UI 可由同一人兼做。若需要多螢幕/遊戲/企業部署，往上加。

---

## 8. 測試計畫

### 8.1 環境矩陣

- 2 台 / 3 台 Windows 10 22H2、Windows 11 23H2/24H2。
- 有線 1GbE、Wi-Fi 5/6（含訊號弱/2.4 GHz 擁塞）。
- 滑鼠 125/1000 Hz；螢幕 60/144 Hz；1080p/1440p/4K。
- 混合 DPI：100% / 150% / 200%；多螢幕（左右、上下、負座標）。
- 防毒：Defender + 1 家第三方。

### 8.2 功能

- 邊界：慢推、快甩、斜向、角落、來回、邊界按鈕/捲軸、最大化視窗。
- 鍵盤：注音/拼音/倉頡、Emoji、AltGr、媒體鍵、Caps/Num、Ctrl+Alt+Del（預期不可）。
- 切換：按住 Shift 切換、拖曳中切換、遠端本地滑鼠取回、熱鍵、逾時、斷線。
- 剪貼簿：CJK/emoji、HTML、20 MB 圖片、密碼管理器格式、連續複製、貼上到不同 app。
- 檔案：1 GB、10 萬小檔、長路徑、Unicode、唯讀、衝突、磁碟滿、取消/續傳、>4 GB。
- 系統：睡眠/喚醒、鎖定/解鎖、UAC、使用者切換、網路拔線、IP 變更、master 離線。

### 8.3 效能

- 端到端延遲：hook timestamp → 遠端注入 timestamp；p50/p95/p99；有線/Wi-Fi 各 1 小時。
- 封包遺失 0 / 0.1 / 1%；jitter 0 / 5 / 20 ms。
- 24 小時 soak：記憶體/CPU/handle 穩定；hook 不失效。
- 檔案吞吐：1 GB 單檔、10 萬小檔；CPU/記憶體。

### 8.4 安全

- 未配對裝置嘗試送輸入/剪貼簿/檔案 → 拒絕。
- MITM/錯誤 SAS → 配對失敗。
- 重播/舊 seq/畸形封包/fuzz → 不 crash、不卡死。
- 路徑穿越、超長檔名、惡意檔案大小。
- 日誌不含剪貼簿/檔案內容。

### 8.5 可用性

- 10 位使用者首次配對：目標 < 2 分鐘。
- 誤切換率：一般文書 1 小時 < 1 次（可調靈敏度後 0）。
- 「絲滑」主觀評分：5 分制 ≥ 4。

---

## 9. 需要你決策的事項

1. **平台**：先做 Windows-only，還是 v1 就要 macOS/Linux？
2. **開源/閉源**：是否商業產品？能否接受 GPL？（決定是否可 fork Deskflow）
3. **控制範圍**：需要控制管理員視窗、遊戲、鎖定畫面嗎？（決定是否做進階模式/驅動/簽章）
4. **主控端**：固定設定，還是執行中可隨時切換任一台上線電腦？（決定星狀 vs 群組全互連）
5. **螢幕**：每台電腦是否多螢幕？解析度/DPI 組合？
6. **網路**：有線為主，還是必須支援 Wi-Fi？現場 AP 是否開 client isolation？
7. **技術棧**：Rust + Tauri（推薦）或 C# / .NET？團隊比較熟哪個？
8. **品牌**：產品名稱？Logo 風格（極簡科技/友善/企業）？是否有既有 CI？
9. **剪貼簿**：預設開啟還是關閉？可接受的最大圖片/檔案大小？
10. **部署**：需要代碼簽章、自動更新、企業大量部署（MSI/GPO）嗎？

---

## 10. 下一步建議

1. **先做 P0 PoC，不要先做完整 UI。** 用兩台 Windows 實機驗證：
   - 低階 hook + 抑制 + Raw Input 方向判定。
   - 絕對定位注入 + 游標隱藏/釘住 + 回切。
   - 1 GbE 下的 p95 延遲。
   - 通過後才進入 P1。
2. PoC 通過後建立 Rust workspace + Tauri 骨架，先做托盤/配對/佈局，再做剪貼簿/檔案。
3. 品牌：Logo 已於 v0.2 定案為 **Seam**；圖示包已輸出，見 `assets/branding/`。
4. 文件：協定規格、狀態機、測試案例、FAQ/限制說明。

### Logo 方向（v0.2 已定案 Seam；以下為歷史草案）

| # | 概念 | 視覺 | 適合 |
|---|------|------|------|
| 1 | **Seam / 無縫** | 兩個圓角螢幕微微重疊，中間一道光，游標箭頭跨過接縫 | 最直接表達「無縫切換」 |
| 2 | **Three Nodes / 三點** | 三個圓角矩形橫排，中間高亮，一條游標線連左右 | 表達主控 + 左/右 |
| 3 | **K Arrow / K 箭頭** | 用螢幕邊框與箭頭Negative space 組成 K | 科技、好記、適合 16px |
| 4 | **Orbit / 環繞** | 三個裝置圍繞一個游標，形成環 | 群組/多機 |
| 5 | **Switch / 切換** | 兩個螢幕 + 一個 toggle/箭頭 | 極簡、企業感 |

- 色彩建議：深藍 `#0B1220`、青色 `#22D3EE`、白色 `#F8FAFC`、警示琥珀 `#F59E0B`。
- 交付：SVG 主檔、單色版、16/24/32/48/64/128/256 PNG、ICO、ICNS、macOS template tray icon、深/淺色背景版。
- 名稱建議（需商標查詢）：`SeamLink`、`KVMesh`、`EdgeLink`、`CursorBridge`、`LanKVM`。避免 `DeskFlow`（與 Deskflow 太近）。

---

## Appendix A：協定草案

```text
ALPN: lankvm/1
Header: { version: u8, msg_type: u16, seq: u64, len: u32 }

Control (reliable stream)
  HELLO            { device_id, name, version, capabilities }
  AUTH_CHALLENGE   { nonce }
  AUTH_RESPONSE    { proof, cert_fp }
  JOIN_GROUP       { group_id, invite_token }
  LAYOUT_UPDATE    { version, nodes[] }
  SWITCH_REQ       { lease_id, target_id, edge, y_ratio, seq }
  SWITCH_ACK       { lease_id, accepted, entry_x, entry_y }
  RELEASE          { lease_id, reason }
  PING / PONG      { t0, t1 }

Motion (datagram)
  MOTION           { lease_id, seq, x, y, buttons, wheel_x, wheel_y }

Input (reliable stream)
  KEY              { lease_id, seq, scan_code, down, repeat, modifiers }
  BUTTON           { lease_id, seq, button, down }
  WHEEL            { lease_id, seq, delta, horizontal }

Clipboard (reliable stream)
  CLIP_OFFER       { kind, size, hash, formats[] }
  CLIP_REQUEST     { offer_id, format }
  CLIP_DATA        { offer_id, format, bytes }
  CLIP_CANCEL      { offer_id }

Files (reliable stream / separate connection)
  FILE_OFFER       { transfer_id, files[{name,size,hash,mtime}], total }
  FILE_ACCEPT      { transfer_id, resume_offsets[] }
  FILE_CHUNK       { transfer_id, file_index, offset, bytes }
  FILE_DONE        { transfer_id, hash }
  FILE_CANCEL      { transfer_id, reason }
```

## Appendix B：參考資料

- Deskflow（GPL-2.0，活躍）：Windows 輸入/剪貼簿/多螢幕參考；其 `MSWindowsDesks` 使用 `ShowCursor` 迴圈隱藏游標、相對移動時暫時關閉滑鼠加速。
- Input Leap（2025-12 已 archive）、Barrier（2024 後停滯）：不建議作為底座。
- Microsoft Docs：`SendInput`、`MOUSEEVENTF_VIRTUALDESK`、低階 hook、Raw Input、`AddClipboardFormatListener`、CF_HDROP、OLE drag-and-drop、UIPI。
- RFC 9000（QUIC）、RFC 9001（TLS over QUIC）、RFC 9002（Loss Recovery）。
- RFC 6762（mDNS）、RFC 6763（DNS-SD）。
- RFC 9382（SPAKE2）。
- Tauri 2、quinn、rustls、mdns-sd、arboard、rcgen 官方文件。

---

> 本文件是技術評估，不是最終規格。PoC 結果（尤其是延遲、游標隱藏、權限）可能改變部分選型。
