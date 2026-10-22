# LAN-KVM 品牌資產

**定案 Logo：Seam** — 兩個螢幕輪廓，中間是左右雙向箭頭；兩個箭頭各跨過左右螢幕內側的一半，表示雙向切換。

## 檔案

| 檔案 | 用途 |
|------|------|
| `logo.svg` | 主要 App Icon（深藍底，512×512） |
| `logo-01-seam.svg` | Seam 主檔（與 `logo.svg` 相同構圖） |
| `logo-mark.svg` | 透明背景 mark，適合深色底 |
| `logo-mark-dark.svg` | 透明背景 mark，適合淺色底 |
| `logo-mono-black.svg` | 單色黑（印刷、印章、浮水印） |
| `logo-mono-white.svg` | 單色白（深色底） |
| `icons/lankvm-*.png` | 16–1024 px App Icon |
| `icons/lankvm.ico` | Windows App Icon（16/24/32/48/64/128/256） |
| `icons/lankvm.icns` | macOS App Icon |
| `icons/tray-light-*.png` / `tray-light.ico` | 深色工作列／系統匣用 |
| `icons/tray-dark-*.png` / `tray-dark.ico` | 淺色工作列／系統匣用 |
| `icons/tray-template-*.png` | macOS menu bar template icon |
| `gallery.html` | 五個草案的完整預覽（已標記定案） |

## 色彩

| 名稱 | Hex | 用途 |
|------|-----|------|
| Ink | `#0B1220` | 主要深色背景 |
| Cyan | `#22D3EE` | 品牌重點色、雙向箭頭 |
| Paper | `#F8FAFC` | 螢幕輪廓、淺色底 |
| Cyan Dark | `#0891B2` | 淺色底上的替代重點色 |
| Amber | `#F59E0B` | 警示／狀態（非 Logo 主色） |

## 使用原則

- 最小尺寸：App Icon 16 px；低於 24 px 建議改用 `tray-*` 的簡化版。
- 安全邊距：圖示外圍至少留 1/8 邊長；不要讓其他元素侵入圓角。
- 深色底用 `logo.svg` 或 `logo-mark.svg`；淺色底用 `logo-mark-dark.svg`。
- 單色印刷用 `logo-mono-black.svg`；深色底單色用 `logo-mono-white.svg`。
- 不要：改變箭頭方向、把箭頭改成單向、拉長/壓扁螢幕、把青色換成其他顏色、加陰影/漸層/外框。
- 系統匣：Windows 依工作列主題選 `tray-light` / `tray-dark`；macOS 用 `tray-template`（template image 會自動隨選單列變色）。

## 重新產生

App Icon 由 `logo.svg` 產生；PNG/ICO/ICNS 使用 `sharp` 與 `png-to-ico`。若需要重新輸出，請在 `assets/branding` 下依下列尺寸產生：

- PNG：16, 24, 32, 48, 64, 128, 256, 512, 1024
- ICO：16, 24, 32, 48, 64, 128, 256
- ICNS：16, 32, 64, 128, 256, 512, 1024（含 @2x 表示）

> `icons/_contact-sheet.png` 是尺寸檢查用的暫存圖，不屬於正式交付。
