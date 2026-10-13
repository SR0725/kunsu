# Harness 答案回流（harness-roundtrip）— BDD 驗收規格

> 依據：docs/CONTRACT.md §4（ask_wait / ask_answers_since）、§0（檔案 inbox）、§8（Reactor）；
> 設計正本 §7（雙車道總原則）。
> 不變量對應：INV-7（讀取面無作答寫入，正式規格在 answer-flow.md）。
> 界外：三態回執與 24h 告警見 answer-flow.md；drain 在 mkt-daily 的掛載見 survival.md。

## 概述

雙車道總原則：**Lane 1（vault ledger + 排程 drain）是永遠成立的保底；Lane 2（ask_wait 即時
long-poll + reactor 喚醒）是加速器——加速器壞了系統只是變慢，不會丟答案。**
消化冪等以 answer 的穩定識別為鍵；已答題永不原樣重現。

## 行為規格

### 場景 1：ask_wait long-poll 即時回流

**Given** agent 建題後同 session 呼叫 `ask_wait {question_id, timeout_seconds: 300}`
**When** 使用者在 timeout 內作答
**Then** 呼叫立即返回 `{status: "answered", answer}`（不等 timeout 走完）；題目被 expired/cancelled 時對應返回該狀態

### 場景 2：ask_wait 超時可迴圈續等

**Given** `ask_wait` 掛著而使用者未作答
**When** timeout_seconds 走完
**Then** 返回 `{status: "pending_timeout"}`；ask_wait 無伺服器端 session 狀態，agent 直接再次呼叫續等（Claude 與 Codex 同構可用）；`timeout_seconds > 540` 的請求被 400 拒收（避開 harness tool timeout）

### 場景 3：answers_since 游標收割

**Given** consumer `mkt-daily` 曾收割到游標 C1，之後又有 3 題進入終態
**When** 呼叫 `ask_answers_since {consumer: "mkt-daily", cursor: C1}`
**Then** 返回那 3 筆終態事件與 `next_cursor: C2`；游標 per-consumer 存 server DB；不帶 cursor 的首次呼叫從頭收割；不同 consumer 各自獨立游標互不干擾

### 場景 4：at-least-once 投遞——crash 不丟事件

**Given** consumer 收到一批事件但在自己落地完成前 crash（未以新 cursor 再次呼叫確認）
**When** consumer 重啟後以舊 cursor 再呼叫
**Then** 同一批事件再次返回——寧可重複、絕不丟失；消費端以 answer 穩定識別冪等去重

### 場景 5：檔案 inbox fallback——app 死掉退化不崩潰

**Given** Kunsu app 死掉（`/health` 不通）
**When** agent 把題目 JSON 落檔 `~/.kunsu/inbox/<id>.json`（並照常推 Discord 鏡像）
**Then** app 復活後自動 ingest：合法檔走與 `ask_create` 完全相同的驗證與 upsert 語義（同 dedup id 不重置、schema 違規標 malformed 不入列），處理完移到 `~/.kunsu/inbox/processed/`；非法 JSON 壞檔不阻塞其他檔案的 ingest，被隔離移出 inbox 並 Discord 告警

### 場景 6：reactor fresh mode 喚醒

**Given** 一題帶 `wake_on_answer: {mode: "fresh", playbook: "..."}`、`source.agent = "claude"` 被作答
**When** reactor 消化該題
**Then** 串行 queue（一次一個）spawn：`claude -p "<prompt>" --output-format stream-json --verbose --permission-mode bypassPermissions --max-budget-usd 2`，cwd = vault 根；prompt = playbook 檔內容 + `## 題目與答案\n<question+answer JSON>`（自包含、不依賴死掉 session 的 context）；spawn 不經 zsh alias、旗標顯式帶、`claude` 路徑以 `which claude` 解析並快取

### 場景 7：codex 題走 codex exec

**Given** 被作答的題 `source.agent = "codex"`
**When** reactor 喚醒
**Then** spawn `codex exec - --json -s workspace-write -C <vault根>`，prompt 走 stdin

### 場景 8：resume 只給活著的互動 session

> 2026-10-08 實機研究推翻本場景：現行實作以 session id 找 `~/.claude/sessions/<pid>.json` 永遠不命中；且對
> 仍開著的 session 背景 resume 會產生分岔、答案被覆蓋。`conversation-continuity.md`（Target）交付後由 AC-CC-06／07
> 取代：只對已結束的對話追加，活著的對話等它空下來。

