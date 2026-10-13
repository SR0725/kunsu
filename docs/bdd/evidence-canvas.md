# AI 證據畫布與草稿註冊（evidence-canvas）— BDD 驗收規格

> 狀態：**已實作，待 QA**（2026-10-08 Ray 拍板：直接採用 Claude Artifact 模式；2026-10-09 QA 第 5 輪修正：HTML 組裝改用
> tokenizer、畫布框不能被導走、runtime 載入失敗不算卡住、加固腳本補字串解析並寫明已知繞過；2026-10-09 QA 第 6 輪修正：
> Kunsu 的資料區塊與 runtime 移到畫布 HTML 之前〔結構性修法，不再靠 tokenizer 切對〕、文件沒載入完成回 `canvas_incomplete`、
> 組裝保證畫布 HTML 原始碼開不出 `<link>`、HTML 靜態檢查先解碼字元參照）。工程契約見 `docs/CONTRACT.md` §12。
> 產品依據：`docs/PRD.md` §2 原則 6（修訂）、§7.2（修訂）、§24；視覺依據：`DESIGN.md`「Evidence canvas」。
> 先例：Claude Artifacts（React＋Tailwind＋Recharts 在沙盒 iframe）、MCP Apps（沙盒 iframe UI）。
> 不變量：INV-20 `inv_canvas_sandboxed`（畫布無網路、無 IPC、不能送出；UI 與殼層各有同名測試）、INV-21
> `inv_controls_native`（題目、選項、送出、錢參數永遠由 Kunsu 原生呈現）。
> 界外：畫布即時連網取數、畫布與 Kunsu 雙向互動、在畫布內作答。

## 背景

AI 需要的證據形式每題都不同：有時是圖表，有時是素材圖，有時什麼都不需要。自創圖表語法要 AI 學新語言、
彈性又不夠。改採 Claude Artifact 模式：文字、表格、圖片用 Markdown；要畫圖就寫一個 React（或 HTML）畫布，
在 Kunsu 的沙盒裡執行。AI 先寫草稿檔，再註冊；Kunsu 檢查並真的渲染一次，有問題逐行回報讓 AI 修正。

## 草稿格式

```
~/.kunsu/drafts/<question_id>/
  question.md     # YAML frontmatter（題目欄位＋why）＋ Markdown 內文（證據）
  canvas.tsx      # 可選；或 canvas.jsx／canvas.html，最多一個
  assets/…        # 可選；內文或畫布以 ./assets/x.png 引用
```

frontmatter 欄位與 `ask_create` 相同（id、title、question_line、kind、options、money…），另加 `why`（Markdown）。
畫布 `canvas.tsx` 以 default export 一個 React 元件，可 import：`react`、`recharts`、`lucide-react`、`kunsu`
（Kunsu 元件與色票）；可用 Tailwind class。

## 行為規格

### 場景 1：註冊成功即進入佇列

**Given** 草稿資料夾內容合法
**When** agent 執行 `kunsu ask <草稿資料夾>` 或呼叫 MCP `ask_register {draft_dir}`
**Then** Kunsu 依序完成檔案、frontmatter、欄位規則、Markdown 安全、畫布靜態檢查與沙盒渲染檢查；全部通過後
建立題目（開始 15 分鐘週期），回傳 `{ok: true, question_id, revision, preview_png}`；`preview_png` 是使用者會看到
的面板截圖，讓 agent 自己檢查畫面

### 場景 2：有問題就逐行回報

