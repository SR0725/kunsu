# 題組（batch-review）— BDD 驗收規格

> 狀態：**已實作，待 QA（2026-10-09）**；2026-10-09 Ray 核准（預設採用、第 2 輪新題組）。工程契約：`docs/CONTRACT.md` §14。
> QA 第 1 輪修正（2026-10-09，Ray 核准）：題組不能帶 `kongko_ref`；Esc 只在輸入框裡不稍後再問；草稿框寬度改變也重算高度；
> 題組 id 留出輪次後綴空間；開新對話的 prompt 走 stdin；回執焦點與 Markdown；全部退回的整體指示先擋；**不准佔位文字**、
> 採用的可編輯項目不能是空的；面板開著時重新註冊保留使用者的修改；窄版標題換行；只核可項目也有「請 AI 改」（與原型一致）。
> QA 第 2 輪修正（2026-10-09，Ray 核准）：主視窗的作答流程也跟著最新清單（重新註冊保留修改、標「AI 更新了這一項」、送出帶新
> 雜湊，場景 9）；上限內最大的題組回原對話追加時訊息塞得進命令列參數、回到原對話（場景 6）；回看已結案的題組唯讀、狀態依
> 實際終態（decision-surface 場景 17）。
> 產品依據：`docs/PRD.md` §25；可操作參考：
> [`docs/prototypes/batch-review.html`](../prototypes/batch-review.html)。
> 依賴：decision-surface（面板版型、送出與回執）、conversation-continuity（答案回原對話、`ask_report`）、
> evidence-canvas（草稿註冊與版本）。
> 界外：題組裡放動錢項目；Kunsu 代寄信或代發文；聊天式來回討論。

## 背景與範圍

AI 一次處理一批同類工作（回信、留言回覆、逐項核可）時，用一題題組讓使用者在同一個面板看完、改完、決定完，
AI 再照確認過的內容逐項執行。題組和其他題一樣走 15 分鐘強制循環、guards、勿擾與 ledger。

## 行為規格

### 場景 1：建立題組

**Given** agent 以 `kind: batch` 建題（JSON 或草稿註冊），帶 1–20 個 `items[]`，每項 `{id, title, subtitle?, context_md,
proposal, editable, default: approve|skip, default_reason?}`
**When** server 收單
**Then** 驗證通過才建題：項目數 1–20、項目 id 不重複且路徑安全、`proposal` 非空（例外：`default: skip` 的可編輯項目可以
是空草稿）、可編輯項目的草稿 ≤ 5,000 字、`context_md` 走 Markdown 安全檢查；題組不能是錢題（`money.is_money_op` 拒收）；
題組不能帶 `kongko_ref`（400 `batch_kongko_ref`：Kongko 發佈事件沒有逐項決定，不能結案題組）；題組 id 去掉輪次後綴
（`-rN`）後最多 116 字（400 `batch_id_too_long`，訊息說明要留 `-r2`～`-r99` 的空間、題目 id 上限 120 字）；違反時 400 並
指出第幾項哪個欄位
**And** AI 建議不處理的可編輯項目，草稿要嘛是空的、要嘛是真的可以送出的回覆，不准寫「（不回）」之類的佔位文字（原因寫在
`default_reason`）——skill 範例照這條寫；server 無法判斷文字是不是佔位，靠下面場景 3 的「採用不能是空的」守住

### 場景 2：一次看完一批

**Given** 一題 10 項的題組
**When** 面板打開
**Then** 右欄列出 10 張項目卡（標題、狀態、一行摘要、草稿或建議、三個決定鈕），左欄顯示目前選中項目的原文；
切換項目時左欄跟著換、不重設任何已改的內容；總結列即時顯示「採用 N 項 · 其中 M 項你改過 · 不處理 K · 請 AI 改 L」
（沒有的類別不寫；全是只核可項目時寫「照建議 N 項」）。窄面板（面板本身 < 1100 px，與決策面板同一個 container query
斷點，decision-surface 場景 14）時單欄、整個面板一起捲動，原文改在選中的卡片裡顯示；寬度剛好 1100 px 時是兩欄、決定欄
自己可以捲；項目卡標題太長時換行顯示完整標題，不省略

### 場景 3：逐項決定

