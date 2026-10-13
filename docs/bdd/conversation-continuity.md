# 回答延續原對話（conversation-continuity）— BDD 驗收規格

> 狀態：**已實作，待 QA（2026-10-08）**：core 交付階梯、server deliveries／等待者租約／resume／held／fresh、`ask_report`、
> `kunsu ask` 身分補齊、`kunsu wait --timeout 0`、回執交付狀態與打開原對話。Claude App／Codex App 畫面是否看到追加的一輪仍需手動驗證。
> 產品依據：`docs/PRD.md` §20；取代 `harness-roundtrip.md` 場景 8（resume 判斷）與場景 12（Monitor 預武裝）。
> 不變量：INV-18 `inv_single_continuation`、INV-19 `inv_never_resume_live_conversation`；INV-7 延伸到 `ask_report`。
> 界外：跨 session 訊息 socket 喚醒（需保存 session 秘密，另案評估）、Kunsu 內建聊天（紅線）。

## 背景（2026-10-08 實機研究）

使用者的典型流程：Claude 每天排程復盤廣告 → 卡住用 Kunsu 問 → 使用者回答。現行 reactor 只會開新的
`claude -p`／`codex exec`，等於另開一段沒人看的新對話；原本那段 Claude App／Codex App 對話停在原地。

已驗證的事實：

- Claude：每個活著的 session 在 `~/.claude/sessions/<pid>.json` 登記（行程結束即刪）；session 內可讀
  `CLAUDE_CODE_SESSION_ID`、`CLAUDE_CODE_HOST_SESSION_ID`（桌面 `local_…`）、`CLAUDE_CODE_SESSION_ATTENDED`、
  `CLAUDE_CODE_EXECPATH`。`claude -p --resume <id>` 沿用同一個 id、接在同一份 transcript 後面；但原 session
  還開著時這樣做會產生分岔，原畫面看不到、之後還會被覆蓋。桌面 app 可用
  `claude://code/continue?session=local_<uuid>` 打開該對話；桌面 session 的 CLI id 會變（`priorCliSessionIds`），
  必須在作答當下重新查。排程任務跑完行程就結束；背景 Bash 在無人值守 session 最長 2 小時。
- Codex：Codex App（`/Applications/ChatGPT.app`，`com.openai.codex`）與 CLI 共用 `~/.codex`；agent 環境有
  `CODEX_THREAD_ID`；`codex exec resume <thread> -` 接在同一個 thread 後面，但 app 正在寫的 thread 會被鎖
  （`already has an active writer`）；`codex://threads/<id>` 可打開 thread；heartbeat automation 每次都回到同一
  thread，cron automation 每次開新 thread。
- 現行 reactor 的 resume 從未觸發（用 session id 找 `<pid>.json` 檔名），而且判斷方向相反（只對活著的
  session resume，正好會分岔）。MCP 走 HTTP，server 看不到呼叫端環境變數。

## 行為規格

### 場景 1：`kunsu ask` 自動帶上原對話身分

**Given** agent 在 Claude 桌面 session（含排程任務）或 Claude CLI 內，以 `kunsu ask` 建題
**When** CLI 送出 `ask_create`
**Then** `source` 自動補上 `host`（claude_desktop｜claude_cli）、`conversation_id`（CLI session id）、
`host_session_id`（桌面 `local_…`，有才帶）、`attended`、`cwd`、`cli_path`；agent 自填的值優先；不讀取、
不傳送任何 messaging token 或其他秘密

### 場景 2：Codex 建題帶 thread 身分

**Given** agent 在 Codex App 或 Codex CLI 內，以 `kunsu ask` 建題
**When** CLI 送出 `ask_create`
**Then** `source` 帶 `host`（codex_app｜codex_cli）、`conversation_id = CODEX_THREAD_ID`、`cwd`；只用 MCP 建題的
agent 可自行傳入同名欄位，server 以同一 schema 驗證；Codex 的 `conversation_id` 必須是 UUID（非 UUID 會被
`codex exec resume` 當成 thread 名稱去找，可能接到別的對話）

### 場景 3：沒有原對話身分時維持現行行為

**Given** 題目沒有 `conversation_id`（舊 client、其他 agent、doctor）
**When** 題目被回答
**Then** 依原 `wake_on_answer` 行為處理（fresh／none），不嘗試 resume；回執說明「沒有原對話可接續」

### 場景 4：原對話正在等 → 直接交給它