**Given** 草稿有任何錯誤（YAML 格式、題目超過 24 字、選項數量、Markdown 含禁用標籤或外部圖片〔含參照式與跨行
`<img>`〕、TSX 語法錯、import 不在允許清單、畫布試圖換頁〔`http-equiv`〕、畫布試圖連網路〔含 WebRTC 與
`<link rel=preconnect／dns-prefetch／preload／prefetch>`〕、執行期錯誤、2 秒內沒畫完〔與正式顯示同一時限〕、內容高度
超過 2400 px、橫向溢出）
**When** 註冊
**Then** 不建立或更新題目；回傳 `{ok: false, issues: [{file, line, column, code, message, hint}]}`，每個問題都指出檔案、
位置與怎麼改；agent 修改草稿後重新註冊。HTML 畫布（`canvas.html`）一樣：靜態檢查在註冊檢查裡真的對 HTML 原始碼執行
（`network_api`、`external_url` 指到 `canvas.html` 的行號，先解碼字元參照）；畫布 HTML 裡的 `<script>` 在解析過程中出錯
（含語法錯誤與未處理的 Promise 拒絕）也回報 `canvas_runtime`（帶 `canvas.html` 的行號與瀏覽器的原始訊息，例如
`ReferenceError: Can't find variable: data`，不是看不出原因的 `Script error.`），不會回 `ready`。畫布 HTML 的 `<script>`、
`<style>`、註解、屬性值或字串裡提到 `<link`、`<meta http-equiv`、`</script` 時（例如 `// 註：沙盒不能用 <link 載入樣式`），
組裝不會把它們當標籤處理：腳本照常轉成 data: 腳本、帶正確的 `canvas.html` 行號，畫布的錯誤照實回報，不會變成 2 秒逾時。
Kunsu 的資料區塊、加固腳本與 runtime 放在畫布 HTML 之前、先執行，畫布 HTML 是沙盒文件的最後一段：畫布 HTML 怎麼寫——忘了關
的註解、`<script>`、`<style>`、`<textarea>`、`<template>`、`<svg>`／`<math>`（含裡面對不上的結束標籤與 integration point）、
`<plaintext>`、`<select>` 裡的 `<style/>`、`<frameset>` 或沒寫完的標籤——都吞不掉它們，畫布照常回 `ready` 或照實的錯誤。
畫布 HTML 讓文件沒在時限內載入完成、但主執行緒沒卡住時（例如呼叫 `window.stop()`、`document.open()` 沒有 `close()`），回報
`canvas_incomplete`（「畫布 HTML 沒有載入完成」＋怎麼改），不是提示查無窮迴圈的 `canvas_timeout`；真的卡住主執行緒才是
`canvas_timeout`

### 場景 3：只預覽不送出

**Given** agent 想先看效果
**When** `kunsu ask <資料夾> --dry-run`（或 `ask_register {draft_dir, dry_run: true}`）
**Then** 執行完整檢查並回傳 issues 與 `preview_png`，但不建立題目、不通知、不開始倒數

### 場景 4：草稿可以再編輯，重新註冊會更新

**Given** 題目已註冊、尚未結案，AI 或使用者修改了草稿檔
**When** 以同一 id 重新註冊
**Then** 通過檢查後題目內容更新為新版本（revision + 1），預設動作也換成新草稿的（並以新選項驗證），保留週期時間戳
與 `cycle_number`、不重發通知；未重新註冊的草稿修改不會出現在面板；草稿題不能用 `ask_update` 或 JSON 重送改內容；
已結案的題目拒絕更新並回傳（去敏的）既有答案；每次檢查都用這次的內容，不沿用上一次的畫布。同一題同時有兩次重新
註冊時，不同內容一定拿到不同 revision；已存在的 revision 快照永不被覆寫（先寫暫存資料夾、確認題目未結案後才放進去）

### 場景 5：版本可追溯

**Given** 題目有多個版本
**When** 使用者回答
**Then** 答案綁定當下顯示的 revision 與內容雜湊；從面板作答一定要帶看到的 revision 與內容雜湊（沒帶或任一不符都拒收）；
使用者作答那一版的快照內容永遠是他看到的那一版；ledger 快照
含 revision；回看歷史時顯示使用者當時看到的那一版

### 場景 6：畫布在沙盒中執行

