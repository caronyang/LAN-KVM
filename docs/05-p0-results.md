# P0 兩台實機測試結果

> 日期：2026-10-21  
> 版本：`lankvm-devtools` dev-fast（未簽章開發版）  
> 狀態：**滑鼠邊界切換 / 回切 / 網路握手：PASS**

## 1. 測試環境

| 角色 | IP | 網路 | 備註 |
|------|----|------|------|
| A（master） | `192.168.137.1` | 區域連線*2 | `D:\caron\99_temp\LAN-KVM` |
| B（client） | `192.168.137.53` | Wi-Fi | `D:\99_Temp\opencode\LAN-KVM` |

- A ping 得到 B。
- B 的 Wi-Fi 網路類別是 **Public**，測試時把 UDP 45871 規則設為 `Profile Any`；正式環境要改回 Private/Domain 並由 IT 管理。
- 使用 debug/dev-fast 建置，因為 release devtools 會被 Apex One 刪除。
- **實體 USB KVM 備註：** 測試時只有一組實體鍵鼠，透過 USB KVM 在 A/B 之間切換。這對「功能驗證」有效：KVM 切到 A 時，A 就是 master，B 透過網路接收。但 KVM 的滑鼠模擬模式（relative/absolute）、polling rate 與切換時的 USB 重新列舉，可能影響手感與延遲；最終效能驗收要用「A 直接接一組鍵鼠、B 不接鍵鼠」的乾淨環境。若 KVM 在 B 也保留一個 emulated device，B 的本地輸入可能與注入衝突。

## 2. 指令

B：

```powershell
.\target\dev-fast\lankvm-devtools.exe poc --role client --port 45871 --seconds 120
```

A：

```powershell
.\target\dev-fast\lankvm-devtools.exe poc --role master --peer 192.168.137.53 --port 45871 --edge right --seconds 120
```

## 3. 結果

### 3.1 網路／握手

A：

```
[master] client online
[latency] rtt=23 ms
[latency] rtt=49 ms
[latency] rtt=3 ms
[latency] rtt=6 ms
...
```

B：

```
[client] listening on 0.0.0.0:45871
[client] master connected from 192.168.137.1:60780
```

- 握手成功。
- RTT 初始有 23/49 ms 尖峰（可能 Wi-Fi 省電/首次封包），之後穩定在 **3–9 ms**。
- 以 dev-fast（未最佳化）來說，這個延遲很好；正式版目標有線 p95 < 15 ms。

### 3.2 邊界切換

A：

```
[master] remote control ON (Right edge, entry ratio 38446)
[master] remote motion 94/s
[master] remote control OFF (edge)
```

B：

```
[client] controlled; entry at (0, 633) from left
[client] released at edge
```

- A 的右邊界觸發成功。
- B 在左邊界 `(0, 633)` 進入，Y 比例映射正確。
- `remote motion 94/s` 表示 `ClipCursor` 後 Raw Input 仍持續收到移動，這是 P0 最關鍵的驗證。
- B 往左邊界推可回切；A 收到 `OFF (edge)`，沒有卡在 remote。

> 註（2026-10-23 新場域實測）：P0 的 client **不捕捉本地輸入**。回切判定是 client 累積 master 送來的 `dx <= -30`，所以實際操作是**把 master 的滑鼠往左推**（或按 `Ctrl+Alt+Shift+Q`）。在 client 機器上推本地滑鼠不會回切；B 本地滑鼠只會停在邊界。這是 P1 local takeover 要解的問題。
- A 的游標隱藏／釘住與恢復正常（由使用者操作確認）。

## 4. 已驗證

- [x] 兩台 UDP 握手、PONG/latency
- [x] 主控端右邊界 + 方向判定
- [x] 被控端左邊界進入、Y 比例映射
- [x] 游標隱藏、ClipCursor 後 Raw Input 持續運作
- [x] 被控端往左邊界回切
- [x] 斷線/無 PONG 時不進入 remote（liveness guard）
- [x] 無卡鍵/游標殘留（本次滑鼠測試）

## 5. 尚未驗證

- [ ] 鍵盤輸入（一般鍵、Shift/Ctrl/Alt/Win、Caps/Num）
- [ ] 滑鼠左右中鍵、XButton、滾輪、水平滾輪
- [ ] 注音/拼音/倉頡/Emoji
- [ ] `Ctrl+Alt+Shift+Q` 熱鍵取回
- [ ] 被控端本地滑鼠/鍵盤「取回控制」
- [ ] 網路中斷、client 關閉、master 關閉
- [ ] 不同 DPI / 解析度 / 多螢幕
- [ ] 4K / 144 Hz / 1000 Hz 滑鼠
- [ ] 24 小時 soak
- [ ] 管理員視窗 / UAC / 登入畫面（下一階段服務）

## 6. 已知限制

- **相對注入**：B 的游標移動受 B 的滑鼠加速影響，A/B 靈敏度可能不同；正式版改絕對定位。
- **未加密、未配對**：只適用於隔離測試網段。
- **dev-fast 未最佳化**：延遲與 CPU 使用率不是最終數字。
- **Windows only**：macOS/Linux 尚未實作。
- **Apex One**：release devtools 會被刪；正式產品需簽章 + 白名單。

## 7. 下一步

1. 完成鍵盤/按鈕/滾輪/熱鍵測試。
2. 修正任何卡鍵或修飾鍵狀態問題。
3. 進入 **Windows 服務 + SYSTEM agent + Winlogon 桌面切換**：
   - 服務以 LocalSystem 啟動。
   - 以 `winlogon.exe` token 在 active session 建立 core。
   - `OpenInputDesktop` / `SetThreadDesktop` 切換 Default / Winlogon。
   - 測試鎖定、UAC、登入畫面。
4. 服務通過後才做 QUIC/TLS、配對、剪貼簿、檔案、Tauri UI。