**Given** 一個可編輯項目
**When** 使用者改了草稿、或按採用／不處理／請 AI 改
**Then** 改過的項目標「你改過」並可還原 AI 版本；不處理的項目淡化；請 AI 改必須寫一句怎麼改，沒寫時送出停用並
說明原因；只核可的項目沒有輸入框（不能改文字），但和可編輯項目一樣有三個決定鈕：照建議、不處理、請 AI 改（2026-10-09
依 Ray 核准的原型 `docs/prototypes/batch-review.html` 修正；舊文寫「只能照建議或不處理」與原型不一致）
**And** 可編輯項目採用時最終文字不能是空的（只有空白也算空）：AI 建議不處理、草稿是空的項目被改成採用而沒寫內容時，
送出停用，總結列下方寫「第 N 項沒有內容，先寫好再採用」；server 收到採用但文字是空的也拒收（400，同一句話）

### 場景 4：一鍵處理全部

**Given** 題組任一狀態
**When** 使用者按「全部採用 AI 草稿」，或「全部退回，請 AI 重寫…」並寫下指示
**Then** 全部採用：非「不處理」的項目都改為採用（保留已改的文字），清掉請 AI 改的指示；全部退回：一次送出，
每項都是請 AI 改，附上同一段指示
**And** 全部退回的整體指示最多 2,000 字：超過時「全部退回重寫」停用、⌘Enter 不送出，指示欄下寫「整體指示超過 2,000 字
（N 字），先縮短再送出」；server 的錯誤訊息分清楚整體指示（「整體指示（套用到所有項目的那段）超過 2,000 字」）與單項
指示（「第 N 項的指示（請 AI 改）超過 2,000 字」），整體指示先檢查

### 場景 5：送出就是確認的內容

**Given** 使用者送出題組
**When** server 收單
**Then** 答案逐項記錄 `{item_id, action: approve|skip|revise, text?, note?}`：採用的項目帶最後文字（改過的就是改過的）；
答案綁定題目版本與每項內容雜湊；同一題只能作答一次；ledger 快照含每項決定；agent 讀到的答案就是要執行的
文字，必須一字不改送出（所見即所送）

### 場景 6：改稿迴圈

**Given** 答案裡有請 AI 改的項目
**When** 答案回到原對話
**Then** agent 先執行採用的項目，再依每項指示重寫要改的項目，以 `<原 id>-r2` 建新題組（只含要重看的項目，標明
「第 2 輪」與原題連結）；`ask_report` 回報已執行幾項、重寫幾項；因為建題時題組 id 已限制長度，`<原 id>-r2`（到 `-r99`）
一定是合法 id
**And** 任何通過建題驗證、使用者照規則作答的題組都交得出去：開新對話時 prompt 走 stdin（`claude -p --input-format text`
不帶 prompt 參數；Codex 本來就是 `codex exec -`），不放進命令列參數，最大的題組（20 項 × 5,000 字草稿＋作答文字、
每封來信 4,000 字以上）也不會撞到 macOS 的參數長度上限而死信
**And** 回原對話追加（`claude -p <訊息> --resume`，訊息在命令列參數）時，上限內最大的題組（20 項只核可、每項建議 5,000 字、
副標 200 字、標題 80 字）的追加訊息塞得進 macOS 的命令列參數上限（留 64 KB 給環境變數），真的回到原對話（交付狀態 resumed），
不會 spawn 失敗改開新對話；建議與副標超過上限在建題時就拒收（`batch_item_proposal_too_long`、`batch_item_subtitle_too_long`）

### 場景 7：鍵盤與無障礙

**Given** 面板顯示題組
**When** 只用鍵盤
**Then** J／K（或 ↑／↓）換項目、E 進入草稿、Esc 離開輸入框（只有焦點真的在 textarea／input／contenteditable 裡才算；
不在輸入框時——包括焦點在草稿區的「還原 AI 版本」、全部退回的「取消」「全部退回重寫」、「確認退回」這些按鈕上——Esc＝
稍後再問）、A／X／R 決定、⌘Enter 送出（在「全部退回」的指示欄裡＝送出全部退回）；單按 Enter 不送出；項目清單是 list，
每張卡的決定鈕是一組可讀出狀態的按鈕；狀態不只靠顏色（都有文字）；長草稿自動長高、不出現水平捲動，面板或視窗寬度改變
後也重新量高度，使用者看到的一定是全文（不會有被藏起來卻照樣送出的文字）