**Given** 題目有畫布
**When** 面板顯示畫布
**Then** 畫布在 `sandbox="allow-scripts"` 的 iframe 中執行（不具同源權限），CSP 禁止所有網路連線、表單、彈出視窗與
上層導覽；所有 Kunsu 視窗的主框架只能導覽到 app 自己的頁面、`about:srcdoc`、`about:blank`、`blob:`，畫布框（任何
子框架）只能停在 `about:srcdoc`、`about:blank`（含同文件片段）與 `blob:`——畫布換頁到外部網址、`data:` 或 Kunsu 自己的
頁面（例如一般的 `<a href="#table">`、`href="#"`、`location.href = 'index.html'`）一律被擋或不會導覽；畫布取不到 Tauri IPC、主視窗 DOM 或任何 Kunsu 資料；Kunsu 只接受該 iframe 送來的
`resize`、`ready`、`error` 三種訊息，其餘忽略。所有 Kunsu webview 都在引擎設定關閉 WebRTC：畫布文件、HTML 畫布、畫布自己開的 srcdoc
子框架或 `blob:` 文件都沒有 `RTCPeerConnection`（沙盒 runtime 另外移除建構子作縱深防禦）；TSX 與 HTML 畫布的註冊
檢查都把 WebRTC 列為 `network_api`；畫布拿不到「現在是檢查還是正式顯示」，兩者行為一致。
CSP 管不到的資源提示也在引擎設定關閉：畫布用 DOM API、JSX 或 HTML 加 `<link rel=preconnect／dns-prefetch／preload／
prefetch>`（含主機名稱，需要 DNS 的）都不會發出任何 TCP 連線或 DNS 查詢，畫布其餘部分照常畫完。**保證在引擎設定**；以下
是縱深防禦：註冊的靜態檢查把這些 `rel` 列為 `network_api`（HTML 畫布先解碼字元參照，`rel="&#112;reconnect"` 一樣抓得到；
逐行文字比對，跨行或執行期組出來的值抓不到）；HTML 畫布組裝沙盒文件時拿掉 `<link>` 標籤，並把其餘每個字面 `<link` 改成
`&lt;link`——畫布 HTML 原始碼不論瀏覽器怎麼切（例如 `<svg></b><title><b><link rel=preconnect>`）、拿掉標籤後怎麼拼接，都開
不出 `<link>` 元素；沙盒 runtime（HTML 畫布是畫布 HTML 之前的加固腳本）擋下以 DOM API 把 `rel` 設成這些值，以及字串解析
產生的 `<link>`（`innerHTML` 等、`DOMParser` 的 HTML 與 XML〔含命名空間前綴 `<h:link>`〕、分成兩次的 `document.write('<li')`
＋`('nk …>')`），並移除 `XSLTProcessor`；加固腳本只是縱深防禦，CONTRACT §12 照實列出已知繞過（例如畫布自己開的子框架）。
引擎設定在每個 Kunsu webview（main、capsule、overlay、canvas-check）建立後立刻套用，卡住後終止網頁行程、重新載入的新行程
同樣生效

### 場景 6b：畫布框不能被導走

**Given** 畫布裡有同頁連結（`<a href="#table">`、`href="#"`）、相對連結（`index.html`、`?view=main`）或程式換頁
（`location.href = …`），或畫布把自己導到 `about:blank`、`blob:` 文件、重新載入
**When** 使用者點了連結或畫布執行到這段程式
**Then** `#id` 連結在畫布內捲動到該元素、`#` 回到畫布頂端，畫布本身不換；相對網址解析失敗、不會導覽（程式換頁丟出的錯誤照
一般執行期錯誤回報）；導到 Kunsu 自己的頁面（含絕對網址 `tauri://localhost/…`）、`data:` 或任何網址都被殼層擋下，iframe
留在原本的畫布，Kunsu 的頁面永遠不會載入到畫布框裡；畫布把自己換成 `about:blank`、`blob:` 文件或重新載入時，面板隱藏畫布、
顯示「圖表載入失敗：畫布換頁了」並記一次 `canvas_failed`（註冊檢查回 `canvas_navigation`），不會留下無聲的白框

### 場景 7：決策控制永遠由 Kunsu 原生呈現

**Given** 畫布內容任意（包括畫出看起來像按鈕的元素）
**When** 使用者作答
**Then** 題目文字、選項、補充說明、送出、稍後再問與錢題的所見即所簽參數只存在於 Kunsu 原生介面；畫布內任何操作
都不能產生答案、批准或改變參數；畫布區域有「AI 提供的資料」標示與一致的外框

### 場景 8：圖片與資產

**Given** 內文、why 或畫布引用 `./assets/` 下的圖片
**When** 註冊
**Then** 圖片被複製進該 revision 的快照並以 data URI 提供（why 裡的圖片同樣顯示）；單張 ≤ 2 MB、合計 ≤ 8 MB；引用外部網址、不存在的檔案、
`assets/` 本身或其中檔案經 symlink 指到草稿外、或超過上限都列為 issue。所有會被渲染成 Markdown 的欄位（內文、why、
證據文字、選項文字與補充說明）都做同樣的檢查，`<image>`、屬性值含 `>` 的標籤與跨行的 `srcset` 也算；
`<picture>`／`<source srcset>` 等其他載圖方式不被 sanitizer 允許。面板
顯示草稿題時，任何欄位都只載入快照圖片，外部圖片一律不載入（不發出任何對外請求）；預覽截圖不跟著 symlink 寫到草稿
資料夾外

### 場景 9：畫布失敗不擋決定

