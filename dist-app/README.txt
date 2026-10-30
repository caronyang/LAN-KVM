LAN-KVM 1.1.9
==========

這是一個**單一安裝檔**：LAN-KVM-Setup-1.1.9.exe
App 與 SYSTEM 服務都已內嵌，不需要其他檔案。
安裝精靈有 English / 繁體中文可選；程式 UI 預設 English，右上角可切換中文。

需求
----
- Windows 10 / 11 x64
- 兩台電腦在同一個區域網路（同一個 AP / Switch）

安裝
----
1. 建議先從系統匣右鍵「停止連線」再安裝（避免升級時中斷遠端控制）。
2. 執行 LAN-KVM-Setup-1.1.9.exe（會跳 UAC，按「是」）。
3. 安裝到 C:\Program Files\LAN-KVM，並自動：
   - 建立防火牆規則：UDP 45870 / 45871 / 45872、TCP 45873（log 回傳）、TCP 45875（剪貼簿）
   - 安裝並啟動 LAN-KVM SYSTEM 服務（登入畫面 / UAC），session agent 不顯示 console
   - 設定開機自動啟動（--silent）
   - 建立桌面與開始功能表捷徑
   - 移除舊版 LAN-KVM 服務與殘留 agent 並換成新檔
4. 安裝完成後會開啟設定頁面；程式常駐在**系統匣（右下角）**，不會有黑色指令視窗。

系統匣
------
- 圖示可能被 Windows 11 收在「^」裡；第一次請打開 ^，把 LAN-KVM 拖到工作列固定。
- 左鍵點圖示：開啟設定頁面。
- 右鍵選單：
  - 狀態（主控中 / 被控中 / 待機）
  - 開啟設定
  - 重新掃描
  - 停止連線 / 重新連線
  - 收集紀錄
  - 結束
- 開機時以 --silent 啟動，只顯示系統匣，不會自動開瀏覽器。

語言
----
- UI 右上角有「中文 / English」可切換。
- 預設是 English；切換成中文後會記在 app.json，系統匣選單也會一起變。
- 系統匣右鍵選單和 UI 使用同一個語言設定。

第一次配對
----------
1. 在 A 電腦開啟 LAN-KVM（桌面捷徑或系統匣）。
2. 「掃描到的電腦」會列出同網段的另一台，按「配對」。
3. B 電腦會自動開啟 UI，顯示 3 位數配對碼。
4. 在 A 輸入該號碼，按確定。
5. 配對成功後會自動連線；之後兩台開機都會自動連線，直到解除配對。

切換
----
- 在 A 的 UI 設定「對方螢幕在我的右邊/左邊」。
- A 的滑鼠推過設定的邊界 → 控制 B。
- 把游標推回 B 的進入邊界（預設越界 10 px）→ 控制權回到 A。
- B 自己的滑鼠/鍵盤不會改變連線狀態（設計如此）。
- 門檻可調：切入推力 px、時間窗 ms、回切越界 px。
- 控制 B 時，**A 的游標會消失**（全螢幕透明 overlay + 1×1 ClipCursor），
  不會再看到游標沿螢幕邊緣上下移動；回切時立即恢復。

卡住 / 緊急取回
---------------
| 熱鍵 | 作用 |
|------|------|
| `Ctrl+Alt+Shift+Q` | 正常釋放（engine 還有回應時） |
| `Ctrl+Alt+Shift+R` | **強制取回**：終止 engine、清掉游標裁切與 overlay，60 秒內不自動重連 |

- 也可以從系統匣右鍵「強制取回滑鼠」或 UI 右上角紅色按鈕。
- 熱鍵是全域的，游標被隱藏、螢幕被 overlay 蓋住時仍有效。
- B 斷線或 B 的服務停止時，A 最多 10 秒會自動取回；若主迴圈卡死，
  watchdog 會在 5 秒後直接結束 engine（游標一定回來）。
- 緊急取回：`Ctrl+Alt+Shift+Q`。