### 場景 8：回執

**Given** 題組已送出
**When** 面板顯示回執
**Then** 列出每項的決定與總數，說明 AI 接下來會做什麼（逐項送出幾項、哪些會以第 2 輪再送來、哪些封存）；
AI 回報後顯示回饋（裡面的連結第二次點擊能打開，蓋板先讓開）；題目和作答時一樣經 Markdown 渲染；送出後焦點移到
「下一題／完成」，焦點在回執裡的連結或項目列上按 Enter 只啟動那個控制項，不會跳下一題

### 場景 9：面板開著時草稿重新註冊

**Given** 面板開著，使用者已經改了幾項（文字、決定或請 AI 改的指示）
**When** agent 重新註冊同一題的草稿（項目內容升版）
**Then** AI 草稿沒變的項目保留使用者的修改與決定；AI 草稿變了的項目顯示「AI 更新了這一項」：使用者改過的文字保留，「還原
AI 版本」換成新的 AI 版本；使用者沒改過的直接換成新版本；新加入的項目照預設；被拿掉的項目消失；送出時帶的是新版本的
雜湊（不會靜默清掉使用者的修改，也不會用舊雜湊被拒收）；蓋板與主視窗的作答流程（「開始處理 N 題」、待決清單點一題、蓋板
就緒逾時的後備）都一樣（2026-10-09 QA 第 2 輪：舊版主視窗只拿開啟當下的清單，送出帶舊雜湊被拒收、重開後修改全部消失）

## 驗收清單

- [ ] AC-BR-01：`kind: batch` 建題驗證（項目數、id、草稿長度、Markdown 安全、拒收錢題、拒收 `kongko_ref`、題組 id 留出
  輪次後綴空間、不處理項目可給空草稿），錯誤指出項目與欄位
- [ ] AC-BR-02：一次列出所有項目；左欄跟著選中項目；切換不重設內容；總結列即時；窄面板（< 1100 px）單欄、與決策面板同一個
  斷點；標題換行不省略
- [ ] AC-BR-03：逐項採用／不處理／請 AI 改（只核可項目也三個）；改過可還原；請 AI 改必填指示；只核可項目沒有輸入框；
  可編輯項目採用時文字不能是空的（面板與 server 都擋）
- [ ] AC-BR-04：全部採用（保留改過的文字）與全部退回（附一段指示，≤ 2,000 字，超過先擋；錯誤分清整體與單項）
- [ ] AC-BR-05：答案逐項記錄並綁定版本與內容雜湊；所見即所送；ledger 可追溯；題組只能在面板作答（Kongko 發佈事件不能結案）
- [ ] AC-BR-06：請 AI 改的項目以 `-r2` 新題組回來，只含要重看的項目並標明第幾輪；`-rN` 一定是合法 id；開新對話的 prompt 走
  stdin，最大題組也交得出去；上限內最大的題組回原對話追加時訊息塞得進命令列參數、真的回到原對話
- [ ] AC-BR-07：鍵盤操作（Esc 只在輸入框裡不稍後再問）、list 語意、狀態有文字、長草稿不水平捲動、寬度改變後重算高度
- [ ] AC-BR-08：回執逐項列出決定與後續；題目 Markdown；焦點到回執主按鈕；連結上 Enter 不跳題
- [ ] AC-BR-09：面板開著時重新註冊不清掉使用者的修改；AI 草稿變了的項目標「AI 更新了這一項」並可還原新版本；蓋板與主視窗的
  作答流程都一樣

## 測試對應