**Given** 正式顯示時畫布出錯或 2 秒內未回報 ready
**When** 面板顯示
**Then** 隱藏畫布，顯示「圖表載入失敗：原因」與 Markdown 內容；題目、選項與送出完全可用；記錄診斷事件
（`canvas_failed`，每個題目／版本／顯示只記一次；runtime 本身載入失敗也記）；畫完之後才發生的錯誤也一樣處理；
HTML 畫布的 `<script>` 在解析過程中出錯、或文件沒在時限內載入完成（`canvas_incomplete`）也一樣。沙盒 runtime 本身讀不到時，顯示「圖表載入失敗：」＋讀取失敗的原因，`canvas_failed`
只記這一次，不會被當成畫布卡住（不終止網頁行程、不重新載入、不停用這一版，使用者正在打的補充說明不會消失）——面板在 runtime
載入成功、真的要執行畫布時才向殼層登記。面板執行畫布前要先得到殼層許可：許可以「題目＋版本」為準（換版不沿用上一版的
許可，新版拿到許可前不執行）；問殼層的 IPC 失敗就當作不許可，顯示「圖表載入失敗：無法確認畫布可以執行」並記 `canvas_failed`

### 場景 9b：畫布卡住主執行緒

**Given** 畫布在顯示後做長時間同步運算或無窮迴圈，面板的 JavaScript 停止回應
**When** 原生殼連續 3 秒收不到面板的心跳回應
**Then** 殼層把該題該版本的畫布停用、記 `canvas_failed`（原因「畫布沒有回應」），終止那個視窗卡住的網頁行程後
重新載入（只影響那個視窗）；蓋板先隱藏，重新載入的網頁掛載後才再顯示，而且只在卡住當下蓋板還開著時才重新呈現——
使用者已按 Esc、勿擾或 kill-switch 收掉的不會被叫回來。卡住期間使用者按的 Esc（網頁沒反應、原生 2 秒後備還沒到期，
看門狗就先判定卡住）不會被吞：殼層照樣完成這次 Esc（記 dismissal、開始新的完整週期）並隱藏蓋板，不再重新呈現。
原生完成的 Esc 和網頁的「稍後再問」範圍相同：所有非 internal 的強制題，加上網頁最後回報的目前題目（還沒結案時），
每一題都記 dismissal——不會只處理最早逾時的一題、讓剩下的強制題又把蓋板叫回來（2026-10-09 第 5 輪 QA，見
overlay-presentation 場景 6）。
停用的版本在任何視窗都不再執行（面板執行畫布前先問殼層），
顯示「圖表載入失敗：畫布沒有回應」，題目、選項與送出可用。
看門狗只判定還開著、而且還是同一個網頁的視窗：視窗被關閉（例如主視窗按 ⌘W）或網頁開始重新載入時，殼層立刻清掉
那個視窗的畫布追蹤，不會因此誤判卡住、停用該版或記假的 `canvas_failed`；重新載入後的面板要再問一次殼層才執行畫布。
註冊檢查時畫布卡住，檢查逾時後先終止檢查視窗的網頁行程再銷毀視窗（不留下持續耗 CPU 的行程），回報
`canvas_timeout`（提示檢查無窮迴圈或過重的計算），不是「Kunsu 檢查器沒有啟動」。每一筆檢查都用新的檢查視窗與網頁
行程：定案（截圖完成或逾時）後就終止網頁行程並銷毀視窗，畫布一併卸下；「畫完才卡住」的畫布不會在背景持續耗 CPU，
也不會讓下一筆註冊逾時。
已知限制：每 3 秒內會讓出一次主執行緒的畫布不會被判定卡住（Esc 的原生後備照常有效）

### 場景 10：外觀一致

**Given** agent 使用 template 與 `kunsu` 模組
**When** 畫布渲染
**Then** 背景透明、使用 Kunsu 色票（CSS 變數）、Recharts 預設主題（2 px 線、髮絲格線、驗證過的類別色）、`Stat`／
`Panel` 元件，看起來與 Kunsu 原生介面一致；字型與顏色不需 agent 自己挑

### 場景 11：Markdown 內文安全

**Given** 內文含 Markdown、表格、圖片或 HTML
**When** 渲染
**Then** 以既有 allowlist sanitizer 處理（禁 script、style、iframe、form、input、事件處理器、任意 class 與定位）；連結
第一次點只提示「再按一次用瀏覽器開啟 網域」，4 秒內再點才用系統瀏覽器開；只開 http(s)。沒有可開網址的連結（例如
`javascript:` 被 sanitizer 拿掉 href 後）點了沒有反應，也永遠不顯示這個提示

