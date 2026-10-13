---
name: kunsu
description: >-
  使用本機的 Kunsu（軍師）正式請求人類決策、緊急確認，或已授權排程的承諾 check-in。
  Claude／Codex 需要使用者拍板才能繼續、使用者說「丟進 Kunsu」「問軍師」，或日記／復盤督導到期時使用。
  能自己查的事實先查；純進度、成功通知、FYI 與開放式聊天不建題。
---

# Kunsu 軍師

Kunsu 是使用者的強制決策收件匣與嚴厲督導，也承接 AI 私人助理準備的可編輯回覆草稿。逼他面對需要回應的事，永不代答、批准或自動擴權。
支援 Claude 與 Codex；排程用 Claude Routines／Codex automations。外掛只接既有本機 app，不排程、不上雲。

客服、私人訊息、課程／Meta／LINE／Threads 回覆任務，先讀 `references/assistant-drafts.md`。自動收件與擬稿，不自動發送；一則也用 batch，使用者不必逐項按確認。

## 1. 建題前

讀取 `http://127.0.0.1:4777/health`（唯讀、無憑證），或用 `~/.kunsu/bin/kunsu doctor </dev/null`。
外掛 MCP bridge 自動讀 `~/.kunsu/token`；**不要印出、複製或將 token 寫入設定、外掛、草稿與聊天**。
若 app 未開，開啟已安裝 Kunsu；仍無法連線，可用 CLI 的檔案 inbox fallback，並在目前對話交代阻塞。
沒有另行授權不要對外傳訊息。最新工具 schema 可從 `/describe` 讀取；契約正本是 `~/Desktop/kunsu/docs/CONTRACT.md`。

## 2. 一題讓使用者十秒看懂

需要現成草稿時讀 `references/workflows.md`，可選 decision／urgent／diary／retrospective／batch。範本先改成真正的事實與提案，不能直接送出提示文字或示例數字。

- title：白話的決策句（≤80 字）；question_line：≤24 字。
- body 第一行「要你決定：…」，給必要事實、你的建議、各選項後果與不決定的後果。
- evidence ≤3 條，每條 `{text, source}`，來源與觀測時間要清楚；未知不是零。
- `kind=choice`：2–4 個 `{id, label, detail?, recommended?}`。策略紅燈必須 choice。
- kind 可用 `text`（日記）、`confirm`、`batch`（同類多項工作；**一批同類工作不要拆成多題**，先讀 `references/batch.md`）、`kongko_review`。
  題組答案逐項給你：採用的項目帶最終文字，一字不改送出；請 AI 改的以 `<原 id>-r2` 建第 2 輪題組。
- tier 預設 t1；策略拍板用 t2；t0 必須不可逆／動錢、完整證據、真實時限三者同時成立；t3 只限已有書面預授權。
- 所有合格題走既有 15 分鐘詢問循環；關閉未回答不結案。嚴厲仍遵守螢幕分享／錄影／會議 guard。

## 3. 建題與來源對話

**優先 CLI** `~/.kunsu/bin/kunsu ask --json <file|->`，CLI 從目前 Agent 環境補真實對話身分。
JSON 至少帶 id、title、question_line、kind、tier、body_md、default_action、source；其他欄位依 schema。
`default_action: {on_expire: "noop_log"}`；穩定 id 用 `pa-<slug>`，同一件事重送同 id，不重設倒數。

透過 MCP `ask_create` 時，自己帶真實 `source`（bridge 不猜共用 MCP 行程的身分）：

- Codex：`agent: "codex"`、`host: "codex_app"`、`conversation_id: $CODEX_THREAD_ID`、`cwd: <目前專案絕對路徑>`。
- Claude 桌面：`agent: "claude"`、`host: "claude_desktop"`、`conversation_id: $CLAUDE_CODE_SESSION_ID`、`host_session_id: $CLAUDE_CODE_HOST_SESSION_ID`、cwd；CLI 用 `host: "claude_cli"`。
- 不臆造 id；讀不到時改用 CLI，仍缺來源則交代此題無法保證接回原對話。

要附圖表／圖片時先讀 `references/canvas.md`，用草稿註冊：

1. 在 `~/.kunsu/drafts/<id>/` 寫 question.md（YAML frontmatter 同建題欄位，body 是 Markdown）；需要時附 canvas.tsx/jsx/html 與 assets/。
2. `~/.kunsu/bin/kunsu ask <dir> --dry-run`，修正 issues，直到 ok:true。
3. 用可用的圖片查看工具查看 `preview_png`，確認使用者真的看得懂；再拿掉 --dry-run 註冊。
4. MCP 等價 `ask_register {draft_dir, dry_run, source}`。草稿更新不重設倒數；已結案不可改。

## 4. 建題後接續與回報

**Codex** 建題後結束這一輪，Kunsu 等 thread 空下來以 `codex exec resume` 接回原對話。
不要同步長等讓 thread 忙碌。收到 `[Kunsu] 使用者已回答…` 後，先 ask_get 確認正式答案再繼續。

**Claude** 建題後立刻用背景 Bash 執行 `~/.kunsu/bin/kunsu wait <id> --timeout 0`，再結束這輪。
結束碼 0：接續；5：已交給其他路徑處理，勿重做；4：原對話不再存活，由 Kunsu 接續；6：使用者在手機上追問你（不是答案）——照輸出用
`~/.kunsu/bin/kunsu followup-reply <id> <追問 id> "回覆"`（或 MCP ask_followup_reply）回答，再背景執行 `kunsu wait <id> --timeout 0` 繼續等。
MCP 的 ask_wait 回 `status: followup` 時同理。

完成後必呼叫 `ask_report {question_id, answer_id, status: "done"|"failed"|"blocked", summary_md}`，交代實際結果與下一步。
讀到答案不等於已完成；避免重複執行，先看 delivery／answer.report。

## 5. 排程督導

設定日記／復盤等排程時才讀 `references/routines.md`。沿用或更新已有排程，不能因這個技能自動新增日記時間。
建題使用 `--routine <穩定id> --routine-name <名稱> --every <小時>`，題目 id `rt-<routine>-<YYYYMMDD>`；同次重送同 id。
目標正本留在原排程 prompt／原資料位置；Kunsu 保存問答與回報。排程由宿主管理，不用 crontab／launchd 重建排程引擎。

## 6. 安全邊界

- MCP 只有 ask_create、ask_register、ask_wait、ask_answers_since、ask_list、ask_get、ask_update、ask_cancel、ask_supersede、ask_report、ask_followup_reply、dnd_get、dnd_set、dnd_clear。
- 不存在 answer／approve／execute；不使用真人 CLI answer、不模擬 UI 點擊、不偽造簽章。不把關閉、過期或沉默當同意。
- 勿擾只有使用者明確要求才 set／clear；先 dnd_get，再做最小變更。最長 24 小時。
- `ask_answers_since`／drain 只用於明確授權的既有批次收割，不把其中錢鏈簽章或原始執行輸出帶進對話。
- 動錢題必帶 `money.is_money_op=true`、精確 `params_line`、prediction_ref、完整證據與安全預設；`executor_prepare_cmd` 只是一個字串，建題絕不執行。
- 動錢只准 choice 且有執行型 recommended 選項；on_expire 只准 noop_log／escalate_discord。批准僅授權該題精確參數，單次消耗、15 分鐘 TTL；紅燈不自動執行。
- 錢鏈由 Kunsu 既有 executor 執行，Agent 只確認與回報；任何授權不豁免既有金額／冷卻護欄，也不延伸到其他支出。
- 畫布無網路、無 IPC、不能作答；原生選項與送出才是正式回答。