**Given** 原對話建題後在背景執行 `kunsu wait <id> --timeout 0`（Claude 用 background Bash；每輪 ask_wait ≤ 540 s，
無總時限地迴圈）
**When** 使用者在 Kunsu 送出回答
**Then** 答案立即交給該等待者，`kunsu wait` 印出答案 JSON 並以 0 結束，原對話在 Claude App 同一段對話中被
喚醒接著做；Kunsu 記 `delivery = live_waiter` 並標 `acked_at`；reactor 不再 resume 或開新對話

### 場景 5：等待者在兩輪之間仍算在等

**Given** 等待者剛結束一輪 `pending_timeout`、正要發下一輪
**When** 回答剛好在這 10 秒空檔送出
**Then** 視為仍有等待者（10 秒租約），答案保留給下一輪 ask_wait 取走，不觸發 resume；租約過期仍沒人取走才
進入下一階

### 場景 6：原對話已結束 → 在同一段對話追加一輪

**Given** 排程任務已跑完（行程已結束），題目有 `conversation_id`
**When** 使用者回答
**Then** reactor 在原對話追加一輪：Claude 用 `cli_path`（或 `which claude`）執行 `-p --resume <最新 CLI id>`，
Codex 執行 `codex exec resume <thread> -`；工作目錄是 `source.cwd`，清除繼承的 `CLAUDE_CODE_*` 環境變數，
其餘旗標沿用 fresh 模式；追加的訊息包含題目一句話、使用者的答案與 `answer_id`，並要求 agent 先 `ask_get`
確認正式答案、做完後 `ask_report`；Kunsu 記 `delivery = resumed`，回執提供「打開原對話」

### 場景 7：原對話仍被開著但沒在等 → 不搶寫

**Given** 有活著的行程持有該對話（Claude：登記檔 `sessionId == conversation_id` 且 pid 存活；Codex：resume 回
`already has an active writer`），而且沒有等待者
**When** reactor 準備接續
**Then** 不做背景追加（避免分岔或被鎖）；記 `delivery = held`，每 60 秒重檢；行程結束後走場景 6；回執顯示
「原對話正開著，等它空下來就接續」與「打開原對話」；24 小時仍未接續時走既有「你答的 X 沒人處理」告警

### 場景 8：桌面 session 的 CLI id 已更換

**Given** 題目記錄的 `conversation_id` 已被桌面 app 換新（舊 id 出現在 `priorCliSessionIds`）
**When** reactor 準備追加
**Then** 以 `host_session_id` 讀取桌面 session 紀錄取得目前的 CLI id 後再 resume；找不到紀錄時用原 id；兩者都
不存在才改走開新對話

### 場景 9：Codex heartbeat 自己取走答案

**Given** 題目來自 Codex heartbeat automation 的 thread，答案送出時 thread 被 app 鎖住
**When** 下一次 heartbeat 在同一 thread 執行、處理答案後呼叫 `ask_report`
**Then** 記 `delivery = picked_up`，reactor 停止重試，不再追加也不開新對話；只呼叫 `ask_get` 不算取走（讀取沒有
副作用——追加的那一輪也會先 `ask_get`），heartbeat 結束後仍未回報就照常追加

### 場景 10：無法接續時才開新對話

**Given** 原對話不存在、已封存（桌面 session 紀錄 `isArchived: true`）、CLI 回報非暫時性錯誤，或暫時性錯誤（含 Claude
API 過載、限流等以 `<synthetic>` assistant 錯誤事件回報者）重試 3 次仍失敗
**When** reactor 放棄接續
**Then** 走現行 fresh 模式開新對話，prompt 附原對話連結；記 `delivery = fresh` 與放棄原因；fresh 也失敗才進
既有死信＋Discord 告警

### 場景 11：同一答案只走一條路