### 場景 12：舊的建題方式仍可用

**Given** agent 直接以 JSON 呼叫 `ask_create`（沒有草稿）
**When** 建題
**Then** 行為與現行相同；可另帶 `why_md`；沒有畫布

### 場景 13：隱私

**Given** 任何 guard 或勿擾
**When** 題目存在
**Then** 畫布與證據只出現在強制面板與主 app，絕不出現在系統通知或瀏海跑馬燈

### 場景 14：上下文成本有限

**Given** agent 第一次需要建題
**When** 讀 kunsu skill
**Then** 主要說明只描述草稿格式、註冊指令與錯誤修正迴圈；template 與範例放在獨立檔案，需要畫圖時才讀

## 驗收清單

- [ ] AC-EC-01：合法草稿註冊成功、開始週期，回傳 question_id／revision／preview_png
- [ ] AC-EC-02：所有檢查失敗都不建立題目，回傳含 file／line／column／code／message／hint 的 issues；HTML 畫布的靜態檢查
  在註冊檢查真的執行（先解碼字元參照），畫布 HTML 的 `<script>` 錯誤回 `canvas_runtime`（`canvas.html` 行號），不回 ready；`<script>`／
  `<style>`／註解／字串裡的 `<link`、`<meta http-equiv`、`</script` 不被當成標籤（腳本照常轉換、行號正確、錯誤照實回報）；
  資料區塊、加固腳本與 runtime 在畫布 HTML 之前，畫布 HTML 之後沒有任何 Kunsu 的東西——QA 第 6 輪的 8 種寫法（沒關的
  `<template>` 接 `</`、`<svg>`／`<math>` 裡對不上的結束標籤、integration point 後的 `<plaintext>`、`<select><STYLE/><script>`）
  與任何沒關的結構都吞不掉它們，畫布照常回 `ready` 或照實的錯誤；文件沒在時限內載入完成但主執行緒沒卡住時回
  `canvas_incomplete`，不是 `canvas_timeout`
- [ ] AC-EC-03：dry-run 完整檢查與預覽，不建立題目
- [ ] AC-EC-04：同 id 重新註冊更新內容為新 revision，不重設週期、不重發通知；已結案拒絕；併發時 revision 不重複、
  既有快照不被覆寫
- [ ] AC-EC-05：答案綁定 revision 與內容雜湊（面板兩者都要帶且相符），作答那一版快照不變，ledger 可追溯
- [ ] AC-EC-06：畫布在 allow-scripts 沙盒＋禁網路 CSP 中執行，無 IPC，只接受 resize／ready／error；WebRTC 與資源提示
  （preconnect、dns-prefetch、preload、prefetch）在每個 webview 建立後以引擎設定關閉（含 HTML 畫布、srcdoc 子框架、blob
  文件、終止行程後重新載入的新行程），畫布用 DOM API、JSX、HTML 三種寫法都沒有任何對外連線或 DNS 查詢——**引擎設定是保證**，
  以下是縱深防禦：TSX 與 HTML 檢查都回報 `network_api`（HTML 先解碼數字與具名字元參照）；HTML 畫布組裝時拿掉 `<link>`，
  其餘字面 `<link` 一律改成 `&lt;link`，畫布 HTML 原始碼不論怎麼切都開不出 `<link>` 元素；畫布拿不到檢查／顯示模式；加固腳本
  另擋 DOM API、字串解析（含 XML 命名空間前綴）、分段 `document.write` 與 `XSLTProcessor`，CONTRACT §12 不宣稱靜態檢查或
  加固腳本擋得住全部，照實列出已知繞過
- [ ] AC-EC-06b：畫布框只能停在 `about:srcdoc`、`about:blank`（含同文件片段）與 `blob:`；`#id`／`#` 連結在畫布內捲動，相對
  網址不導覽；導到 Kunsu 自己的頁面、`data:` 或任何網址被殼層擋下（依 WebKit 的目標框架判斷，主框架白名單不變）；畫布把
  自己換掉時隱藏並顯示「圖表載入失敗：畫布換頁了」、記一次 `canvas_failed`（檢查回 `canvas_navigation`）
- [ ] AC-EC-07：題目、選項、送出與錢參數只在原生介面；畫布不能作答或改參數；畫布有來源標示
- [ ] AC-EC-08：assets 複製進快照為 data URI（含 why），大小上限；所有 Markdown 欄位的外部圖片在註冊時擋（含
  `<image>`、屬性值含 `>`、跨行 srcset）、在面板不載入；sanitizer 不允許 picture／source