| AC | 層 | 測試 |
|---|---|---|
| AC-BR-01 | core | `crates/kunsu-core/tests/batch_review.rs`：`batch_kind_parses_and_items_default_to_approve`、`unknown_item_field_is_rejected`、`twenty_items_and_a_5000_char_draft_are_allowed`、`batch_create_rejections_name_the_item_and_field`、`money_batch_is_rejected_before_money_field_checks` |
| AC-BR-01 | server | `crates/kunsu-server/tests/batch_review.rs`：`ask_create_accepts_a_batch_and_points_at_the_bad_item`、`json_batch_context_md_goes_through_the_markdown_checks`、`batch_drafts_register_with_items_and_check_each_context`、`ask_update_cannot_add_options_to_a_batch` |
| AC-BR-02 | UI | `ui/src/surface/batch.test.ts`（初始狀態、總結列）、`BatchColumn.test.tsx`（list、左欄原文、總結列）；瀏覽器 QA 1440／980 |
| AC-BR-03 | UI | `batch.test.ts`（逐項決定、送出條件）、`BatchColumn.test.tsx`（只核可項目沒有輸入框）；server `item_text_and_note_rules` |
| AC-BR-04 | UI | `batch.test.ts`（一鍵處理全部）；shell `batch_payload_becomes_an_item_by_item_ui_answer` |
| AC-BR-05 | core＋server＋UI | core `every_item_gets_a_server_computed_content_hash`、`draft_update_replaces_items_and_rehashes`；server `batch_answer_is_recorded_item_by_item_in_question_order`、`every_item_must_be_decided_exactly_once`、`panel_answers_are_bound_to_each_item_hash`、`batch_needs_item_decisions_and_items_need_a_batch`、`item_decisions_reach_the_ledger_and_agent_reads`、`same_id_resend_only_touches_and_keeps_items`；UI `batch.test.ts`（送出載荷） |
| AC-BR-06 | core＋server | `kunsu_core::batch::tests::rounds_follow_the_r_suffix`；server `continuation_prompt_lists_each_decision_and_the_next_round`、`fresh_conversation_prompt_carries_the_batch_rules`；agent 端規則見 `plugins/kunsu/skills/kunsu/references/batch.md` |
| AC-BR-07 | UI | `batch.test.ts`（鍵盤）、`BatchColumn.test.tsx`（list 語意、狀態有文字）；瀏覽器 QA（長草稿自動長高、無水平捲動） |
| AC-BR-08 | UI | `batch.test.ts`（回執）、`BatchColumn.test.tsx`（回執逐項列出） |
| QA 第 2 輪 | server＋UI | server `crates/kunsu-server/tests/qa_batch_round2.rs`（開新對話的 prompt 走 stdin：`qa2_br06_fresh_claude_prompt_arrives_on_stdin_and_never_in_argv`、`qa2_br06_fresh_codex_prompt_arrives_on_stdin_and_never_in_argv`；上限內最大的只核可題組回原對話：`qa2_br06_resume_prompt_for_the_largest_valid_approve_only_batch_fits_the_command_line`）；UI `ui/src/surface/qaBatchRound2.test.tsx`（重新註冊 rebase 與送出雜湊、送出前擋下、鍵盤）、`qaDecisionRound3.test.tsx`（主視窗傳 `latest`）；瀏覽器 QA（mock 主視窗重新註冊） |
| QA 第 1 輪 | core＋server＋UI | server `crates/kunsu-server/tests/qa_batch_round1.rs`（Kongko 不能結案題組、`-rN` 合法、最大題組交得出去、CLI／Discord 不能作答、`ask_answers_since` 帶逐項決定）、`batch_review.rs`（`overall_instruction_error_names_the_overall_instruction`、`empty_skip_draft_registers_and_cannot_be_approved_empty`、`fresh_conversation_prompt_carries_the_batch_rules` 讀 stdin）、`reactor_inbox.rs`（`reactor_dry_run_assembles_claude_fresh_command`：`-p --input-format text`、prompt 走 stdin）；core `crates/kunsu-core/tests/batch_review.rs`（`batch_with_kongko_ref_is_rejected`、`batch_id_leaves_room_for_the_round_suffix`、`skip_default_item_may_have_an_empty_draft`、`approving_an_empty_text_names_the_item`）；UI `ui/src/surface/qaBatchRound1.test.tsx`（Esc、空草稿採用、整體指示長度、回執 Markdown）、`batch.test.ts`（`rebaseDrafts`、`redoAllBlocker`）、`ui/src/surface/surfaceRound2.test.tsx`（回執焦點與 Enter）；瀏覽器 QA（寬度改變後草稿框重算高度、1100 px 兩欄可捲、窄版標題換行） |