**Given** 同一 `answer_id` 可能同時被等待者、resume、heartbeat、fresh、drain 看見
**When** 任兩條路徑競爭
**Then** 第一個成功取得交付租約的路徑生效，其餘一律跳過；reactor 在 spawn 前先佔成 `resuming`，佔不到就不
spawn；追加的那一輪照指示呼叫 `ask_report` 時只記回饋，交付仍記 `resumed`；`delivery` 一旦進入 live_waiter／
resumed／picked_up／fresh 就不再改變；副作用（resume、spawn、Discord）恰好一次；等待者連線被切斷（例如排程結束
殺掉 `kunsu wait`）也會釋放，10 秒租約過後照常追加；沒有原對話身分的題，等待者不影響現行 wake。
交付狀態每次改變都追加一行帳本；資料庫重建（`kunsu rebuild-db`、`/admin/rebuild-db`）後交付狀態與重建前一致，
已被等待者取走、已追加、已回報或已開新對話的答案不會再交付一次，held 的答案在原對話釋放後照常追加；重建前已回答、
還沒被 reactor 處理的答案重建後照常交付（錢題若 approval 已消耗卻沒有執行紀錄，不重跑，改發 Discord 待人工對帳）。
帳本寫不進去（例如沒有完全磁碟權限）時，該行排進資料庫的待補寫佇列，權限恢復後依序補寫；佇列還沒清空時拒絕重建。
重建一定先完整讀完帳本才清資料庫，讀不到、或資料庫裡有任何非 internal 題目在帳本與 OPEN.json 都找不到時拒絕，
資料庫保持原樣，錯誤寫明原因（HTTP 409）；只自動重做 24 小時內還沒處理（含帳本只留到 pending／held）的答案，更舊的
連同題目與作答時間列在重建報告、發一則 Discord 請使用者確認，之後不再逐題告警；approval 已消耗的錢題不論多久都走人工
對帳。帳本寫入卡住超過 2 分鐘時發一次 Discord 請使用者重啟 Kunsu，`/health` 與 `kunsu doctor` 看得到帳本積欠

### 場景 12：AI 回報處理結果（AI 回饋）

**Given** 原對話處理完答案
**When** agent 呼叫 MCP `ask_report {question_id, answer_id, status: done|failed|blocked, summary_md}`
**Then** Kunsu 寫入 `report`、`executed_at`（沒有才寫）、補 `acked_at`，ledger 追加 `execution` 行；永不寫
`execution_result`（Kunsu 自己的執行紀錄）；只有適用回答延續原對話的題，交付仍 pending／held 時改為 `picked_up`（現行
喚醒的題照舊喚醒），回報與交付變更在同一段寫入中完成；同一
`answer_id`＋相同 status＋相同內容重送冪等（不重複寫帳本）；內容或 status 不同視為更新回饋；`answer_id` 不符或
題目未回答回 400；`ask_report` 永遠不能新增、修改答案或改變題目狀態（INV-7）

### 場景 13：回執看得到答案去了哪裡

**Given** 題目已回答
**When** 使用者在蓋板或主 app 看回執
**Then** 顯示接手狀態：「已交給等待中的原對話」「已在原對話追加處理」「原對話正開著，等它空下來」「已開新
對話處理（原因…）」；有原對話身分時提供「打開原對話」按鈕（Claude App／Codex App deep link）；收到
`ask_report` 後顯示 AI 回饋全文（sanitized Markdown）；錢題另顯示錢的結果一行（已執行完成／乾跑，未實際執行；被擋時
進失敗畫面）；交付未定案時輪詢，讀取失敗照退避重試，不會停止

### 場景 14：錢題不因接續而繞過授權鏈

**Given** 錢類題被回答
**When** 答案以任何路徑送回原對話
**Then** 追加訊息與開新對話的題目 JSON 都不含 approval record、HMAC 簽章、prepare 命令、executor token 或錢鏈原始
輸出；`ask_get`／`ask_list`／`ask_wait` 也不回簽章；執行仍只能走既有 prepare → 簽章 → commit 鏈（INV-15）；原對話
只得到人話狀態（已執行／被護欄擋下／乾跑）；錢鏈被擋時原對話一樣會收到通知。等待者路徑：錢鏈跑完前 `ask_wait` 不交付，
跑完才把含人話錢鏈狀態的答案交給等待者。`ask_create`／`ask_register` 重送已答題時回傳的答案同樣去敏。
`/events` 串流的執行事件同樣不帶 executor token。去敏不因 agent 已 `ask_report` 而失效：錢鏈結果（`execution_result`）與 AI 回饋（`answer.report`）分開保存，回報永不覆寫或
遮住錢鏈結果；回報早於錢鏈時，錢鏈之後被擋或成功的結果照樣寫入並顯示在回執；任何交付路徑（第一輪、held 之後、等待者租約之後）
最後進死信時，執行紀錄都以錢鏈結果開頭、後接交付失敗原因（被擋就保留被擋原因），一次寫成、不先閃「執行失敗」，
回執顯示「已執行完成」並另外說明交付失敗；已經執行完成的答案拒絕重新授權，錢不會被再動一次。agent 讀到的錢題狀態
依錢鏈結果的開頭判斷：還沒跑完是「錢鏈還在跑，以 Kunsu 回執為準」（錢鏈列還在 queued／executing、這一輪的錢鏈結果還沒寫入時一律如此，即使 executor 已驗證、commit 還沒回報——那不是結果不明；錢鏈跑完、同一列還在交付時照錢鏈結果說，跟追加訊息一致）、錢鏈不在跑而 executor 已用掉授權卻沒有結果是「結果不明：錢可能已經動過，等使用者人工對帳」、安全選項或紅燈是「未觸發」。migration 0007 之前已執行完成的錢題啟動與重建後補上結果，不被誤判結果不明（money-approval 場景 13）