- [ ] AC-EC-09：畫布錯誤或 2 秒未 ready 時隱藏並顯示原因，決定完全可用；`canvas_failed` 每次顯示只記一次；執行許可以
  題目＋版本為準（換版不沿用），問殼層失敗視同不許可；runtime 讀不到時顯示讀取失敗的原因、只記一次 `canvas_failed`，
  runtime 載入成功前不向殼層登記（看門狗不追蹤、不誤判卡住）
- [ ] AC-EC-09b：畫布卡住主執行緒 3 秒內被偵測，停用該版本畫布（任何視窗都不再執行），終止卡住的網頁行程並重新
  載入；只在卡住當下蓋板還開著時重新呈現；卡住期間按的 Esc 照樣完成 dismissal、不重新呈現；視窗關閉或重新載入時
  清掉追蹤、不誤判；檢查逾時回 `canvas_timeout`；每筆檢查定案後終止檢查視窗的網頁行程再銷毀，下一筆用新視窗
- [ ] AC-EC-10：template＋`kunsu` 模組產出一致外觀
- [ ] AC-EC-11：Markdown 內文沿用 allowlist sanitizer；沒有可開網址的連結永不顯示「再按一次」提示
- [ ] AC-EC-12：JSON `ask_create` 相容並支援 `why_md`
- [ ] AC-EC-13：證據與畫布不出現在通知或瀏海
- [ ] AC-EC-14：skill 主文只含格式與迴圈，template／範例按需載入

## 測試對應

- Rust：草稿讀取與 frontmatter 解析、路徑必須在 drafts 目錄內、issues 位置正確、revision 與 upsert 語意、資產
  上限、ledger 快照含 revision、MCP `ask_register` 與 CLI `--dry-run`。
- UI（vitest）：畫布 runtime 的 import 允許清單、靜態禁用 API 檢查（含資源提示 `rel`、HTML 先解碼字元參照）、srcdoc 與 CSP
  組裝（資料區塊、加固腳本、runtime 在畫布 HTML 之前且畫布 HTML 之後沒有 Kunsu 的東西、`<link>` 拿掉與字面 `<link` 一律中和、
  行號偏移、`<base href="about:srcdoc">`）、HTML tokenizer（script data 的 escaped／double escaped、raw text、註解、外來
  內容）、parse5 差分測試（隨機畫布 HTML 組裝後，規格 parser 看到的資料區塊與 runtime 都是畫布之前的 HTML script、沒有 HTML
  `<link>` 元素、行號不變）、訊息白名單、失敗 fallback、HTML 畫布錯誤收集、lint 與 `canvas_incomplete`、加固腳本安裝時記下
  token、執行許可以題目＋版本為準、runtime 載入成功才向殼層登記、畫布換頁回報 `canvas_navigation`、加固腳本的 XML 前綴／
  分段 write／XSLT、沒有 href 的連結。
- Rust（殼層）：`inv_canvas_sandboxed`（導覽白名單分主框架與子框架；引擎關閉清單與子框架關卡；每個 webview 都經同一個
  建立路徑套用）、看門狗忘掉
  關閉／重新載入的視窗、`inv_overlay_never_blank` 窮舉含「卡住期間按 Esc」、檢查視窗每筆定案後拆掉。
- 整合：shell 離屏沙盒渲染檢查回報編譯錯誤行號與執行期錯誤；預覽截圖產出。
- 原生實驗（真 WKWebView，依產品時序建立後才套用設定）：preconnect／dns-prefetch 三種寫法沒有連線、畫布照常；終止行程
  後重新載入仍生效；檢查視窗終止網頁行程再銷毀後不再耗 CPU；HTML 畫布早期錯誤的行號；註解提到 `<link` 的 HTML 畫布照實
  回報錯誤；畫布框的 `#id`／相對連結／程式換頁（含 wry 的 navigation delegate 加上子框架關卡後）；加固腳本對 XML 前綴、
  分段 write、XSLT 的效果（關掉引擎設定作對照）；QA 第 6 輪的 8 種畫布 HTML 照常回 ready、`<svg></b><title><b><link …>` 與
  字元參照的 `rel` 不再留下 `HTMLLinkElement`（關掉引擎設定作對照也沒有連線）、`window.stop()`／`document.open()` 回
  `canvas_incomplete`、真正的無窮迴圈仍是 2 秒逾時。
- 手動：以 template 實作四種範例（廣告、預算、素材、日記）視覺比對 prototype。