**Given** 一題 `wake_on_answer.mode = "resume"` 帶 session_id
**When** reactor 查 `~/.claude/sessions/*.json` registry
**Then** 命中活的互動 session 才走 resume；未命中（含排程 session、已死 session）一律 fallback fresh——不嘗試 resume 脆的 transcript

### 場景 9：執行產出回寫與失敗死信

**Given** reactor 起跑一個喚醒
**When** (a) 成功；(b) 失敗
**Then** (a) `execution_result` + `executed_at` 回寫題目與 ledger；(b) 重試 3 次指數退避，仍失敗進死信 + Discord 告警——絕不靜默；起跑當下即標 `acked_at`

### 場景 10：冪等消化——同一答案絕不重複執行

**Given** 一題的 answered 事件同時可能經 `ask_wait`、`ask_answers_since`（at-least-once 重放）、reactor 三路被看到
**When** 任一消化端（reactor、drain consumer）對同一 answer 處理第二次
**Then** 以 answer 穩定識別（answer_id，沿用 executor `_consumed` 模式）判重跳過；副作用（executor commit、decision/prediction 寫檔、Discord 推送）只發生一次

### 場景 11：已答題永不原樣重現

**Given** 題目 `pa-x` 已 answered，agent（如隔天排程）不知情地以同 id 再次 `ask_create`
**When** server 收單
**Then** 不建新題、不把題目重置回 pending、不重新通知使用者；回傳讓 agent 看得到 `state = answered` 與既有答案的結果——agent 必須先讀答案再決定下一步；若執行失敗要再問，必須開**新 id** 的新題且首行寫明「你 X 日已批，執行失敗因為 Y」

### 場景 12：Monitor 預武裝 WS 事件流

> 2026-10-08：Monitor 無法帶 Bearer 連 WS，此路徑對 Claude session 不可行；`conversation-continuity.md` 改以
> background Bash 執行 `kunsu wait` 作為原對話等待者（AC-CC-04／05）。WS 事件流本身仍保留給其他 client。

**Given** Claude Code 互動 session 對一題預武裝 `Monitor ws://127.0.0.1:4777/events?question=<id>`（Bearer）
**When** 該題發生任何狀態轉移
**Then** WS 推送一個 JSON frame（體感零延遲）；`?question` 過濾只收該題；WS 斷線不影響 Lane 1 保底回流

### 場景 13：記憶候選不繞過仲裁

**Given** reactor 執行產出包含記憶候選（memory_op）
**When** 回寫階段
**Then** reactor 不自配 M-ID，只 append `projects/meta-ads/data/memory-inbox/<date>-<qid>.md`，由隔天 mkt-daily cleanup（唯一記憶仲裁者）統一編號落盤

## 驗收清單

- [ ] AC-HR-01：ask_wait 在作答當下即返回 answered+answer；expired/cancelled 正確透傳
- [ ] AC-HR-02：ask_wait 超時回 `pending_timeout` 可無狀態續等；timeout>540 拒收
- [ ] AC-HR-03：answers_since 依 per-consumer 游標返回增量終態事件 + next_cursor；多 consumer 隔離
- [ ] AC-HR-04：cursor 未推進時重呼叫重收同批（at-least-once），事件零丟失
- [ ] AC-HR-05：inbox 檔案 ingest 與 ask_create 同語義（驗證+upsert），成功移 processed/，壞檔隔離不阻塞 + Discord 告警
- [ ] AC-HR-06：fresh mode spawn 命令旗標與 cwd 與契約逐字一致，不經 shell alias，prompt 自包含
- [ ] AC-HR-07：codex 題走 `codex exec - --json -s workspace-write -C <vault>`，prompt 走 stdin
- [ ] AC-HR-08：resume 僅 registry 命中互動 session；否則 fallback fresh
- [ ] AC-HR-09：成功回寫 execution_result+executed_at；失敗 3 次退避 → 死信 + Discord
- [ ] AC-HR-10：同一 answer 多路/重放消化，副作用恰好一次（answer_id 冪等）
- [ ] AC-HR-11：已 answered 的 dedup id 再建題不重現、不重置、不重通知，agent 可讀到答案
- [ ] AC-HR-12：WS /events 每轉移一 frame、支援 question 過濾、需 Bearer
- [ ] AC-HR-13：reactor 串行 queue 一次只跑一個喚醒；記憶候選只進 memory-inbox 不自配 M-ID