### 場景 15：舊式 resume 與身分注入

**Given** agent 用舊格式 `wake_on_answer {mode: resume, session_id}` 建題，或 `conversation_id` 帶 `--last` 之類的值
**When** 建題
**Then** 舊格式正規化成 `source.conversation_id` 並走接續（不退化成開新對話）；不符 `[A-Za-z0-9_-]`、以 `-` 開頭或
超過 128 字的身分回 400 `bad_conversation_identity`；`host_session_id` 必須是 `local_…`

### 場景 17：追加的那一輪已開始處理就不重來

**Given** reactor 已在原對話追加一輪，agent 開始處理（Claude 有 assistant 事件、Codex 有 item 事件）或已呼叫 `ask_report`
**When** 子行程非零結束（例如處理到一半超過預算）或超過執行上限（預設 30 分鐘）被中止
**Then** 記 `delivery = resumed`，原因寫「原對話已開始處理，但結束時出錯／逾時」並發 Discord；不重試、不開新對話。
「已開始處理」＝Claude 有模型產生的 assistant 事件（帶 `error` 欄位或 model 為 `<synthetic>` 的 API 錯誤事件不算）、
Codex 有 `item.*` 事件，或 agent 已 `ask_report`。
失敗分類只看 stderr 與結尾的錯誤事件，不看 agent 的工作內容；「被鎖」只屬於 Codex，「找不到對話」只在開始處理前才算

### 場景 18：等待者不能變成孤兒

**Given** 原對話以背景 `kunsu wait` 等答案
**When** 發問的對話行程消失（Claude：`$CLAUDE_PID` 不再存活；其他：直接父行程被 launchd 收養），或答案已經在別處處理
**Then** 孤兒等待者以結束碼 4 自行退出並釋放租約；答案已由（或正由）追加、開新對話或原對話自行回報處理
（`delivery` 是 resuming／resumed／fresh／picked_up）時，`kunsu wait` 以結束碼 5 提醒「不要重複處理」；交付失敗（dead）
照常結束碼 0，由等待者處理

### 場景 16：held 不算接手

**Given** 答案因原對話被開著而 `held`
**When** 24 小時內一直沒接續
**Then** `acked_at` 保持空白、不寫 `execution_result`，既有「你答的 X 沒人處理」告警照常觸發；Lane 1 drain（`ask_answers_since`）
收割也不算接手；卡在 `resuming` 超過 15 分鐘時，已回報的記 resumed，沒回報的記 dead 並只告警一次（之後的 24 小時
告警不再重複）

## 驗收清單

- [ ] AC-CC-01：`kunsu ask` 在 Claude 環境自動補 host／conversation_id／host_session_id／attended／cwd／cli_path，不碰秘密
- [ ] AC-CC-02：Codex 環境帶 `CODEX_THREAD_ID`；MCP 直接傳同名欄位同樣受驗證；Codex 的對話 id 必須是 UUID
- [ ] AC-CC-03：無 conversation_id 時沿用現行 wake 行為並在回執說明
- [ ] AC-CC-04：等待者在線時答案直接交付、`delivery = live_waiter`、acked，reactor 不 resume／不 spawn
- [ ] AC-CC-05：等待者 10 秒租約涵蓋兩輪空檔
- [ ] AC-CC-06：已結束對話以 resume 追加；cwd＝source.cwd、清 `CLAUDE_CODE_*`、訊息含 answer_id 與 ask_get／ask_report 指示；回執有打開原對話
- [ ] AC-CC-07：對話被活行程持有時不追加，`delivery = held`、60 秒重檢、釋放後追加
- [ ] AC-CC-08：以 host_session_id 取得桌面最新 CLI id 再 resume
- [ ] AC-CC-09：heartbeat 處理後以 `ask_report` 回報時 `delivery = picked_up` 並停止重試；`ask_get` 沒有副作用
- [ ] AC-CC-10：無法接續才 fresh（附原對話連結與原因，含已封存）；API 錯誤照常重試；fresh 也失敗才死信＋Discord
- [ ] AC-CC-11：同一 answer_id 只有一條交付路徑生效（spawn 前先佔 `resuming`），副作用恰好一次；被切斷的等待者會釋放；
  交付狀態改變都進帳本，重建資料庫後不重複交付、不遺漏未交付、held 照常接續；帳本積欠時排隊補寫、拒絕重建
