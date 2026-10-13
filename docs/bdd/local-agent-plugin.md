# 本機 Agent 外掛與正式 icon

狀態：**Current，2026-10-09 本機打包／安裝與 QA 通過。證據留在本機 QA 紀錄，不進 repo。**

後續 0.2.0 完整維護流程與相同 transport 回歸證據同樣只留在本機。

## 範圍

把既有 Kunsu MCP 與建題 SOP 包成 Codex 本機外掛；原 API、題目狀態、guard、排程與 Claude HTTP 接線不變。採用作者選定的原始 K 圖。界外：新排程、公開／雲端發佈、代答、批准、UI layout 或動效變更。

## 場景與 acceptance criteria

1. **AC-LP-01 — 憑證與 transport**（Python integration）
   Given 本機 token 檔存在，When stdio 收到 initialize／tools/list／唯讀 tools/call，Then 只送往固定 `127.0.0.1:4777/mcp`，從檔案帶 Bearer，stdout 只有 JSON-RPC 回覆；不使用系統 proxy、不跟隨重新導向。下一次呼叫重新讀 token，支援輪換。
2. **AC-LP-02 — 作答邊界**（Python unit/integration）
   Given Agent 透過外掛呼叫工具，When 工具為 answer／approve 或未知 method，Then 不送到上游並回 JSON-RPC error；工具清單只保留 CONTRACT §4 的 13 tools。
3. **AC-LP-03 — 失敗與復原**（Python integration）
   Given token 缺失、空白或 app 未開，When 呼叫工具，Then 回可操作的錯誤且不洩漏 token／請求內容；恢復檔案與 app 後下一次呼叫正常。無 id notification 不回覆；格式錯誤一行不終止後續合法請求。
4. **AC-LP-04 — 原對話與日常督導**（skill/package QA＋既有 continuity/routine tests）
   Given Claude／Codex 發問，When 技能建題，Then 使用真實來源對話 id 與 cwd、草稿先 dry-run 並查看預覽；Codex 建題後交還控制權，Claude 背景 wait；回應後 ask_report。排程只有穩定 routine id／cadence，工具不臆造排程或自動勿擾。
5. **AC-LP-05 — 可安裝且不重複**（package unit＋host integration）
   Given personal marketplace 已有其他外掛與舊 Codex kunsu MCP，When 安裝新外掛，Then 保留其他項目、manifest 的 stdio command 使用宿主允許的 bare executable（python3），Codex 列出 kunsu 為 installed/enabled，舊獨立 kunsu MCP 停用以免重複；外掛 server 命名 kunsu-inbox，避免宿主把舊 server 的 enabled=false 套到外掛。doctor 接受 enabled 外掛或 enabled 直接 MCP，兩者都停用時失敗。備份舊設定，Claude HTTP 註冊不動。
6. **AC-LP-06 — 指定原圖**（asset integration＋native QA）
   Given 作者選定的原圖，When 打包，Then master SHA-256 等於 `476b1cd31f845540ebd010642c0bf5ab4d2365c14a7fc36ef3d5ae1ae9025f84`，PNG 為指定尺寸、有透明背景且非空，ICNS 可解碼；menu-bar template hash 保持一致，外掛／favicon 圖與 native icon 來自同 master。原生 Finder／bundle readback 確認已安裝新 icon。
7. **AC-LP-07 — 本機交付**（smoke＋native QA）
   Given targeted tests 通過，When cargo test 與 scripts/build-install.sh 完成，Then doctor 通過、app health 正常、原生 icon 可透過 Finder 鍵盤流程查看，沒有新增提醒浮窗；外掛新行程能 initialize、發現 13 tools、呼叫 dnd_get。取消／回復安裝只需停用外掛與還原已備份設定、app bundle。沒有改動決策 UI，沿用本次完整 UI suite 的 sanitizer／reduced-motion／long-content／guards 回歸證據。

8. **AC-LP-08 — Codex runtime 相容性**（Rust unit＋host smoke）
   Given 系統 CLI 與桌面版版本不同，When Kunsu 選擇 Codex 執行檔，Then resume／fresh 使用同一 resolver：測試指定路徑優先，其次可執行的桌面版內建 CLI，再其次 PATH；檔案不存在、是資料夾或無執行權限時不可選。保持目前模型設定，桌面版真實 MCP 唯讀呼叫可成功。只對固定本機 Kunsu 外掛工具預先允許呼叫，不改其他外掛批准設定；作答／批准仍不存在於 MCP。

## QA checklist（取得證據後才勾選）

- [x] AC-LP-01
- [x] AC-LP-02
- [x] AC-LP-03
- [x] AC-LP-04
- [x] AC-LP-05
- [x] AC-LP-06
- [x] AC-LP-07
- [x] AC-LP-08

由目前 Agent 做隔離第二輪稽核；未啟動獨立 QA Agent。