跨機剪貼簿
----------
- 任一台複製**文字 / 圖片 / 檔案**，另一台直接貼上。
- 傳輸：TCP 45875（只有 client 端監聽、master 端主動連線），雙向。
- 連線建立時，master 會先送一次目前剪貼簿，之後兩邊只要剪貼簿變更就送。
- 檔案會先複製到 `%LOCALAPPDATA%\LAN-KVM\clipboard\<時間>\`，
  再以「檔案」形式放進對方剪貼簿；上限：單次 128 MB、200 個檔案。
- 舊的暫存檔會在 2 天後自動清除。
- 回音抑制：用「自己寫入的剪貼簿序號 + 3 秒內容窗」精確忽略回音，
  不會互丟同一份內容，也不會吞掉你貼上後立刻複製的新東西。
- log（app.log）：
  clipboard listening on TCP 45875
  clipboard link up with <對方> (id)
  clipboard sent: text (12 chars) / image 640x480 (…) / files (2) [a.txt, b.txt]
  clipboard received: files (1) [b.txt] -> C:\Users\...\LAN-KVM\clipboard\<時間>
- 檔名相同、大小相同、內容相同的檔案貼上後看起來會一模一樣（例如兩台都有同一個安裝檔）。
  要驗證請用名稱不同的檔案。
- 注意：剪貼簿是「使用者工作階段」資源，**兩台都要登入 Windows**
  （B 停在登入頁時只有鍵鼠可用，剪貼簿要等登入後才會同步）。

登入畫面 / UAC
--------------
- B 的 client 注入由 SYSTEM 服務執行，所以 B 按 Win+L 或跳 UAC 時，
  仍然可以用 A 的鍵鼠操作。
- 這一版把 Winlogon 注入改成獨立 helper process（`lpDesktop = winsta0\Winlogon`），
  且在服務啟動時就預先建立、連線常駐；鎖定時只切換路由，不再有 1–2 秒卡頓。
- log 會出現 `winjector prestarted`、`winjector attached (idle)`、`winjector enabled (Winlogon)`。
- **第一次測試仍建議在 B 暫時接一個鍵盤**，若仍有問題才能解鎖/重開；確認
  `inject_fail=0` 後就可以移除。
- 相關 log：
  winjector: listening / agent connected / enabled (Winlogon)
  winjector: SendInput failed kind=... err=...
  Winlogon: input counts ... inject_fail=...
- 若卡住，A 可按 `Ctrl+Alt+Shift+Q` 強制取回（不會自動跳回，方便你測密碼欄位）。
- Num Lock / Caps Lock / Scroll Lock 已改用 virtual key 注入；數字鍵盤需 Num Lock 開啟。
- 若 UI 或系統匣顯示「client 服務未就緒」，請確認：
  sc query LAN-KVM
  服務必須是 RUNNING；必要時以管理員執行 sc start LAN-KVM。

B 先開機 / IP 變更
------------------
- B 的 SYSTEM 服務在登入頁就會讀取 App 的持久化 ID，每 3 秒廣播
  `{"type":"peer","id":...,"name":...,"control_port":45871,"data_port":45872}`。
- A 的 App 必須已登入並執行；收到同 ID 會自動更新 IP。
- 若 A 的 master 連在舊 IP，會自動 stop 後用新 IP 重連。
- log：
  B: identity announce id=... name=...
  A: peer ... IP updated old -> new
     peer address changed; master stopped for a clean reconnect
- 因此 B 先開機停在登入頁、或 DHCP 換 IP，都不需要手動改設定。

收集測試紀錄
------------
- 測試完在任一台的 UI 右上按「收集紀錄」，或用系統匣右鍵「收集紀錄」。
- 它會收集本機日誌，並透過 TCP 45873 向已配對且在線的另一台要 log。
- 會開啟一個資料夾（桌面或使用者目錄下）：
  LAN-KVM-logs-<電腦名>-<時間>
- 內含：
  app.log / engine.log / app.json / state.json
  service.log / session.log（登入畫面相關）
  system.txt（ipconfig、服務狀態、UDP、防火牆規則）
  setup.log（安裝記錄）
  peer-<對方名稱>/（對方回傳的完整日誌）
- 把這一個資料夾交給開發者即可，不用去另一台找 log。

設定與資料
----------
- 配對、名稱、門檻：%LOCALAPPDATA%\LAN-KVM\app.json
- 程式日誌：%LOCALAPPDATA%\LAN-KVM\app.log
- 連線日誌：%LOCALAPPDATA%\LAN-KVM\engine.log
- 服務日誌：C:\ProgramData\LAN-KVM\logs\service.log、session.log

解除配對 / 停止
---------------
- UI 的「解除」：兩邊都會移除配對，之後不再自動連線。
- UI 的「停止連線」：停止目前 session，60 秒內不自動重連。
- 系統匣右鍵「結束」：停止 session 並結束常駐程式（服務仍在背景）。

移除程式
--------
- 用 Windows「設定 → 應用程式 → 已安裝的應用程式」找到 **LAN-KVM 1.1.7.0** 按解除安裝，
  或執行開始功能表的「解除安裝 LAN-KVM」。
- 解除安裝會自動：結束常駐程式、移除 SYSTEM 服務、刪除兩個開機排程、
  刪除防火牆規則、移除登入自動啟動。
- 保留（可手動刪除）：
  %LOCALAPPDATA%\LAN-KVM\app.json（配對與設定）
  C:\ProgramData\LAN-KVM\config.txt、logs\（診斷用）

> 舊版（`0.3-clip3` 以前的自製 setup）沒有標準卸載流程，請先安裝新版（1.1.x）後再解除安裝。

目前限制
--------
- P0 傳輸是未加密 UDP；只可在信任的區域網路使用。
- 配對碼只有 3 位數，沒有防止暴力嘗試；正式版會換成 SPAKE2/憑證。
- 程式與服務未簽章，McAfee/Apex One 可能隔離；必要時請 IT 對檔案 hash 加白名單。
- 僅支援 Windows。
- 系統匣圖示由 Windows 管理；若在 ^ 裡，請手動拖到工作列。

疑難排解
--------
- 掃不到對方：確認同網段、兩台都開著、AP 沒有 client isolation。
- 配對碼視窗沒出現：在 B 開啟 LAN-KVM。
- 連線中斷：系統匣右鍵「收集紀錄」。
- **貼上檔案卻是上一個檔案**：已在 `-clip3` 之後的版本修掉（寫入前沒有清空剪貼簿，
  Explorer 貼上時用到舊的 Shell IDList）。請確認兩台都是 `1.1.x`。
  驗證時用檔名不同的檔案，並看 app.log 的 `clipboard received: files (1) [檔名]`。
- 登入畫面無法控制：確認 sc query LAN-KVM 是 RUNNING，且 service.log 有
  Winlogon: injector active。
- A 完全看不到 B（B 在登入頁）：
  1. B 登入頁的 Wi-Fi/網路必須已連上（登入頁就能上網）。
  2. 登入 B 後 `sc query LAN-KVM` 必須 RUNNING。
  3. `service.log` 要有 `identity announce`、`started session agent`；
     `session.log` 要有 `session agent started`、`winjector prestarted`。
  4. 若服務不是 RUNNING 或沒有上述 log，重新執行 `LAN-KVM-Setup-1.1.9.exe`。
- 安裝失敗：看 %TEMP%\lankvm-setup.log。