- [ ] AC-CC-12：`ask_report` 記錄 AI 回饋、不寫執行紀錄、只對適用接續的題改 picked_up；相同 status＋內容重送冪等、不同內容視為更新；不能寫入或改變答案（INV-7）
- [ ] AC-CC-13：回執顯示交付狀態、打開原對話按鈕與 AI 回饋；交付未定案時輪詢前 2 分鐘每 2 秒、之後每 10 秒，
  自回執打開起 30 分鐘停止（新資料不重設起算時間、讀取失敗照退避重試）；錢題顯示錢的結果
- [ ] AC-CC-14：錢題接續不外洩授權秘密（追加訊息、開新對話 JSON、MCP 讀取面——回報前後皆然）、不繞過 UI 簽章鏈；
  被擋也通知原對話；回報不遮住錢鏈結果（回報早於錢鏈、重新授權、任何路徑的交付死信都保留真實錢鏈結果）；已執行完成
  的答案拒絕重新授權；錢的結果不明（某一次授權已被消耗卻沒有結果、或 `EXEC_UNKNOWN`，以 nonce 判斷）拒絕重新授權並走人工對帳，
  使用者在回執標「錢已經動了／錢沒有動」才解除，
  重建前後一樣；executor 已用掉授權後 commit 失敗或逾時記「結果不明」，不是「被護欄擋下」；錢題不論 wake 設定都跑錢鏈；
  `/events` 不帶 executor token；狀態文字依錢鏈結果開頭判斷；錢鏈還在跑（這一輪的結果還沒寫入）時一律「錢鏈還在跑」（money-approval AC-MA-24）
- [ ] AC-CC-15：舊式 resume 正規化後仍接續；身分值不合規回 400
- [ ] AC-CC-16：held 不標 acked、不寫 execution（drain 也不標），24 小時告警可觸發
- [ ] AC-CC-17：已開始處理（不含 API 錯誤事件）或已回報的追加，失敗或逾時不重試、不開新對話；分類只看 stderr 與錯誤事件
- [ ] AC-CC-18：發問對話行程消失時等待者自行退出；答案已（正）在別處處理時 `kunsu wait` 結束碼 5

## 工程對應（已寫入 CONTRACT §2/§4/§8）

- `Source` 新增選填：`host`、`conversation_id`、`host_session_id`、`attended`、`cli_path`、`trigger`、
  `routine_name`、`routine_kind`、`expected_every_hours`（後四者見 routine-check-in.md）；`wake_on_answer.mode`
  新增 `continue`，有 `conversation_id` 時為預設。
- 新增 `deliveries` 表：`answer_id UNIQUE`、`state`（pending｜held｜resuming｜live_waiter｜resumed｜picked_up｜fresh｜no_continuation｜dead）、
  `attempts`、`reason`、`updated_at`；等待者以 ask_wait in-flight 計數＋10 秒租約表示。
- MCP 工具新增 `ask_report`（之後證據畫布再加 `ask_register`，共 13 個）；`kunsu wait --timeout 0` 支援無總時限迴圈。
- 測試：交付階梯為純函數表驅動（輸入：等待者、登記檔、pid 存活、resume 結果、heartbeat、重試次數），
  spawn 以注入的 runner 驗證命令、cwd 與環境；server integration 驗證租約與冪等；手動驗證 Claude App 與
  Codex App 畫面確實看到追加的一輪。
- agent 端 SOP（`~/.claude/skills/kunsu`、`~/.codex/skills/kunsu`）同步更新：Claude 建題後以 background Bash
  掛 `kunsu wait`（連不上 app 時以 1、2、4…30 秒退避重試，不退出）；收到答案先 `ask_get` 再繼續，做完 `ask_report`。
