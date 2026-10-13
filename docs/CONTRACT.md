# Kunsu 系統契約（CONTRACT）— 平行建造的唯一錨點

> 所有 builder agent 以本檔為準。與設計文件衝突時以本檔為準；改本檔必須顯式說明理由。
> 設計歷史：初版系統設計（2026-07-31）在 repo 以外的作者筆記，只供脈絡；一律以本 repo 文件為準。

## 0. 命名與路徑常數

| 常數 | 值 |
|---|---|
| App 名 | Kunsu（軍師）；bundle id `me.ray.kunsu` |
| HTTP bind | `127.0.0.1:4777` |
| MCP endpoint | `http://127.0.0.1:4777/mcp`（streamable HTTP，`Authorization: Bearer <token>`） |
| Bearer token | `~/.kunsu/token`（首次啟動自動生成 32 hex；agent 讀此檔） |
| 檔案 fallback inbox | `~/.kunsu/inbox/*.json`（app 死時 agent 落檔，復活 ingest 後移到 `~/.kunsu/inbox/processed/`） |
| policy | app watch `~/.kunsu/policy.toml`（build-install 建 symlink → repo `config/policy.toml`，改 repo 檔即熱載） |
| playbooks | `~/.kunsu/playbooks/` symlink → repo `playbooks/` |
| SQLite | `~/Library/Application Support/me.ray.kunsu/kunsu.db`（WAL；可由 ledger 重建） |
| vault ledger | `<vault 根>/logs/asks/YYYY-MM-DD.jsonl`（append-only）+ `OPEN.json` + `ASKS-SCHEMA.md` |
| Keychain | service `me.ray.kunsu.approval-hmac`（HMAC key，僅 app 建立/讀取） |
| launchd | `~/Library/LaunchAgents/me.ray.kunsu.plist` → `/Applications/Kunsu.app`（**絕不指 repo build 路徑**） |
| vault 根 | 由 `kunsu_core::paths::resolve_vault_root` 決定，新使用者預設 `~/Kunsu`（reactor cwd 與 ledger 寫入基準；見 §18「資料位置」） |

## 1. Workspace 結構

```
kunsu/
  crates/
    kunsu-core/      # 純函數層：型別、狀態機、schema 驗證、policy 解析。零 I/O、零 async。
    kunsu-server/    # SQLite 儲存、axum HTTP+MCP、ledger 寫入、timer loop、reactor、guards FFI
    kunsu-cli/       # `kunsu` binary：ask/wait/drain/list/doctor/rebuild-db
  src-tauri/         # Tauri 2 shell：tray、膠囊視窗、蓋板視窗、通知、autostart；內嵌啟動 kunsu-server
  ui/                # React 19 + Vite + Tailwind v4：卡片流/膠囊/收據（Tauri IPC，不走 HTTP）
  config/policy.toml
  playbooks/*.md
  docs/CONTRACT.md（本檔）、docs/bdd/*.md、INVARIANTS.md
  scripts/build-install.sh
```

依賴方向：`core ← server ← (cli, src-tauri)`；ui 只透過 Tauri IPC。**core 不 import tokio/sqlx**。

安裝簽章：`scripts/build-install.sh` 在登入鑰匙圈有「Kunsu Local Signing」程式碼簽署憑證（`KUNSU_SIGN_IDENTITY` 可覆寫）時用它簽
app，designated requirement＝identifier＋憑證，跨版本不變，macOS 的「桌面／完全取用磁碟」與 Keychain 授權只要給一次。沒有時退回
ad-hoc（requirement 是 cdhash，每次重建都被當成新 app、授權失效、vault 檔案呼叫卡在權限對話框），並印出建立憑證的步驟。

## 2. 資料模型（core 型別，serde 全部 snake_case）

```rust
Question {
  id: String,              // 穩定 dedup id（沿用 pa-*；upsert 鍵）
  title: String,           // ≤80 字
  question_line: String,   // ≤24 字決策句（蓋板大字）
  body_md: String,
  visual_symbol: Option<String>, // ≤4 Unicode 字元；左側粒子圖形，缺省依 source.agent
  kind: Kind,              // choice | text | confirm | kongko_review | batch（題組，§14）
  tier: Tier,              // t0 | t1 | t2 | t3（app 守門後的生效值）
  requested_tier: Tier,    // agent 原始請求值（分級準確率計分用）
  product: Option<String>, // 使用者自訂的產品代號（例如 course-a、demo-shop）；account＝帳戶層級
  gate: Option<Gate>,      // yellow | red（沿用 meta-ads 語義）
  evidence: Vec<EvidenceLine>,   // ≤3 條 { text, source }（source=身世標籤，必填）
  params_line: Option<String>,   // 動錢題必填：精確變更一行（所見即所簽）
  options: Vec<Opt>,       // { id, label, detail?, recommended?: bool }；choice 題 2..=4
  allow_free_text: bool,   // 恆 true
  money: Option<Money>,    // { is_money_op, amount_twd?, entity_id?, executor_prepare_cmd? }
  decision_ref / prediction_ref: Option<String>,  // vault 相對路徑
  kongko_ref: Option<String>,    // batchId；發佈事件自動結案鍵
  deep_link: Option<String>,     // 開 Kongko / 開檔案
  draft: Option<DraftRef>,       // 草稿註冊題（§12）：{ revision, content_hash: "sha256:…", canvas?: tsx|jsx|html, registered_at }
  items: Vec<BatchItem>,         // 題組（kind=batch，§14）1..=20 項；其他題型為空（不序列化）
  rejected: Option<Rejection>,   // 使用者按「這題不該問我」：{ at, reason? }；跟著終態快照進帳本，重建後仍在
  expiry_after_h: Option<u32>,   // 建題當下的「一般題到期時間」（policy.expiry.t1_default_after_h 快照，2026-10-09）
  max_pending_d: Option<u32>,    // 建題當下的「題目最長等待天數」（policy.expiry.t2_supersede_after_d 快照，≤14）
                                 // 兩者 serde 預設 None＝舊題沿用目前 policy；建題寫入、upsert／草稿重新註冊不改
  default_action: DefaultAction, // { on_expire: noop_log|escalate_discord|pick_option, option_id? }
  deadline_at: Option<Ts>, wake_on_answer: Wake,  // { mode: none|resume|fresh|continue, session_id?, playbook? }
  source: Source,          // { agent: claude|codex|other, session_id?, cwd, routine_id?,
                           //   host?: claude_desktop|claude_cli|codex_app|codex_cli|other,
                           //   conversation_id?, host_session_id?, attended?, cli_path?,
                           //   trigger?: scheduled|interactive, routine_name?, routine_kind?, expected_every_hours? }
  state: State, snooze_count: u8, forced_today_used: bool, // 後兩欄只保留舊資料/遙測相容
  cycle_started_at / warn_at / force_at: Option<Ts>,
  cycle_number: u32,      // 新題=1；每次未回答 dismissal +1
  first_proposed: Date,    // upsert 不動
  last_seen / created_at / next_transition_at: Ts,
  answer: Option<Answer>,  // 見下
}
Answer { answer_id: String,     // uuid v4，作答當下生成；drain/reactor 的冪等鍵
         choice_id?: String, free_text?: String, edited_params?: String,
         channel: Channel,  // ui | kongko | discord | cli | auto_policy
         answered_at: Ts, signature?: String,   // UI 簽章（approval 題才有）
         acked_at?: Ts, executed_at?: Ts, execution_result?: String,
         delivery?: { state: pending|held|resuming|live_waiter|resumed|picked_up|fresh|no_continuation|dead,
                      reason?, conversation_link?, updated_at },   // 回答延續原對話（§8）
         report?: { status: done|failed|blocked, summary_md, reported_at },  // ask_report；永不改答案
         revision?: u32, content_hash?: String,   // 作答當下顯示的草稿版本（§12）
         items?: Vec<ItemDecision> }  // 題組逐項決定（§14）；題組的 free_text＝整體指示
```

**2026-07-31 P1 收貨後補的契約定案（server/ui builder 必讀）**：
1. `Question` 加欄位：`irreversible?: bool`（不動錢但不可逆時標 true）、
   `standing_policy_ref?: String`（agent 建題時比對 vault policies 命中才填）、
   `tier_downgrade_reason?: String`（server 守門降級時回寫）。
2. **T0 分級程式化判準**（requested_tier=t0 → 生效 t0 的必要條件，缺一降 t1 並寫
   tier_downgrade_reason）：`(money.is_money_op || irreversible)` && `params_line` 有值 &&
   `prediction_ref` 有值 && `deadline_at` 有值 && options 含安全預設（至少一個非執行型選項）&&
   內容驗證通過。tier 只表示風險與排序，**不再決定是否進入 15 分鐘 force cycle**。
3. **T3 standing policy 資料模型**：政策正本＝vault `logs/asks/policies/<slug>.md`
   （frontmatter: policy_id/signed_at/signing_question_id/condition/action/revoked_at?）。
   **比對引擎在 agent 端**（建題時讀 policies 命中才填 standing_policy_ref），server 只負責：
   帶 standing_policy_ref 的題過期時執行 default_action、channel=auto_policy、Discord 留痕、
   標 24h 否決窗（`veto_until` 欄位；可逆操作在窗內可一鍵 revert）。政策簽署本身是一張 T0 題
   （由 agent 建）。server 偵測「同型題同方向答 2 次」提案政策＝後置，不在本次建造。
4. `/health` 回應加 `oldest_pending_days`（心跳行需要）。
5. `/events/kongko` 對同 batchId **冪等**：重複事件不重複結案、不重複生成啟用題（at-least-once
   投遞是 Kongko 端語義）。
6. 檔案 inbox 格式：與 ask_create arguments 完全相同的 JSON、一檔一題、檔名
   `<timestamp>-<id>.json`；ingest 後移 `~/.kunsu/inbox/processed/`；壞檔隔離移
   `~/.kunsu/inbox/quarantine/` + Discord 告警，不阻塞其他檔。
7. Ledger type 白名單七值：`answered | expired | cancelled | superseded | approval_consumed |
   execution | delivery`（與 vault logs/asks/ASKS-SCHEMA.md 一致）。`delivery`：交付狀態改變（同狀態重檢不算）時追加，
   帶當下的題目快照（含 `answer.delivery`、`acked_at`）。
8. `ask_create` 尊重 client 提供的 `first_proposed`（積壓天數誠實原則）；未提供才填今日。
9. `ask_wait` timeout_seconds > 540 → 400 拒收。
10. doctor 測試題帶 `internal: true` 標記：不佔 T0 預算、不推 Discord、reactor 只 dry-run、
    ledger 行標 internal 供統計排除。

**Server 端硬驗證（400 拒收，錯誤訊息給 agent 可讀原因）**：
- enum 全白名單；`money.is_money_op && prediction_ref.is_none()` → reject
- `money.is_money_op && default_action.on_expire == pick_option` → reject（錢永不因超時自動動）
- `gate == red && kind != choice` → reject（紅燈必須多選項拍板）
- evidence 每條缺 `source` → reject；`question_line` 非決策句不驗語義但 >24 字 reject
- `visual_symbol` 超過 4 個 Unicode 字元 → reject；只控制受限粒子輪廓，不得承載 URL/CSS/script
- 十秒可決 lint：choice 題無 options、動錢題無 params_line → reject 標 `malformed`
- 動錢題必須是 choice 且有一個執行型選項（`recommended: true`）；純確認題（沒有選項）動錢一律 reject——按「確認」永遠不會跑錢鏈，
  會讓 agent 誤以為已批准
- 非紅燈的動錢題（黃燈或沒有 gate）必須帶 `money.executor_prepare_cmd`，否則 reject `money_needs_prepare_cmd`——沒有 prepare 命令
  錢鏈不會跑，使用者按「批准並執行」等於被靜默吞掉；不能一鍵執行的錢題請設 `gate: red`（只記錄決定，由使用者本人執行）
- 作答載荷（`answer_question`，400 `BadAnswer`，壞載荷不終結題目）：`choice_id` 與 `free_text` 至少一個；`free_text` trim 後
  非空（選項＋空白補充視為只有選項）；`choice_id` 必須是本題 options 的 id。唯一例外：`kind = confirm` 且沒有 options 時，
  `choice_id = "confirm"` 代表「確認」（決策面板 AC-DS-15）——不會觸發錢鏈（錢鏈只認黃燈錢題的執行型選項），交給 agent
  的訊息顯示為「確認」。
- 原對話身分（`bad_conversation_identity`）：`source.conversation_id` 與舊式 `wake_on_answer.session_id` 長度 ≤128、
  只能是 `[A-Za-z0-9_-]`、不得以 `-` 開頭（防止被當成 CLI 旗標，例如 `--last`）；Codex（agent 或 host 為 codex）的
  `conversation_id` 必須是 UUID；`host_session_id` 必須是 `local_[A-Za-z0-9-]{1,80}`

## 3. 狀態機（core 純函數；15 分鐘循環 vNext）

`transition(q: &Question, event: Event, now: Ts, policy: &Policy, guards: &GuardState) -> (State, Vec<Effect>)`

狀態：`created → pending → warned → forced → pending ... → answered | expired | cancelled | superseded`；
`snoozed` 只為既有 SQLite/ledger 快照相容，新流程不再產生；已回答的題投遞完成（`ask_answers_since` 收割）後標 `routed`，
過期、取消、被取代收割後保持原終態（事件照常回傳；歷史、督導與重建前後一致）。

每個非 internal、非終態題保存 `cycle_started_at`、`warn_at`、`force_at`、`cycle_number`：新題
`cycle_number=1`、`warn_at=created+12min`、`force_at=created+15min`。時間真相是 UTC wall-clock
timestamp，不是 UI interval。

轉移規則：

| 從 | 事件/條件 | 到 | 備註 |
|---|---|---|---|
| pending | `warn_at` 到（建題/新週期後 12min） | warned | 顯示「3 分鐘後將開啟強制詢問」；不受 tier/預算/guards 排除 |
| warned | `force_at` 到且 guards 全綠 | forced | 顯示唯一全螢幕 overlay |
| warned | `force_at` 到但 guard 命中 | warned | overdue/deferred；短輪詢，週期時間戳不動 |
| forced | 未回答 dismissal（關閉/Esc/稍後） | pending | dismissal event；cycle_number++；新完整 15min |
| forced | 顯示中 guard 轉為命中 | warned | HideOverlay；保留已逾時 force_at，解除即恢復 |
| legacy snoozed | tick | pending/warned/forced | 依既有/補建 cycle timestamp 正規化，不再套次數封頂 |
| 任一非終態 | 作答（任一通道） | answered | |
| 任一非終態 | deadline_at 到 | expired | 執行 default_action effect |
| 任一非終態 | agent cancel/supersede | cancelled/superseded | 不刪、留軌跡 |

Guards：`{ screen_shared, camera_on, mic_on, frontmost_fullscreen, quiet_hours, system_dnd, kunsu_dnd_until }`
任一 true → 不禁 pending→warned；順延不取消。quiet hours 預設 23:00–09:00。Kunsu 自身勿擾另在
shell presentation gate 禁止 notification/capsule/overlay 三種主動 surface；system DND 仍只保證禁
forced overlay（作業系統可自行處理 notification）。

**Kunsu 勿擾持久化：** `kunsu_dnd` 儲存在 SQLite settings 單列 JSON
`{starts_at, ends_at, actor}`，服務啟動時載入；`now >= ends_at` 視為 inactive 並可惰性清除。
restart/sleep 不調整 ends_at。舊 kill-switch IPC／快捷鍵升級為 `dnd_set(240)` 相容入口，不再維護
獨立記憶體 timer。
T1 48h 未答走 default_action；T2 14 天自動 superseded；其他 tier 到最長等待天數走 expired（INV-14）。到期判斷一律用題目上
建題當下固定的 `expiry_after_h`／`max_pending_d`（`Question::pin_expiry`，建題時由當下的生效 policy 寫入；同 id upsert 與草稿
重新註冊保留原值），沒有快照的舊題才用目前 policy——設定頁之後改到期參數只影響新題（INV-26，2026-10-09）。timer 所有時限落 DB
`next_transition_at`，server 每秒掃 due rows；重啟與 sleep/wake 從 DB timestamp 恢復。

同時多題逾時時，每題週期獨立，但 shell 全域最多一個 overlay window；清單排序為最早逾時、
風險 tier、建立時間，回答後可在同一 flow 進下一題。

## 4. MCP tools（14 個，名稱即 tool name）

`ask_create` `ask_register` `ask_wait` `ask_answers_since` `ask_list` `ask_get` `ask_update` `ask_cancel`
`ask_supersede` `ask_report` `ask_followup_reply` — schema 依 §2；細節：
- `ask_register {draft_dir, dry_run?, source?}`：草稿註冊（§12）→ `{ok, dry_run, question_id?, revision?, dedup?:
  created|updated|unchanged, state?, preview_png?, issues[], answer?}`；`issues[]` 每條 `{file, line?, column?, code,
  message, hint}`。`source` 與 frontmatter 的 `source` 合併，呼叫端有值的欄位優先。
- `ask_create`：同 id upsert=只 touch `last_seen`/證據，不重置倒數不發通知，回 `{question_id, state, dedup: created|touched_existing}`；per-source 每日建題配額（policy，預設 12）超額回錯誤。
- `ask_create`：`source.conversation_id`（或舊式 `wake_on_answer {mode: resume, session_id}`）存在且非 internal →
  server 一律把 `wake_on_answer.mode` 設為 `continue`（回答延續原對話，§8）。
- `ask_wait {question_id, timeout_seconds<=540}` → `{status: answered|expired|cancelled|pending_timeout|followup, answer?, delivery?, followup?, hint?}`；無狀態可迴圈續等。
  `followup`：使用者在手機上問本人，`followup {id, text}` 每則只交一次；用 `ask_followup_reply {question_id, followup_id, text≤4000}` 回答（只轉文字給手機，INV-31），回完續等（§17）。
  呼叫期間登記為該題的等待者（離開後 10 秒租約）；登記以 RAII 釋放，連線被切斷或呼叫被取消也會離場。
  只有適用回答延續原對話的題（有原對話身分、非 internal）回 `answered` 時才認領交付 `live_waiter`，並附
  `delivery`（最終交付去向；交付已被別條路徑定案時，遲到的等待者看得到答案去了哪裡）；其餘題維持現行 wake。
- CLI `kunsu wait <id> [--timeout N]`（`--timeout 0` 不限總時長）：迴圈呼叫 `ask_wait`；連不上 app 以 1→30 秒退避重試；
  發問對話的行程（`$CLAUDE_PID`）不再存活，或直接父行程變成 pid 1 → 結束碼 4；答案已（正）由 resuming／resumed／fresh／
  picked_up 處理 → 印出結果並以結束碼 5 提醒不要重複處理；其餘答案結束碼 0。
- `ask_report {question_id, answer_id, status: done|failed|blocked, summary_md≤20000}`：原對話處理完後回報結果與
  AI 回饋 → 寫 `answer.report`、`executed_at`（沒有才寫）、補 `acked_at`，ledger 追加 `execution` 行；**永不寫
  `execution_result`**（那是 Kunsu 自己的執行紀錄：錢鏈結果與交付文字）。同內容重送冪等；未作答或 answer_id 不符 400。
  寫完帳本後（同一段寫入鎖內），只有適用回答延續原對話的題、交付仍 pending／held 時改為 `picked_up`（另有自己的
  `delivery` 帳本行；重送時同樣補做）；現行喚醒的題不動交付。永不改答案或題目狀態。
- `ask_answers_since {consumer, cursor?}` → `{events[], next_cursor}`；游標 per-consumer 存 DB；at-least-once。
- agent 讀取面去敏：`ask_get`／`ask_list`／`ask_wait` 回傳的答案不含 `signature`；錢題的 `execution_result`（錢鏈原始
  輸出，含 approval 路徑與 token）**一律**換成人話狀態（已執行完成／被安全護欄擋下／乾跑，未實際執行／已送出），不論
  agent 是否已回報；agent 自己的回饋在 `answer.report`，照原樣。`ask_create`／`ask_register` 重送已答題時回傳的既有答案同樣去敏。只有
  `ask_answers_since`（Lane 1 drain 要把簽章回寫 pending-approvals）保留原貌。
- 錢題的 `ask_wait`：答案已寫入但錢鏈（該答案的 reactor 列）還沒跑完時不回 `answered`，跑完才交付（答案帶人話錢鏈狀態）。
- 錢鏈還在跑時（錢鏈適用、該答案的 reactor 列還是 queued／executing，而且這一輪的錢鏈結果還沒寫進執行紀錄），`ask_get`／`ask_list`／
  `ask_wait` 與追加訊息一律說「錢鏈還在跑，以 Kunsu 回執為準」——commit 正在跑、executor 已驗證但還沒回報的空檔不是「結果不明」（§7）。
  同一列在錢鏈跑完之後接著交付（在原對話追加）時，錢鏈已經不在跑：讀取面照錢鏈結果說（例如「已執行完成」），跟追加訊息一致。
  「這一輪的錢鏈結果已寫入」＝執行紀錄以錢鏈結果開頭（`reactor::is_money_result`），且 `executed_at` 不早於這一輪的授權時間
  （`reauthorized_at`，沒有就是 `answered_at`）。
- `ask_answers_since` 收割回答延續原對話的題時，交付還沒到原對話（pending／held／resuming／dead）就不補 `acked_at`。
- **MCP 面永不暴露作答/批准寫入**——answer 只能來自 Tauri IPC（UI）、`/events/kongko`（發佈即同意）、Discord 讀回（後述）、CLI `kunsu answer`（限使用者本人終端使用，isatty 檢查）。

勿擾工具：

- `dnd_get` → `{active, starts_at?, ends_at?, remaining_seconds, actor?}`。
- `dnd_set {duration_minutes, actor?}` → 驗證 `1..=1440`，以 server wall clock 設定／覆寫
  `starts_at`、`ends_at`，回完整 status；不接受絕對遠期 timestamp。
- `dnd_clear {actor?}` → 提前解除並回 inactive status；冪等，已 inactive 仍成功。
- 勿擾 tools 只修改 presentation guard，不碰 question state/answer/authorization；每次有效 set/clear 以
  `dnd_changed` event append actor、舊／新 ends_at。三工具與其他 MCP 一樣需要 Bearer token。

## 5. HTTP（非 MCP）

- `GET /health` → `{version, uptime_s, pending_count, overdue_count, last_answer_at, oldest_pending_days, ledger_backlog,
  open_json_dirty, ledger_stuck_s}`（無 auth，loopback）。後三個是 app 行程自己的帳本狀態（待補寫行數、OPEN.json 待重寫、
  在途寫入卡住秒數，沒卡住為 0）；`kunsu doctor` 印出 `ledger_backlog` 一行，有積欠時標 ⚠️（不計入失敗，不觸發安裝回滾）；app 沒開時改以唯讀方式打開資料庫
  （`KUNSU_DB_PATH` 或預設路徑）數待補寫行數，打不開就寫「無法確認，刪資料庫前先打開 Kunsu 再跑一次」；app 版本較舊、`/health` 沒有這些
  欄位時也寫「無法確認」，不當成 0。
- `GET /describe` → tool schemas + 狀態機轉移表 + 當前 policy（machine-readable）
- `POST /events/kongko`（Bearer）→ `{type:"ad_publication_confirmed", publicationId, batchId, product, ads[], confirmedAt}`；`kongko_ref==batchId` 的題 → answered(channel=kongko)；隨後自動建獨立 T0 啟用題（發佈≠啟用）
- `GET /events`（WS，Bearer）→ 每個狀態轉移一個 JSON frame（Monitor 預武裝用；`?question=<id>` 過濾）；執行事件不帶
  executor token（只說「已取得執行憑證」）
- `POST /approvals/verify`（Bearer）→ executor.py 驗證通道：`{approval_record}` → `{valid, consumed_now, reason?}`；單次消耗記 DB+ledger。
  除了驗章、TTL、簽發登記與題目狀態，再多兩條拒絕（2026-10-09 錢 QA 第 11 輪，INV-9）：這個 nonce 在 `money_outcomes` 已經有任何
  結果（例如 Kunsu 已記成 `EXEC_BLOCKED`）；或這題有比它更新的簽發 nonce（被重新授權取代）。兩條都在消耗的同一段寫入鎖內判斷，
  與錢鏈記 `EXEC_BLOCKED` 互斥——Kunsu 已判定結束或被取代的授權，事後永遠花不掉。

## 6. 錢的授權鏈

1. 建題帶 `money.executor_prepare_cmd`（完整 prepare 命令字串，不預先執行）。
2. 使用者點批准 → server spawn 該 prepare 命令（stdin＝/dev/null，跟其他沒有輸入的 spawn 一樣不繼承 Kunsu 的 stdin）→ 解析 stdout 取 token → 組 approval record：
   `{question_id, executor_token, param_hash, answered_at, nonce}`，`param_hash = sha256(params_line 正規化)`；
   HMAC-SHA256 簽章（Keychain key）。落檔 `<vault 根>/projects/meta-ads/data/executor_pending/approvals/<question_id>.json`。
3. reactor 跑 `executor.py ... commit --approval-file <path>`。executor 端**已實作完成**
   （2026-07-31，主 agent 親改，9 測試綠）：讀檔 → 欄位齊全 → executor_token 綁定 → TTL
   （answered_at 起 15 分鐘）→ param_hash 重算相符 → `POST /approvals/verify`（fail closed）
   → 走既有 guard/漂移檢查 → fire。**舊 isatty 通道一行未動**。
   **param_hash 正規化定義（server 必須逐字元一致，見 executor.py normalized_param_hash）**：
   - update_budget：取 spec 的 `{action, entity, product, new_budget, current_budget}`
   - update_field：取 spec 的 `{action, entity, product, field, new_value, current}`
   - `json.dumps(obj, sort_keys=True, separators=(",",":"), ensure_ascii=False)` → sha256 hexdigest
   - approval record 欄位：`{question_id, executor_token, param_hash, answered_at(epoch 秒或
     ISO8601), nonce, hmac}`；verify 請求體 `{"approval_record": {...}}`
4. 威脅模型註記：HMAC key 存 Keychain ACL 限 app；verify 走 app 活體 DB 單次消耗；就算全被繞過，
   executor 既有護欄（NT$2,000 紅線、≤20% 偏離、3 天冷卻、TWD 50,000 硬上限）是獨立最後防線。
5. 一鍵通道只覆蓋 gate=yellow；red 題答案落 ledger 後執行仍走使用者本人終端或互動 session。

**QA 收貨後補的錢鏈硬規則（2026-07-31，上線前必修）**：
6. **錢鏈觸發通道白名單＝channel:ui 獨佔**：只有 Tauri IPC（使用者物理點擊）作答會觸發
   prepare→簽章→commit 鏈。`/cli/answer`、未來任何 Discord/ingest 面、kongko、auto_policy
   對 `money.is_money_op` 題**一律拒絕作答**（回明確訊息「錢題只能在 Kunsu app 點擊批准」）
   ——否則本機 Bearer token 洩漏＝繞過物理點擊動錢，整個設計的核心承諾就破了。
   （INVARIANTS 新增第 15 條 `inv_money_chain_ui_only`。）
7. **編輯後所見即所簽全鏈成立**：edit_option / edited_params 改動金額時，server 必須同步改寫
   `money.executor_prepare_cmd` 的數值引數（prepare-budget --new-budget N / prepare-bid
   --new-value N 兩型的目標數字）；無法可靠改寫（格式不認得）→ 鎖一鍵（升 red 語義）。
   如此點擊當下 prepare 產的 spec 即編輯後值，param_hash 自然綁到使用者看到的數字。
   （INVARIANTS 第 16 條 `inv_edited_params_rewrite_prepare_cmd`。）
8. **T1 批次窗僅保留相容 schema**：`windows.t1_batch`、`next_t1_batch` 與
   `ServerEvent::BatchWindow` 暫留給舊設定／舊 client 反序列化；timer 不再發出 BatchWindow，
   T1 與其他合格題一樣只走各自持久化的 15 分鐘 force cycle。
9. **24h 無 ack 告警**：timer 日掃 answered 且 acked_at 空且 >24h → Discord「⚠️ 你答的 X 沒人
   處理」+ 記已告警防重複。
10. **疲勞降載落地**：create_question 時查 7 天滑動窗同型題（product+category）略過率 >
    policy.fatigue 閾值 → tier 自動降一級寫 tier_downgrade_reason + Discord 承認（週一次）。
11. `ask_update` patch 開 deny_unknown_fields（未知欄位 400，不靜默忽略）。
12. Discord 讀回＝agent 側 SOP（mkt-daily drain 步驟讀 Discord 回覆），對非錢題經
    `/ingest/discord-answer`（Bearer、channel=discord、錢題拒絕）落地；錢題的 Discord 回覆
    只轉述進日報、執行仍等 app 點擊。Kongko reconcile 兜底＝mkt-daily SKILL 步驟（agent 經
    Kongko MCP 查 AdPublication diff ledger、漏的補打 /events/kongko）。

## 7. Ledger（vault 真相層）

- 終態當下 append `logs/asks/YYYY-MM-DD.jsonl`：`{ts, type, row_id, question:<全量快照>, answer?, execution?}`。`row_id` 是每行
  唯一的 uuid（舊行沒有）；同一行可能因逾時後補寫而出現兩次，重建時同一個 `row_id` 只認第一次出現。
- 每次變更重寫 `logs/asks/OPEN.json`（pending 快照，含 days_pending 動態欄不落盤原則之外的例外：不落，讀取端算）
- `ASKS-SCHEMA.md` 正本由 integration agent 落在 vault；enum 白名單與 §2 一致
- `kunsu rebuild-db`：ledger + OPEN.json → 完整重建 SQLite（含清空 `deliveries`）。**先完整讀完並解析** OPEN.json 與所有
  帳本檔，才清資料庫：目錄或檔案讀不到（權限、逾時）一律拒絕重建（`exists()` 失敗不能當成「沒有檔案」）；資料庫裡
  任何一題非 internal 的題目在 OPEN.json 與帳本都找不到時也拒絕（重建永不丟掉資料庫裡的題目，錯誤列出前 10 個 id）。
  拒絕一律回 `RebuildRefused`（HTTP 409，`code: rebuild_refused`），訊息寫明「資料庫保持原樣」與原因。
  錢題若已有 `approval_consumed` 卻沒有執行紀錄，不論作答多久，都不重新排入，發 Discord「重建後需要人工對帳」（先於
  24 小時窗判斷）。其餘已回答、需要喚醒、沒有交付紀錄也沒有執行紀錄、**而且 24 小時內作答**的答案重新排進 reactor；
  帳本還原出未定案交付（pending／held）的答案也套同一個 24 小時窗：超過的不排重檢。更舊的都不自動重做，列在重建報告
  （`stale_unexecuted` 數字＋`stale` 清單：id、標題、作答時間；人工對帳的列在 `reconcile_items`）並發一則 Discord 請
  使用者確認；這些題同時補上「已告警」標記，重建後不再逐題發「24 小時沒有任何 agent 處理」。wake=none 的答案（非 internal）重建時跟作答當下一樣登記待決交付，下一次 reactor 掃描定案 `no_continuation`（等待者不影響，與作答當下相同）。被擋的錢題（`EXEC_BLOCKED`）
  重建後可重新授權（沒有 reactor 列時重新授權會建立一列）；但「錢的結果不明」的錢題一律拒絕重新授權——錢可能已經動過，只能人工對帳（重建前後都一樣）。「結果不明」以**每一次授權（nonce）**
  判斷，不看時間：這題任何一個已被 executor 消耗的 nonce，在 `money_outcomes`（nonce → 結果，跟著 `execution` 帳本行的
  `money_outcome {nonce, result}` 重建）裡沒有結果、或結果是 `EXEC_UNKNOWN`，就是結果不明。錢鏈對每個簽出的 nonce 記一筆結果
  （`COMMIT_OK`／`EXEC_BLOCKED`／`EXEC_UNKNOWN`）；之後交付、追加、回報改寫執行紀錄或時間都不影響這個判斷。錢鏈結果的開頭一律是
  `COMMIT_OK`／`EXEC_BLOCKED`／`EXEC_UNKNOWN`／`RECONCILED_EXECUTED`／`RECONCILED_NOT_EXECUTED`／`DRY_RUN`／`APPROVAL_SIGNED`
  （`reactor::is_money_result`）。重建時同一個判斷決定人工對帳（未解決的 `EXEC_UNKNOWN` 也列；已記成 `EXEC_UNKNOWN` 的當下已告警，
  只列報告不再發 Discord）。錢鏈還在跑（錢鏈適用、reactor 列 queued／executing）的空檔不算結果不明：executor 已驗證、commit 還沒
  回報時讀到的是「錢鏈還在跑」（§4），人工對帳也被拒。`mark_reactor_done` 不覆寫 `needs_reconcile` 的列。
  **舊資料補結果（2026-10-09 錢 QA 第 11 輪）：** migration 0007 之前沒有 `money_outcomes`，舊資料庫升級後、或用舊帳本（沒有
  `money_outcome` 行）重建後，每一題一鍵執行過的錢題都會變成「已消耗卻沒有結果」。`ServerHandle` 啟動時、以及 `rebuild-db`
  重放完帳本之後（判斷人工對帳之前），跑同一個冪等補資料（`backfill_money_outcomes`）：對「已被 executor 消耗卻沒有結果」的 nonce，
  若這題執行紀錄開頭是 `COMMIT_OK` 或 `RECONCILED_EXECUTED`，補上同樣開頭的結果（`<開頭>: 補資料：…依執行紀錄開頭推定`），
  並寫一行 `money_outcome {nonce, result, backfill: true}` 的 `execution` 帳本行（下次重建直接讀到，不必再補）。推論是安全的：
  一旦 `COMMIT_OK`／`RECONCILED_EXECUTED`，重新授權永遠被拒，之後不會再有新的消耗，那個沒有結果的消耗就是產生這個結果的授權。
  其他開頭（例如 8/6 那題的 `EXEC_FAILED: 死信…`）不補，維持結果不明、走人工對帳。同一題有兩次以上已消耗卻沒有結果的授權時也不補：
  上面的推論只對產生那個結果的那一次成立，較早那次（舊版把驗證後失敗記成被擋、又重新授權）的結果真的不明，整題維持結果不明，由使用者
  對帳。同一步也處理舊版本的重啟恢復留下的錢鏈列：
  錢題的 `needs_reconcile` 列若沒有結果不明的 nonce、錢也沒有執行完成（舊版本把 executor 驗證前中斷也標成 needs_reconcile），
  照下面重啟恢復第 2 種的資料處理（記 `EXEC_BLOCKED`、列改 `dead`），不重發 Discord。
  **重啟恢復（app 重啟時卡在 executing 的錢鏈列，`Reactor::process_pending` 開頭）：** 依這題每一次授權的狀態分三種——
  1. 有已被 executor 消耗卻沒有結果的 nonce（executor 已驗證、可能已經打了 API）→ 那些 nonce 記 `EXEC_UNKNOWN`、執行紀錄寫
     `EXEC_UNKNOWN: Kunsu 在執行中途中斷…`、reactor 列 `needs_reconcile`、Discord `reactor_needs_reconcile` 請使用者在回執人工對帳；
     agent 讀取面、等待者、追加訊息與回執都一致讀到「結果不明」。
  2. 沒有（executor 還沒驗證，或還在 prepare），錢也沒有執行完成 → 這題所有簽發但沒有結果的 nonce 記
     `EXEC_BLOCKED: Kunsu 在 executor 驗證授權之前中斷，錢沒有動`，執行紀錄寫同樣內容、刪掉 approval 檔（best effort）、reactor 列
     `dead`（不是 needs_reconcile）、Discord `money_interrupted_before_verify`「錢沒有動，可以在回執重新授權」；之後 `/approvals/verify`
     對這些 nonce 一律拒絕（§5），殘留的 commit 子行程晚到也花不掉；`money_state` 是 `reauthorizable`；回答延續原對話的題照被擋的
     規則通知原對話。
  3. 錢已經執行完成（執行紀錄開頭是 `COMMIT_OK`／`RECONCILED_EXECUTED`），中斷在之後的交付 → 跟不動錢的題一樣：交付卡在
     `resuming` 就走交付中斷回收（§8，已回報 → resumed；否則 dead＋一則 `delivery_interrupted`），不發錢鏈對帳訊息；
     不能對帳、也不能重新授權。
  **人工對帳（只有使用者在 UI）：** IPC `resolve_money_outcome {question_id, executed: bool}`（只收主視窗與蓋板）——使用者核對後標
  「錢已經動了」或「錢沒有動」：把結果不明的 nonce 記成 `RECONCILED_EXECUTED`／`RECONCILED_NOT_EXECUTED`（帳本有行）、執行紀錄改成
  同樣開頭、`needs_reconcile` 的 reactor 列結束成 done。**只接受確實有結果不明 nonce 的題**（reactor 列 `needs_reconcile` 本身不算；
  某個 nonce 已是 `COMMIT_OK` 的題永遠推翻不了）；錢鏈列還在 queued／executing 時直接拒絕。標「沒有動」之後才能重新授權（產生
  新的 nonce）；標「已經動了」等同已執行完成，永不能重新授權。agent 讀到「已執行完成（使用者人工對帳確認）」或「沒有執行（使用者人工
  對帳確認）」。
  **重新授權（只有使用者在 UI）：** IPC `reauthorize_question {id}`（只收主視窗與蓋板）。全部成立才放行：UI 通道作答的錢題、
  非紅燈、答案是執行型選項（`recommended`）且題目有 prepare 命令（錢鏈適用）；這題**沒有任何 nonce** 的結果是
  `COMMIT_OK`／`RECONCILED_EXECUTED`，執行紀錄開頭也不是；錢鏈列不在 queued／executing；沒有結果不明的 nonce；執行紀錄開頭是
  `EXEC_BLOCKED`（含 executor 驗證前中斷）或 `RECONCILED_NOT_EXECUTED`——錢鏈明確沒有動錢（乾跑、從沒跑過的舊答案不給：重新授權不是
  補跑舊決定的通道）；reactor 列是 done／dead（或重建後沒有列，會建立一列）。
  **錢的狀態 `money_state`（伺服器算，`ServerHandle::money_state`）：** 已作答的錢題才有——
  `running`（錢鏈適用、reactor 列還是 queued／executing，含錢鏈跑完後同一列的交付：列結束前人工對帳與重新授權一律被拒，所以不給按鈕）→
  `unknown`（列已結束、有結果不明的 nonce；可人工對帳）→
  `reauthorizable`（上面重新授權的所有檢查都過）→ 其餘 `settled`。依序取第一個成立的。shell 送給網頁的題目（`list_questions`、
  `get_question`、作答、重新授權、對帳等回應）附上這個欄位，網頁一律依它決定回執的按鈕（§9）。
  executor commit 失敗或逾時時，先查這次的 nonce 是否已被 executor 消耗：已消耗＝executor 已驗證、可能已經打了 API，記成
  `EXEC_UNKNOWN: …`、發 Discord `execution_unknown` 請使用者人工對帳，agent 讀到「結果不明：executor 已用掉授權但沒有回報
  結果，錢可能已經動過，等使用者人工對帳」；沒消耗才是 `EXEC_BLOCKED`（護欄擋下）。查消耗與記 `EXEC_BLOCKED` 在同一段寫入鎖內
  （`settle_unverified_nonce`），與 `/approvals/verify` 的消耗互斥：誰先誰贏，記成被擋的 nonce 之後一律驗證失敗。記 `EXEC_BLOCKED`
  時同時刪掉這次寫的 approval 檔（best effort，失敗只記 log；真正的保證是 verify 拒絕）。
  拒絕重建時，若帳本有無法解析的壞行，錯誤訊息列出前幾個壞行的檔名與行號。
- 錢題不論 `wake_on_answer` 都進 reactor（Answered 的 SpawnReactor 條件是 wake≠none **或** 錢題）：使用者批准的錢鏈一定會跑；
  wake=none 只表示不喚醒 agent。重建時 wake=none 的錢題也依同樣規則重新排入。
- `ask_wait`／`kunsu wait` 等到被使用者退回的題：回 `{status: "cancelled", rejected: {at, reason?}}`，等待中的 agent 知道是使用者
  推掉與原因（結束碼照舊）。
- 收割（`ask_answers_since`／`kunsu drain`／回執回寫）第一次替答案標 `acked_at` 時追加一行 `delivery` 帳本行（含 routed 狀態），
  重建後 `acked_at` 仍在，不會誤報「24 小時沒有任何 agent 處理」。
- **帳本待補寫佇列**：帳本寫入失敗（權限、逾時）時，該行存進 SQLite `ledger_outbox`（不屬於可重建資料），之後每次
  寫帳本前先依序補寫；佇列沒清空時 `rebuild-db` 先嘗試補寫，仍失敗就拒絕重建（避免丟掉只在資料庫裡的變更）。
  approval 消耗（INV-9）寫不進帳本時仍 fail closed，不排隊。同一時間最多一個在途的帳本寫入：上一個寫入還卡著時，
  新的寫入直接排進佇列、不再開執行緒（不會累積卡住的執行緒）；重建讀帳本也走同一個限制，寫入卡著時拒絕重建。
  在途讀寫卡住超過 2 分鐘：tick 對每一次卡住（以卡住的開始時刻區分）各發一次 Discord「帳本讀寫卡住，請重新啟動 Kunsu（答案都在資料庫，重啟後自動補寫）」，
  `/health` 的 `ledger_stuck_s` 顯示卡住秒數；寫入結束後解除。逾時錯誤分讀、寫兩種訊息。OPEN.json 重寫失敗時記下「待重寫」（資料庫 settings `open_json_dirty`，重啟後仍記得），tick 每 30 秒
  重試，待重寫時拒絕重建。已知限制：資料庫整顆刪除時佇列一起消失——刪資料庫前先確認 `kunsu doctor` 顯示帳本
  沒有積欠。

## 8. Reactor

- 串行 queue（一次一個），answered 且 `wake_on_answer.mode != none` 觸發。錢鏈先跑（§6），再走交付。
- **交付階梯（回答延續原對話，PRD §20）**：純函數 `kunsu_core::continuity::next_step`；`deliveries` 表每答案一列，
  `pending`／`held` 之外的狀態一旦寫入永不改變（`inv_single_continuation`）；唯一例外是 reactor 自己的過渡狀態
  `resuming`：spawn 前先以 CAS 從 pending／held 佔成 `resuming`（佔不到 → 別條路徑已定案，跳過且不 spawn），
  命令結束再以 `WHERE state='resuming'` 轉成 resumed／held／fresh／dead。期間 agent 呼叫 `ask_report` 只記回饋、
  不改交付狀態。`resuming` 超過 15 分鐘（app 在追加途中中斷）不重做（避免同一答案送兩次）：agent 已回報 → `resumed`；
  否則 `dead`＋Discord「可能已送出，請打開原對話確認」（事件 `delivery_interrupted`，同題只發一次）。app 重啟時，
  不動錢且卡在 `resuming` 的 executing 列直接走這個回收，不發錢鏈的 needs_reconcile 訊息。
  1. 等待者在線或 10 秒租約內 → 不動，3 秒後重看；`ask_wait` 回答案時認領 `live_waiter`。
  2. Claude：桌面 session 紀錄 `isArchived: true` → 開新對話（原因「原對話已封存」）。登記檔
     `~/.claude/sessions/<pid>.json` 的 `sessionId`／`hostSessionId` 命中且 pid 存活 → `held`，每 60 秒重檢
     （`inv_never_resume_live_conversation`）。
  3. 否則在原對話追加：Claude `<cli> -p <prompt> --resume <id> --output-format stream-json --verbose
     --permission-mode bypassPermissions --max-budget-usd 2`，`<id>` 以 `host_session_id` 讀
     `~/Library/Application Support/Claude/claude-code-sessions/*/*/<local_…>.json` 的 `cliSessionId`（找不到用原 id），
     `<cli>` 僅在 `~/Library/Application Support/Claude` 下才採用 `source.cli_path`，否則 `which claude`；Codex
     `codex exec -s workspace-write -C <cwd> --json --skip-git-repo-check resume <thread> -`（prompt 走 stdin）。
     Codex resume 與 fresh 共用執行檔 resolver：測試注入 `codex_bin` 優先；macOS 其次採已安裝
     `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`（必須是一般檔案且可執行），
     再退回 PATH，最後才使用 bare `codex`。保持使用者目前模型設定，避免系統 npm CLI 版本落後桌面版。
     cwd = `source.cwd`（不存在則 vault 根）。所有 agent 子行程（追加與開新對話）spawn 前都清除 `CLAUDE_CODE_*`／
     `CLAUDE_PID`／`CLAUDECODE`／`CODEX_THREAD_ID`；子行程自成行程群組，超過 `exec_timeout`（預設 30 分鐘）整組
     中止並保留已收到的輸出；stdin 在背景寫入，同樣受逾時保護。
     prompt 含題目一句話、答案、answer_id、`ask_get`／`ask_report` 指示；錢題只附人話狀態，不含 approval 路徑、token、
     nonce 或簽章。成功 → `resumed`。
  4. 失敗分類（`kunsu_core::continuity::classify_resume_failure`）：agent 已開始處理（Claude stream-json 有 `assistant`
     事件、Codex `--json` 有 `item.*` 事件）、已 `ask_report`、或逾時 → `resumed`＋原因「原對話已開始處理，但結束時
     出錯／逾時中止」＋Discord `resume_partial`（已回報則不發），不重試、不開新對話。帶 `error` 欄位或 model 為
     `<synthetic>` 的 assistant 事件是 Claude 的 API 錯誤，不算開始處理。其餘只看 stderr 與錯誤事件
     （Codex `error`／`turn.failed`、Claude `result` is_error），不看 agent 的工作內容：Codex 的
     `already has an active writer` → `held`；`No conversation found`／`no rollout found` → 開新對話；其他重試 3 次
     （每次累加 `deliveries.attempts`）後開新對話。
     `source.cli_path` 必須解析（canonicalize）後仍在受信任根目錄下、是一般檔案且有執行權限，採用解析後的真實路徑。
  5. 開新對話（fresh）：spawn `claude -p --input-format text --output-format stream-json --verbose --permission-mode bypassPermissions --max-budget-usd 2`，prompt 走 stdin（不放進命令列參數：最大題組的 prompt 超過 1 MB，會撞 macOS ARG_MAX 而死信；`claude --help`：`--input-format` 只用於 `--print`、預設 text，2026-10-09），cwd=vault 根；prompt = playbook 檔內容 + `## 題目與答案\n<question+answer JSON>`（去敏：不含 `answer.signature`、`answer.execution_result`、`money.executor_prepare_cmd`），有原對話時另附「## 原對話」無法接續原因與連結；source.agent==codex → `codex exec - --json -s workspace-write -C <vault>`（prompt 走 stdin）。⚠️ 不經 zsh alias，旗標顯式帶；`claude` 路徑用 `which claude` 解析並快取。成功 → `fresh`（含原因）；新對話已開始處理或逾時 → `fresh`＋原因、`EXEC_FAILED`＋Discord，不重跑；還沒開始就失敗才
     重試，耗盡 → `dead`＋死信。
  6. 沒有原對話身分且 wake=none → `no_continuation`。
- 交付變更發 `DeliveryChanged` 事件（shell 轉 `kunsu://questions_changed`）。
- 產出回寫 `execution_result` + `executed_at`，一律寫入（AI 回饋另存 `answer.report`，兩者互不覆寫）；失敗重試 3 次指數退避
  → 死信 + Discord 告警。錢鏈已有結果（`COMMIT_OK`／`EXEC_BLOCKED`／乾跑）而交付進死信時（任何路徑：第一輪、held
  之後、等待者租約之後），死信一次寫成「<錢鏈結果>；交付失敗：…」，被擋就保留 `EXEC_BLOCKED` 開頭；已經
  `COMMIT_OK` 的答案拒絕重新授權（錢不會被再動一次）。agent 讀取面的錢題狀態依 `execution_result` 開頭判斷：
  `COMMIT_OK` 已執行完成、`EXEC_BLOCKED` 被擋、`DRY_RUN`／`APPROVAL_SIGNED` 乾跑、`EXEC_FAILED` 執行沒有完成；
  判斷順序：錢鏈適用、錢鏈列還在 queued／executing、這一輪的錢鏈結果還沒寫入 →「錢鏈還在跑，以 Kunsu 回執為準」（即使已有消耗卻還沒有
  結果的 nonce；錢鏈跑完、列還在交付時照錢鏈結果說，§4）；
  不在跑而有結果不明的 nonce →「結果不明…」（§7）；錢鏈不適用（安全選項、紅燈、非 UI 通道）一律「未觸發」，
  不看執行紀錄開頭；錢鏈適用但還沒有結果 →「錢鏈還在跑」；其餘依開頭。執行事件、`execution_blocked` 的 Discord 訊息與 `/events` 都先遮掉 executor token 與 approval 路徑。
- **錢鏈與交付**：錢鏈被擋（`EXEC_BLOCKED`）仍照樣交付，原對話只拿到「被安全護欄擋下」的人話狀態；held 之後
  才追加時，交付文字接在既有錢鏈結果後面（`COMMIT_OK…；已在原對話追加處理`），不覆寫。暫時狀態（等待者租約、
  held）且沒有錢鏈結果時不寫 `execution_result`，只結束 reactor 列。
- **acked_at**：現行 wake 在 reactor 起跑或 consumer drain 到即標。回答延續原對話的題只在真的有人接手時才標
  （`live_waiter`／`resumed`／`picked_up`／`fresh`，或錢鏈已執行）；`held` 不算接手，所以 24 小時告警照常觸發。
  answered（含 drain 後的 routed）後 24h 無 acked → Discord「⚠️ 你答的 X 沒人處理」；交付已記 `dead` 的題（中斷或死信
  當下已告警）不重複告警；錢題中斷已發錢鏈對帳訊息時不再發中斷訊息。
- 記憶候選：reactor 不自配 M-ID，只 append `projects/meta-ads/data/memory-inbox/<date>-<qid>.md`。

## 9. Tauri shell / UI

- 視窗三件套與 NSWindow level 依設計 §3（膠囊 `.floating`+全 Space；蓋板 `.popUpMenu`、非 macOS fullscreen mode）。主視窗是可由 tray 開啟的實體 app（待決／歷史／Agents／設定＋首次 onboarding）。
- **邊緣瀏海（2026-10-08，取代浮動膠囊；2026-10-09 QA 第 1–4 輪修正；`docs/bdd/edge-notch.md`）**：沿用 `capsule` 視窗
  （`index.html?view=capsule` 渲染 `EdgeNotch`）與既有存在規則（只有非 internal、非終態題且未勿擾時顯示；UI 端
  `notchQueue` 同樣排除 internal 與終態題，題數與跑馬燈都只算它們）。
  - **舞台視窗**：建窗 `focusable(false)`（`canBecomeKeyWindow`＝NO，永遠不成為 key window）；`show_capsule` 只在視窗
    隱藏時才貼合目標螢幕並顯示，已顯示時對帳不做任何事（`edge::stage_plan`）。目標螢幕＝`~/.kunsu/edge-notch.json`
    記住的螢幕（`edge::match_screen`／`pick_screen`）：螢幕識別是 CGDirectDisplayID（`NSScreen.deviceDescription` 的
    `NSScreenNumber`），先比識別；找不到或舊檔只記名稱時，名稱恰好對到一台螢幕才採用；都對不到就主螢幕。透明、無陰影、層級 26（選單列 24／狀態列 25 之上，下拉選單
    101 與蓋板之下）、CanJoinAllSpaces＋Stationary＋FullScreenAuxiliary＋IgnoresCycle，預設 ignoresMouseEvents。
    250 ms 主執行緒輪詢（`windows::poll_main_thread`）在舞台可見時比對 frame 與目標螢幕，不一致就重設 frame 並發
    `kunsu://edge_stage_changed`，網頁收到後重讀 `edge_info`。原生層記住舞台上次貼合的螢幕識別（有 `display_id` 用它，
    沒有才用名稱，與網頁 `stageIdentity` 一致）：`prepare_stage`（顯示前）與 `realign_stage` 貼合的螢幕識別和上次不同時，
    即使 frame 尺寸相同也發 `kunsu://edge_stage_changed`（`edge::stage_display_changed`），所以舞台隱藏期間換到另一台
    同尺寸螢幕時，網頁也改用新螢幕的識別與保存值。
  - **命中與穿透**：網頁以 `edge_hit {shapes, dragging}` 回報命中區，`shapes` 是畫出來的每個多邊形（`[x0,y0,x1,y1,…]`，
    舞台本地 CSS px，與 SVG path 同樣取兩位小數），與 SVG 填色同一組點；沒有畫出瀏海時（`notchQueue` 為空、幾何還沒算好）
    回報 `shapes: []`、`dragging: false`（`notchHitReport`）。原生輪詢用 `edge_hit::decide`（even-odd 點在多邊形內，不加容差）
    切換穿透：命中區外一律穿透；兩種情況暫時接管：網頁回報的拖曳中，以及原生按下鎖定（`edge_hit::press_latch`：輪詢先看到
    滑鼠鍵放開、再看到按下時游標在命中區內，就鎖定到滑鼠鍵全部放開，不等網頁回報開始拖曳）；滑鼠鍵放開即結束接管，網頁回報
    的拖曳另發 `kunsu://edge_drag_end`（`inv_edge_notch_click_through`）。舞台隱藏或重新顯示時鎖定清除，且視為滑鼠鍵按著，
    必須先看到放開才會鎖定。`edge_hit` 收到空命中區時立即解除網頁回報的拖曳與按下鎖定並恢復穿透（`edge_hit::grab_after_report`），
    網頁同時把進行中的拖曳當作放開處理（吸附、清掉拖曳與 hover 狀態，`notchHiddenCleanup`），所以瀏海之後再出現時不會帶著
    舊拖曳接管別的 app；這次放開以結案前畫出來的長度定位頭端（`settleRelease` 的 `drawn {len, head}`），保存的頭端就在放開處。
    原生輪詢看到舞台被隱藏（勿擾、⌘⇧K、沒有待決題）時，以及下一次顯示前（`prepare_stage`），若網頁回報的拖曳或按下鎖定
    還在，就一併解除並發 `kunsu://edge_drag_end`（`edge_hit::grab_after_hide`；只發一次，隱藏期間閒置輪詢不重發）；網頁收到
    `edge_drag_end` 時只要還有手勢或還在拖曳，就照「沒畫出瀏海」清理（`notchHiddenCleanup(drawn, …, nativeEnded = true)`：
    手勢當作放開結束——只是按著沒拖不算點擊、hover 收回、取消待開始的 hover、不再拖曳）並重繪一次，命中區回報立即改成不拖曳，
    之後瀏海再出現、幾何改變時都回報不拖曳；網頁已自己收到 pointerup 才收到的 `edge_drag_end` 不動任何狀態。輪詢間隔＝游標到命中區外接矩形的距離 ÷ 3 px/ms，夾在 16–250 ms；拖曳或在
    命中區內 16 ms；舞台隱藏 500 ms。游標進出命中區時發 `kunsu://edge_pointer {inside}`，在命中區內每 1 秒重送一次
    `inside: true`（`edge_hit::pointer_signal`）；網頁 3 秒沒收到任何「在瀏海上」訊號就收回 hover。
  - **隱私**：`edge_privacy` 回當下重新偵測的隱私 guard（`privacy_guard_active`：screen_shared、camera、mic、前景全螢幕
    任一）。舞台可見時原生輪詢約每 0.5 秒偵測一次（間隔 500–750 ms），改變就發 `kunsu://edge_privacy {active}`。網頁每次 hover 開始都視為隱私中，
    直到這次 hover 的 `edge_privacy` 回應或之後的推送說「未命中」才顯示題目文字。偵測限制：`screen_shared` 只用
    `CGDisplayIsCaptured`（獨佔擷取），macOS 沒有公開 API 能得知別的 app 正在錄影或用 ScreenCaptureKit 分享，這類情況
    偵測不到（BDD edge-notch 場景 12 已知限制）。
  - **靜止文字**（`ui/src/edge/content.ts` `notchRestState`）：最早一題 `state = forced`（蓋板或主視窗後備正在詢問它）→
    `asking`，顯示一般待處理文字、無障礙名稱「最早一題正在詢問」；否則已到 `force_at` → `deferred`（讀 `defer_reasons`）；
    到 `warn_at` → `warning`；其餘 `pending`。題目清單更新時立即重算；下一次重算排在 15 秒與最早一題下一個
    `warn_at`／`force_at`（加 20 ms）兩者較早者，warning 期間對齊秒數（`notchClockDelay`）。`RestState.warnMs`＝最早一題
    `force_at − warn_at`（`cycleTimes`）；`restText` 把 warning 的分鐘補零到 `warnMs` 的分鐘位數（`warnMinuteDigits`：
    `⌈warnMs/1000⌉/60` 取整的位數；預設 3 分鐘是 1 位「2:59」，10 分鐘以上是 2 位「10:00」→「09:59」），量寬
    （`restLength`）與繪製都用同一串字，所以提前警告設成 10 分鐘以上時倒數換字也不改變長度（PRD FR-03）。
  - **延後原因**：`defer_reasons` 回 `{reasons: [code…], dnd_until?, quiet_until?}`，依顯示優先序排列：`screen_shared`、
    `camera_on`、`mic_on`、`frontmost_fullscreen`、`dnd`（Kunsu 勿擾，含 kill switch；系統專注模式偵測目前恆 false，也歸這一碼）、
    `quiet_hours`（`GuardState::defer_reasons`，純函數，與狀態機判斷同一份 guard 快照；`quiet_until`＝安靜時段結束的時刻）。瀏海在最早一題到期（deferred）時讀，每 2 秒重讀；文字對照見 BDD edge-notch 場景 10c。
    第一次讀到之前（`defer` 為 undefined）瀏海顯示前一個靜止狀態（剛載入用一般待處理文字，`shownRestState`）；讀取失敗或
    2 秒內沒有回應記為 null，才顯示「時間到了，馬上跳出來」。`dnd` 這碼目前不會被看到（Kunsu 勿擾時瀏海隱藏）。
  - **資訊與位置**：`edge_info` 回 `{width, height, menubar, display, display_id, saved}`：螢幕大小；`menubar`＝選單列實際佔用
    （visibleFrame 推得）、實體瀏海（safeAreaInsets.top）與系統選單列標準高度 `max(NSStatusBar.system.thickness, 24)` 三者最大
    （`edge::top_inset`；自動隱藏選單列時也至少 24 pt）；`display` 螢幕名稱、`display_id` 螢幕識別（取不到為 null）；`saved`
    只回格式正確且記住的螢幕就是目前舞台螢幕的值（`edge::saved_for_screen`）。`edge_save {display, display_id, position}` 寫
    `{display_id, display, position}`；位置只接受 `{corner: tl|tr|br|bl}`、`{edge: top|right|bottom|left, head: 0..1}`
    （直邊的圖示端沿邊比例，現行格式）或舊格式 `{edge, ratio: 0..1}`（中心比例，只讀相容），其餘回錯誤不寫檔
    （`edge::valid_position`）。網頁存檔一律寫 `{corner}` 或 `{edge, head}`（`savePosition`，需要當下長度定位頭端）；
    還原時頭端格式依當下文字長度從頭端往尾端展開（`restorePosition`），所以文字長度不同頭端也不動。讀到不合格的保存值一律用
    預設位置（`parseSavedPosition`），存檔時同步更新記憶體中的位置。舞台識別（`stageIdentity`）＝大小＋`display_id`（沒有才用名稱）。
    網頁記憶體中的保存位置是 `SavedMemory {display, saved, resetGen}`（`ui/src/edge/geometry.ts`）：重讀 `edge_info`
    （`kunsu://edge_stage_changed`、視窗 resize）只在螢幕（`displayKey`：有 `display_id` 用它，沒有才用名稱）改變時換成那台
    螢幕的 `saved`（`savedAfterRead`）；同一台螢幕讀到 `saved: null` 不改記憶體、不移動瀏海——放開後延後 400 ms 的寫檔可能
    還沒發生或寫檔失敗，這不是重設。放開時記憶體立即更新（`savedAfterRelease`），`edge_save` 延後 400 ms 合併，寫檔前比對
    排程當下的重設世代（`saveStillCurrent`），不同就不寫。**回到預設位置**只看 `kunsu://edge_position_reset`（§15
    `edge_reset_position` 刪檔成功後發，只送瀏海視窗）：網頁先取消待寫的存檔計時器，再 `savedAfterReset`（清掉記憶體位置、
    `resetGen` 加一），然後以預設位置重新定位；`kunsu://edge_stage_changed` 只讓網頁重新貼合與重讀，不再因 `saved: null`
    改位置（2026-10-09 QA 第 5 輪）。殘餘時窗：shell 刪檔到網頁收到事件之間（同一台機器上的事件遞送，毫秒級）剛好觸發的
    延後寫檔仍會寫出，網頁無從得知。
  - **長度與動畫**：靜止長度＝文字寬＋`T/2`＋28 px；停在角落時再加 `2r − πr/2`（r＝10，文字中心線轉角圓弧比直角少走的長度，
    `restLength(…, corner)`）。中心與長度走同一個彈簧（`stepMotion`：靜止與放開 ω 20，`OMEGA_LENGTH`＝`OMEGA_SETTLE`；拖曳中
    兩者都 ω 30），所以 hover 伸縮與文字長度改變的動畫中頭端不動；每一幀的時間差夾在 0–33 ms（rAF 時間戳可能早於觸發動畫的
    事件，負值當 0；`springStep` 遇到 dt ≤ 0 不移動），不會先往反方向跳；減少動態時 `stepMotion` 不跑彈簧，任何目標直接到位、速度
    歸零，回報不再需要下一幀（`notchFrameNeeded`：減少動態不為文字淡入與跑馬燈跑 frame）。hover 一律往尾端延長 88 px
    （`growDirection(head)`），空間不足時尾端沿周長轉過角落成 L；收回與「從 hover 開始拖曳」都以頭端為錨（`headAnchoredCenter`、
    `grabAfterCollapse`：抓取偏移以收回後的中心計算）。按下時抓取的邊是游標底下那一臂的邊（`grabStart(st, s, p, len)`：
    瀏海各段所在的邊裡離游標最近的那一條，不偏向中心所在的邊、不加遲滯），抓取偏移在這條邊上量，`grabAfterCollapse` 也用
    這條邊，所以 L 形靠角落那一段與 hover 轉角出來的短臂開始拖曳都不跳。例外（2026-10-09 QA 第 5 輪）：按在 L 形兩臂於
    螢幕角重疊的 T×T 方格裡（`cornerSquareArms(st, s, p, len)`：瀏海畫出來的各段含這個角落相鄰的兩條邊，且游標離兩條螢幕邊
    都小於 T；`grabStart` 回傳 `arms`）時，按下時不決定：移動滿 `DRAG_THRESHOLD`（4 px）那一刻以 `grabByDirection(st, s,
    start, p, arms)` 依按下點到這一點的位移選臂（|dx| > |dy| 選橫的那條邊、|dy| > |dx| 選直的、相等選離按下點較近的邊），
    這次選擇不加遲滯，抓取偏移在那時、在選定的邊上以**按下的位置**量（`s` 用那一刻畫出來的中心），之後換邊照常 40 px 遲滯；
    從 hover 開始拖曳時 `grabAfterCollapse` 用選定的邊。所以在方格裡按下、沿任一臂拖，頭端位移也等於游標沿那一臂的位移。
    `grabStart` 沒給長度時不判斷方格（照四條邊找最近的）。文字長度改變時頭端不動，但靜止位置一律完整留在
    原本那條直邊上：`resizeCenter` 經 `keepOnEdge`，文字變長超出時整條往回移，尾端離螢幕邊 24 px（`EDGE_GAP`，與放開吸附一致）；
    還原保存值（`restorePosition`）只保證完整在那條邊上，放不下就整條往回移到尾端貼齊邊界，不加 24 px。放開時以 `settleRelease`
    同時決定吸附位置與最終長度（角落用角落長度，否則直邊長度；24 px 間隙以最終長度計算），並把記錄的靜止長度同步成最終
    長度，之後長度 effect 不再移動中心。英數字詞＝連續的半形可見字元（含夾在其中的 `$ , . % - : /` 等）與帶重音的拉丁字母
    （`tokenize`）；字詞裡的數字一律等寬：量寬時每個數字都換成最寬的數字再量（canvas `measureText` 不支援
    `font-variant-numeric`），繪製用 `font-variant-numeric: tabular-nums`，所以 warning 倒數每秒換字時靜止長度不變、不跑
    伸縮動畫（PRD FR-03）；跑馬燈純文字只去掉真正的 HTML 標籤（`<` 後接英文字母、`</`、`<!`），自動連結顯示網址（`plainLine`）。
  - 點擊沿用 `open_overlay_from_capsule`，轉場起點是圖示中心。尚未實作：拖曳跨螢幕轉移（只停留在目前螢幕）、點擊時
    不啟用 app（舞台不會成為 key window，但點擊仍會讓 app 成為前景 app）。
- ~~**膠囊存在規則（2026-08-02）**~~（由上方邊緣瀏海取代；以下保留歷史）：只有至少一張非 internal、非終態題時顯示；零待決時必須 hide，不得留下透明攔截區。tray 永遠存在並可開主 app。膠囊可由使用者自由拖曳；放開後投影到最近螢幕邊界並保留沿邊座標，距端點 64 logical px 內才磁吸至角落。以 320ms `cubic-bezier(.16,1,.3,1)` 完成最短平移，reduced motion 則立即定位。位置在本次 app session 內由視窗殼保留。
- **蓋板呈現（2026-10-08，`docs/bdd/overlay-presentation.md`）**：目標螢幕依膠囊所在螢幕 → 游標螢幕 → 主螢幕；
  在主執行緒以 `NSScreen.frame`（Cocoa 點）一次 `setFrame:display:` 後才帶到前景，顯示後讀回校正，螢幕重排時重套。
  呈現 handshake 狀態機（`overlay_presenter`）：網頁掛載後呼叫 `overlay_ready` 主動拉取待播轉場（事件只送蓋板視窗、
  以 id 冪等），題目、輸入與關閉都渲染後呼叫 `overlay_mounted(id)` 才顯示；1.5 秒未確認 → 隱藏、改開主視窗聚焦該題、
  重載蓋板網頁，連續三次發 Discord。原生 Esc 看門狗：網頁 2 秒內未 `overlay_escape_ack` 就由原生層完成 dismissal。
  原生 dismissal（Esc 後備與畫布卡住期間的 Esc 共用 `native_dismiss`）的範圍與網頁「稍後再問」相同
  （`overlay_presenter::dismiss_targets`）：所有非 internal、`state=forced` 的題，加上網頁最後以 `overlay_current
  {question_id}`（只收蓋板視窗）回報的目前題目（非終態、非 internal 才算），去重後逐題 `dismiss_question`；蓋板隱藏時清掉
  回報的目前題目。
  **同一個 session 每題只記一次 dismissal**（2026-10-09 QA 第 3 輪）：蓋板視窗呼叫的 `dismiss_question` 與原生後備都經過
  `windows::dismiss_once(handle, id, session)`——一把 async 鎖讓兩條路排隊，帳本是 `overlay_presenter::DismissLedger`
  （session＝`Presenter::last_transition_id()`，新轉場開始時清空；記錄「記完後的 `cycle_number`」）。鎖內先讀最新題目，
  `already_dismissed(id, cycle_number, forced)` 成立（這個 session 已記過，而且之後沒有再到期——又到期＝又是強制、週期編號
  ≥ 記錄的）就不再記、直接回傳目前題目；否則 `dismiss_question` 並記帳。原生後備在 `NativeDismiss`
  當下取 session，網頁晚到的 dismissal 照樣被帳本擋下。主視窗的 `dismiss_question` 不經過帳本。
  網頁只在真的處理了 Esc 才呼叫 `overlay_escape_ack`（決策面板已掛載且不在收起中：稍後再問／關閉在 dismissal **成功後**
  才 ack、失敗不 ack 交給原生後備；焦點真的在行內區塊的輸入框裡（textarea／input／contenteditable）收起編輯、輸入法
  取消組字當下 ack；判斷表 `ui/src/surface/decision.ts` `escapeAction`，`field` 欄位＝焦點在輸入框）；面板未掛載或收起中
  不 ack。「稍後再問」dismissal 失敗：面板不收起，`role="alert"` 顯示可讀錯誤（頁首下方的流內區塊，面板 grid 第 2 列，
  不蓋內容），可重試。重試（`runLater({ids, dismiss, done})`）：`done` 是面板持有、跨重試保存的「這次已成功的題」
  （id → 記完後的 `cycle_number`），已在 `done` 的跳過；dismissal 錯誤是「已在終態」（server `Terminal`）視同成功；
  範圍 `laterTargets(current, session)`（目前題目未結案＋session 的強制題，去重）；每次稍後再問前
  `forgetDueAgain(done, session)` 把又到期的題（session 裡又是強制、週期編號 ≥ 記錄的）從 `done` 拿掉，照樣記（與原生帳本
  同一條規則）。蓋板網頁的呈現決策
  （`ui/src/overlayShell.ts`）：`holdingReceipt`＝面板**當下**正顯示這次 session 在面板裡作答的回執（決策面板以
  `onHoldingReceipt(boolean)` 回報，切去別題就變 false）時不自動關閉，直到使用者按「完成」或 Esc；`answering`＝面板有
  作答正在送出（決策面板以 `onAnswering(boolean)` 回報：決策欄與題組在呼叫 `answer_question` 前同步回報 true，結果回來、
  寫進 session 後才回報 false）時一律 `render`。`overlayShellAction` 回 `close` 時蓋板等 `OVERLAY_CLOSE_GRACE_MS`（300 ms）
  才送 `kunsu://overlay/close`，緩衝期間 `overlayShellMounted('close')`＝true、面板照樣掛著；面板在緩衝期間回報回執或作答
  中，判斷變回 `render`、計時取消（server 先發 `questions_changed` 再回傳作答結果，舊版在這一刻就卸載面板）。
  **session 內容**（`ui/src/components/overlaySession.ts` `mergeQuestionSession(current, latest)`）：蓋板網頁把完整清單
  （含終態）交給面板；session 既有題目以最新資料更新（被其他通道結案的題就地唯讀），清單裡 `state=forced`、非 internal、
  不在 session 的題接到尾端（PRD §5.8）。**收起後對帳**：`kunsu://overlay/close` 隱藏蓋板、結束 session 後立刻跑一次
  `reconcile_overlay`（`overlay_plan(any_forced, guards_green, session_open)`：還有非 internal 強制題且 guards 全綠 →
  再呈現；session 開著且沒有強制題 → 不動；否則隱藏）。
  **沒有點擊座標的起點**：瀏海網頁在圖示中心改變時以 `edge_anchor {x, y, size}`（舞台本地 CSS px，只收 capsule 視窗、
  有限值、size 8–96）回報；`show_overlay`（強制、tray、批次窗）用最後回報的位置（`overlay_geometry::overlay_origin`：
  點擊 → 最後回報 → 固定估計），再經 `anchor_in_overlay` 換算成蓋板座標。網頁端回報由 `ui/src/notchAnchor.ts` 在瀏海
  視窗量 `.edge-stage [data-agent-mark]`（來源圖示）的中心：命中區更新後 150 ms 量一次（同一段時間只量一次、拖曳中不量），
  變動 > 0.5 px 或大小（取整數）改變才送。
  勿擾讀取失敗沿用最後已知值；啟動後從未讀成功時 60 秒後視為未啟用並告警一次。
- ~~**膠囊視窗固定貼合內容**（2026-08-04）~~（邊緣瀏海後不再使用 `resize_capsule`）：UI（CapsuleBar）用 ResizeObserver 量測膠囊實際
  渲染大小，經 `resize_capsule {w, h}`（CSS px → LogicalSize，Rust 端 clamp 60–480 × 32–400）
  通知殼 setFrame。膠囊約 140×44，hover 不得放大、預覽、顯示 tooltip 或改變任何視覺狀態。
  視窗貼合後全程可點；任何情況下不留大面積透明攔截區。
- UI↔server 全走 Tauri IPC commands：`overlay_ready/overlay_mounted/overlay_escape_ack/overlay_current/list_questions/get_question/get_evidence/canvas_failure/canvas_check_take/canvas_check_done/edge_info/edge_hit/edge_anchor/edge_save/edge_privacy/defer_reasons/answer_question/reauthorize_question/resolve_money_outcome/dismiss_question/edit_option/open_deep_link/open_overlay_from_capsule/resize_capsule/capsule_drag_geometry/move_capsule/snap_capsule/dnd_get/dnd_set/dnd_clear/kill_switch/board_pack`；`kill_switch` 相容呼叫等同 `dnd_set(240)`，`snooze_question` 僅保留舊 client alias；**answer_question 是唯一作答入口**（產簽章）。
- 卡片六區解剖、收據卡、執行流串流依設計 §4；選項編輯後 params_line 重算 + 紅線重檢（>NT$2,000 或偏離>20% 自動升 red 並鎖一鍵）。
- 分層捲動：T0/T1 卡一屏禁捲動（十秒可決）；t2 董事會長文（body_md 等）line-clamp 6 行＋「展開」鍵，展開後僅卡片內容區內部捲動（header／footer 不動）。
- `question_line/body_md/evidence/options` 支援 Markdown＋safe HTML；UI 必須 sanitize。輸入框接受 Markdown 原文，但不另做即時預覽。禁止 script/style/form/iframe/object/event handler/任意 class 與 inline positioning。Agent 只提供內容結構與受限 `visual_symbol`，版面、控制元件、粒子行為與 design tokens 由 Kunsu 固定。
- 通知：tauri-plugin-notification；tray 抄 task-engine tray.rs 模式。
- 全域快捷鍵勿擾（4h preset，落 dnd_changed/fatigue event）。
- `DndChanged` server event 觸發 shell 原子對帳：active 時先 hide overlay/capsule；事件橋接在發送任何
  notification 或 show surface 前再次查 active status，避免 race 洩漏。解除時重新 reconcile capsule
  與 overlay；主動開 main window 始終允許。
- menu bar 每 15 秒及每次 server event 依 `dnd_status(now)` 重建：狀態列分別顯示模式與
  `remaining_seconds + ends_at`，動作提供 1/2/4/8/12/24h、提前結束與 `open_settings`。active 時
  不列 question title；preset 全部走同一 `set_dnd(minutes, "user")`，更多時間只開主視窗設定頁。

**P2 收貨後固化的 IPC/事件契約（UI 已按此實作，Tauri/HTTP builder 必須對齊）**：
- Tauri event：執行流串流 `kunsu://execution`（payload `{question_id, step: prepare|guard|api|
  ledger, status: start|ok|fail, line?, receipt?}`）；清單變更 `kunsu://questions_changed`。
- `answer_question` payload 是 tagged union `{question_id, type: choice|free_text|reject|batch, ...}`（`batch` 見 §14）；
  `reject`＝「這題不該問我/退回重擬」→ server 映射 cancelled + 出題品質計分，並在題目寫 `rejected {at, reason?}`（終態帳本行
  帶著它，`rebuild-db` 後督導頁仍分得出「退回」與 agent 取消；`ask_get` 讀得到，agent 知道使用者推掉這題與原因）。
- `edit_option` 參數 `{id, option_id, new_to}` → 回 `{question, redline}`。
- 「轉討論」條件＝tier t2 或 gate red，走 `open_deep_link("kunsu://discuss/<id>")`，Rust 端
  spawn 帶脈絡互動 session。
- 非 T0 題「稍後」純 UI 跳卡不打 snooze IPC（snooze 語義僅 forced/T0）。
- invoke 參數鍵 snake_case；Rust command serde 對齊。
- Question 對 UI 加 `execution_blocked?: string`（執行紀錄以 `EXEC_BLOCKED:`／`EXEC_FAILED:` 開頭時的原因）與
  `money_state?: running|unknown|reauthorizable|settled`（已作答的錢題才有，伺服器算，§7）。所有回傳題目的 IPC（`list_questions`、
  `get_question`、`answer_question`、`reauthorize_question`、`resolve_money_outcome`、`dismiss_question`、`edit_option`、`board_pack`）都附。
- `reauthorize_question {id}` 與 `resolve_money_outcome {question_id, executed}` 只收主視窗與蓋板（使用者的點擊），瀏海舞台與其他
  視窗一律拒絕（`money_window_allowed`）。
- 紅線常數與 executor.py 同值且**嚴格大於才升紅**（恰 2000/恰 20% 不升）。
- 紅燈題「執行鎖 ≠ 作答鎖」：紅燈/升紅/缺 prediction 時主鍵＝「拍板此選項」（只落 ledger
  不執行）；locked 僅防雙擊。

**決策面板（2026-10-08，`docs/bdd/decision-surface.md`；取代 2026-08-03 的粒子 workspace 與墨水轉場）**：
- 蓋板內容是單一黑色面板（`ui/src/surface/`）。有轉場時，黑色形狀從起點（目前是膠囊位置推得的瀏海，邊緣瀏海交付後
  改為瀏海本身）先沿邊延伸、再往內展開（`critical(t, ω)`，pA ω=20、pB ω=11 延遲 90 ms，t>0.2 s 內容以 45 ms 間隔依序
  淡入），來源圖示從起點飛到頁首；收起從當下進度倒放。形狀定案（stage=open，約 0.93 s）前面板 `inert`，鍵盤焦點與點擊
  都進不去；Esc 由面板在掛載當下以 window capture 監聽處理（`escapeAction`），不等動畫。動畫迴圈沒跑時計時後備：張開
  `OPEN_FALLBACK_MS`=1200 定案、收起 `CLOSE_FALLBACK_MS`=900 完成。沒有轉場或減少動態（`ui/src/motion.ts`
  `reducedMotion()`：系統 `prefers-reduced-motion` 或 App 設定 `kunsu:motion:v1=reduced`／`html[data-motion=reduced]`）→
  直接開啟並 160 ms 淡入、收起 160 ms 淡出（CSS 以更高權重保留，不被全域減少動態規則壓成 0）；圓角依瀏海所在邊
  （`panelRadius(anchor)`）。
- 頁首：來源圖示與名稱、任務與觸發方式、提出時間；佇列「第 N 題，共 M 題」＋進度點與 ‹ ›；右上只有「稍後再問」，按鈕上
  以看得到的文字寫「N 分鐘後再問」＋`Esc`：N 是面板掛載時 `ipc.settingsGet()` 的 `effective.force.cycle_min`（讀不到、還沒回來
  或不合理時是 15；`laterHint(cycleMin)`，2026-10-09）。回看（`review`）時右上改成「關閉 `Esc`」，不出現稍後再問。不顯示倒數
  圓環、倒數文字或倒數公告（AC-DS-11）。面板舞台 `.surface-stage` 自己宣告 `color-scheme: dark`；主視窗裡（`html[data-surface=app]`）
  舞台是黑底，蓋板裡維持全透明。面板字型把 `--font-sans` 換成只涵蓋拉丁字元的 "Kunsu Geist"，中文與全形標點交給蘋方。
- 版型：有證據（Markdown、結構化證據行或畫布）→ 證據｜決定兩欄（左欄獨立捲動）；沒有 → 置中單欄（≤760 px）。面板本身
  寬度 < 1100 px（`.surface-panel { container: surface / inline-size }`＋`@container surface (width < 1100px)`；題組的
  `batch.css` 用同一個條件，2026-10-09 取代 `@media (max-width: 1099.98px)` 與題組的 `max-width: 1100px`）：
  單欄、`.surface-body` 整體捲動、證據撐開高度、底部操作列 absolute 固定在面板底部且內容區預留其高度；任何
  版型下操作列自帶底部留白、捲動內容不從其下方露出；決定欄長字串 `overflow-wrap: anywhere`、不橫向捲動（AC-DS-14）。
  左欄：為什麼問你（時間軸辨識 `HH:MM`／`現在` 清單；打開原對話；看原文與字數）→ 證據畫布（§12）→ Markdown → 證據行。
- 決定欄：選項是 radiogroup（一個 Tab 停點、上下鍵移動、點選或 1–4 只選取可取消、`aria-checked`）→ 補充說明 →
  決定摘要；底部左「這題不該問我」、右主要動作（未選且未輸入時停用）。主要動作文字（`primaryLabel`）：選擇題「送出決定」；
  文字題或沒有可選選項（含十秒可決未過）「送出回答」；確認題（confirm、無選項）「確認」；黃燈錢題選了執行型（AI 建議）
  選項「批准並執行」；錢題選了選項但一鍵被鎖（紅燈、升紅、缺 prediction）「拍板此選項（不執行）」；送出中「送出中…」。
  鍵盤（`surfaceKey`＋`focusKind`）：Enter 送出、Shift+Enter 換行、輸入法組字中 Enter 不送出；焦點在選項上 Enter 攔下瀏覽器
  預設、送出目前明確選取（不改選取）；焦點在其他按鈕或連結上 Enter 交給瀏覽器啟動該控制項；開題與切題時焦點落在選項
  （文字題落在回答框）；Tab／Shift+Tab 在面板內循環（`wrapFocus`）；畫布 iframe 不留住焦點。
- 送出：選項＋補充說明一起送（server 存 `choice_id`＋`free_text`）；帶作答當下的 `revision`。回執就地顯示決定、
  補充、「已收到（時間）」「已寫入決策帳本」與交付狀態（`ui/src/delivery.ts`，§8），可打開原對話、顯示 AI 回饋；
  交付未定案時輪詢（前 2 分鐘每 2 秒、之後每 10 秒、30 分鐘停）。「下一題 ⏎」或「完成 ⏎」；回執出現時焦點移到這顆按鈕，
  焦點在回執其他按鈕（打開原對話）上 Enter 只啟動該按鈕。回執與執行失敗畫面的題目與選項經 Markdown 渲染；確認題顯示
  「確認」。蓋板裡答完最後一題時回執保留到「完成」或 Esc（見上方蓋板呈現）。
- 錢題回執的後續動作（`ui/src/surface/MoneyActions.tsx` `moneyActionsFor`；BDD money-approval AC-MA-18／19／20）一律依
  `money_state`：`unknown` → 「我核對過：錢已經動了／錢沒有動」；`reauthorizable` → 「重新授權並執行」；`running`／`settled` → 沒有按鈕。
  沒有 `money_state` 的舊資料才退回看執行紀錄開頭（`EXEC_UNKNOWN` 對帳；黃燈 UI 執行型答案的 `EXEC_BLOCKED`／`RECONCILED_NOT_EXECUTED`
  重新授權）。按鈕依最新的題目（prop 換了就以新的為準，本地只保留「剛送出」的暫態）；每個按鈕都先「再確認一次」，打開時焦點移到
  「確定」、取消回到原按鈕、送出成功後焦點移到回執的「完成／下一題」。失敗畫面的文字依狀態：`reauthorizable` 才寫「修好原因後可以
  在這裡重新授權」；`unknown` 時標題是「結果不明」並寫「executor 已用掉授權但沒有回報結果，錢可能已經動過，請先到廣告後台核對
  再對帳」。作答當下收到執行失敗事件後重新讀題：`money_state` 是 `unknown` 就走回執＋對帳鈕，不停在「執行沒有完成」；還是
  `running`（錢鏈列要等交付結束才收尾）就留在回執，回執在 `money_state = running` 時同樣輪詢，狀態定案後按鈕跟著出現；其他狀態
  才進失敗畫面。
  錢那一行（`moneyOutcome`）的圖示依狀態：已執行 ✓、乾跑或沒有執行 –、結果不明 !、還在跑 …，都附文字。
- 切題只換內容不重播張開動畫、不重設倒數；舊內容 140 ms 淡出，新的 `.surface-body`（`key={q.id}` 重新掛載）帶
  `data-enter="true"` 以 140 ms `surface-fade` 動畫淡入；減少動態直接換、不帶 `data-enter`。已答題回看顯示唯讀回執；
  在面板外被結案（session 裡的題變成終態、不是面板送出的）→ 唯讀「已在其他地方結案」畫面（`closedElsewhereLine`）。
  「稍後再問」對本次所有 forced 題（含 session 中途加入的）記 dismissal。
  進度點的無障礙名稱（`dotLabel`）依實際終態：未結案「第 N 題」、在面板結案「第 N 題（已回答／已退回重擬）」、被其他通道結案
  「第 N 題（已結案：<`closedElsewhereLine`>）」。
- 回看（2026-10-09 QA 第 3 輪）：一打開就是終態、不是在這次 session 結案的題，`reviewPhase(q)`：`answered`／`routed`，或
  `cancelled` 且有 `rejected`（使用者推掉）→ 回執（執行失敗的錢題 → 失敗畫面）；`expired`、`superseded`、沒有 `rejected` 的
  `cancelled`（出題的 AI 取消）→ 唯讀狀態畫面（`closedStatus(q)`：已過期＋「沒有回答」或「沒有回答（逾時依預設：選項）」、
  AI 取消了這題、被新題取代），沒有任何作答控制。回執與題組回執在 `cancelled` 時只有 `rejected` 才寫「已退回重擬」，並附
  「你退回了這題：原因」。`closedElsewhereLine` 對有 `rejected` 的 `cancelled` 寫「你在另一個 Kunsu 視窗退回了這題」。
- 主視窗的作答流程（`App.tsx`）也傳 `latest`：一般 flow 傳完整清單；歷史紀錄回看（`openFlow([question], 0, {review: true})`）
  傳 `flowFeed(questions, session, true)`（只含 session 那幾題的最新資料，不接強制題）與 `review`。作答流程記住開啟時的頁面
  （`View.from`）：面板開著時主視窗底下仍是那一頁（不先閃待決事項），收起後回到那一頁、篩選與捲動位置不變，焦點回到開啟它的列
  （2026-10-09 主視窗 QA 第 1 輪）。歷史紀錄的篩選由外框保存（`historyFilter`）。
- 選項卡片（AC-DS-13）：`.surface-opt` 是 `div`（`data-checked`），裡面一顆鋪滿卡片的 `button.surface-opt__hit`
  （`role="radio"`、`aria-checked`、`aria-labelledby` 指向標題、`aria-describedby` 指向說明），標題與說明是按鈕外的兄弟元素、
  疊在上面且 `pointer-events: none`，只有其中的連結 `pointer-events: auto`；連結不在按鈕裡，點擊不會冒泡到按鈕。選項標題、
  說明與回執的你選的選項都走 `RichContent onOpenLink={openLink}`；`RichContent` 的連結點擊 `stopPropagation`。
  `RichContent` 交給 react-markdown 的連結（`RichLink`，讀 `LinkStateContext` 的「等第二次點擊」狀態）與行內段落
  （`InlineParagraph`）是模組層級的固定元件：第一次點擊／Enter 出現提示時連結只更新、不重新掛載，鍵盤焦點留在連結上，
  再按一次 Enter 才打得開（2026-10-09 QA 第 3 輪瀏覽器實測：舊版每次渲染都換新元件，提示一出現焦點就掉到 body）。
- 往外開（`openOutside`）：蓋板裡先 `await stepAside()`（稍後再問：dismissal 成功 → 收起動畫 → `onExit` 送出
  `kunsu://overlay/close` 並等它完成）再呼叫 `open_deep_link`；讓開失敗不打開。面板裡每個 `RichContent`（why、時間軸、
  證據 Markdown、證據行、題組原文、一般題與題組回執的 AI 回饋、選項標題與說明、回執的你選的選項、題目句）都接 `onOpenLink`。
- 回執焦點：一般題與題組送出後焦點移到回執的「下一題／完成」；回執裡 Enter 依 `focusKind`：焦點在其他按鈕或連結上交給
  瀏覽器，不跳題。

## 10. policy.toml 鍵（預設值）

```toml
[force]  cycle_min = 15  warn_before_min = 3  deferred_retry_s = 1
[windows] t1_batch = ["09:45", "21:30"]  quiet_hours = ["23:00", "09:00"]  quiet_enabled = true # t1_batch 僅相容舊設定
[expiry]  t1_default_after_h = 48  t2_supersede_after_d = 14
[quota]   per_source_daily_create = 12
[fatigue] skip_rate_window_d = 7  skip_rate_downgrade = 0.5
[discord] webhook = ""   # 空=沿用 meta-ads config.json 的 mkt_daily webhook（server 讀該檔）
```

**Target（§15）：** 生效值＝`~/.kunsu/policy.toml`（安裝時由 repo `config/policy.toml` 複製）逐鍵疊上 `~/.kunsu/policy.local.toml`
（設定頁寫的個人值，安裝永不碰）。兩檔都熱載；local 檔解析失敗或超出範圍時只用 base，設定頁顯示錯誤。

## 11. 品質閘（每層完工條件）

- kunsu-core：`cargo test` 全綠；狀態機表驅動測試覆蓋全轉移矩陣 + INVARIANTS 各一條測試。
- kunsu-server：integration test（tokio + 臨時 DB）：12 tools 正反例、upsert 去重、long-poll、
  游標收割、/events/kongko 結案+啟用題生成、approvals/verify 單次性、ledger append+rebuild。
- ui：`pnpm build` 過 + vitest 卡片 lint 邏輯。
- src-tauri：`cargo build` 過；doctor 冒煙（建測試題→模擬答→驗 ledger+reactor dry-run→清除）。
- 每層一個 git commit；不跨層混 commit；executor.py 改動獨立 commit（在 executor 所在的 vault repo）。
```

## 12. 證據畫布與草稿註冊（PRD §24；BDD evidence-canvas）

- **草稿**：`~/.kunsu/drafts/<id>/`（必須在這個根目錄下；任何檔案經 symlink 指到外面一律拒絕）。
  `question.md` = `---` YAML frontmatter（欄位同 `ask_create`；`id` 預設資料夾名、寫了必須一致；`why` → `why_md`；
  未寫時 `title` = `question_line`、`tier` = t1、`default_action` = noop_log）＋ Markdown 內文（→ `body_md`）；
  可選 `canvas.tsx|jsx|html`（最多一個，≤200 KB）；可選 `assets/`（只收被引用的圖片：png/jpg/gif/webp/svg，單張
  ≤2 MB、合計 ≤8 MB）。
- **檢查順序**（全部 server 端，有問題就不渲染、不建題）：路徑 → 檔案 → YAML（錯誤行號）→ 欄位反序列化 →
  `validate_create`（錯誤對到 frontmatter key 的行號）→ Markdown 安全（禁用標籤、`on*`／style／class 屬性、
  `javascript:`，逐行；圖片以 pulldown-cmark 解析，含參照式 `![x][r]` 與跨行 `<img>`，外部或 data: 一律擋；內文、why、
證據文字、選項 label／detail 都檢查，錯誤對到 frontmatter key 的行號）→ 畫布
  外部網址（`http://www.w3.org/` 命名空間除外）與 `http-equiv` → 資產（`assets/` 本身是 symlink 也擋）。
  issue codes：`draft_outside_root` `question_missing` `encoding` `canvas_multiple` `canvas_too_large`
  `frontmatter_missing` `frontmatter_yaml` `frontmatter_not_mapping` `frontmatter_field` `id_mismatch`、
  `validate_create` 的各 code、`canvas_navigation`（畫布含 `http-equiv`；renderer 也以它回報畫布把自己換頁）、`markdown_forbidden_tag` `markdown_forbidden_attr` `markdown_forbidden_url`
  `markdown_data_url` `external_url` `asset_missing` `asset_outside_draft` `asset_type` `asset_too_large`
  `asset_unreadable` `assets_total_too_large`；renderer 回報 `import_not_allowed`（含多行 import）`dynamic_import` `network_api`（含 WebRTC 與資源提示
  `rel=preconnect|dns-prefetch|preload|prefetch|modulepreload`）
  `storage_api` `parent_access` `canvas_compile` `canvas_runtime`（只有行號）`canvas_no_default_export`
  `canvas_load` `canvas_navigation` `canvas_incomplete`（HTML 畫布的文件沒有載入完成）`canvas_overflow_x` `canvas_too_tall`
  `canvas_timeout` `renderer_unavailable` `question_closed`。
- **沙盒渲染檢查**：server 透過 `CanvasRenderer` trait（app 殼層注入）渲染候選題。殼層用隱藏的 `canvas-check` 視窗
  （1440×900，`index.html?view=canvas-check`）載入同一個決策面板，畫布以檢查模式執行（與正式顯示同一個 2 秒時限、
  回報橫向溢出與 >2400 px）；檢查頁每次都用這次 job 帶來的內容（證據快取鍵含 `content_hash`）；檢查期間視窗放在螢幕外、最後面、不收滑鼠、Transient＋IgnoresCycle，並關閉 WKWebView 遮蔽偵測，
  定案後以 `WKWebView takeSnapshot` 存成草稿資料夾裡的 `.kunsu-preview.png`（先寫同資料夾的新暫存檔再 rename，不跟著
  symlink 寫到外面），再 orderOut。一次只檢查一筆；檢查回報上限 12 秒、截圖上限 5 秒；有畫布時逾時回 `canvas_timeout`
  （提示檢查無窮迴圈），沒有畫布才回 `renderer_unavailable`。**每一筆檢查都用新的檢查視窗與網頁行程**：定案（截圖完成、
  失敗或逾時）後先以 `_webProcessIdentifier` 終止檢查視窗的網頁行程（SIGKILL），再銷毀視窗並等它從視窗清單消失；下一筆
  重新建立（冷啟動算在 12 秒內）。卡住或「畫完才卡住」的畫布不會留下持續耗 CPU 的行程，也不會讓下一筆逾時。
  畫布拿不到檢查／顯示模式（兩者給畫布的資料相同）。沒有 renderer 時：有畫布 → `renderer_unavailable`；純 Markdown 照常註冊、不附截圖。
- **版本**：通過且非 dry-run → 快照寫入 `~/.kunsu/revisions/<id>/<revision>/`（question.md、畫布、引用的資產、
  meta.json）。revision 在同一題的寫入鎖內決定；快照先寫 `revisions/<id>/.tmp-*`，鎖內確認題目未結案且目標 revision
  不存在才 rename 進去，既有 revision 資料夾永不覆寫。新題 revision 1；同 id 未結案且 `content_hash` 不同 → revision+1，只換內容（題目文字、why、內文、
  kind、選項、證據、錢參數與參照、預設動作〔已以新選項驗證〕），不動狀態、週期時間戳、cycle_number、first_proposed、
  tier、到期、來源與喚醒方式，不發通知（`QuestionTouched`）；雜湊相同 → `unchanged`；已結案 → `question_closed`＋
  去敏的既有答案。草稿題的內容只能經由重新註冊改變：`ask_update` 拒收，`ask_create` 同 id 重送只 touch。
  MCP `dry_run` 必須是布林；CLI 不認得的旗標回結束碼 64。
- **作答綁版本**：`answer_question` payload 帶 `revision` 與 `content_hash`；草稿題從面板作答任一沒帶 → 拒收；任一與目前
  版本不同 → 拒收 `StaleRevision`（請使用者看過新內容）；
  所有作答通道都把當下 `revision`／`content_hash` 寫進答案，ledger 快照可追溯。
- **顯示**：`get_evidence {question_id, revision}` → `{canvas?: {kind, source}, assets: {"./assets/x": dataURI}}`。
  畫布在 `sandbox="allow-scripts"` iframe（無同源）＋CSP `default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'
  blob: data:; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data:; connect-src 'none';
  form-action 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none';
  manifest-src 'none'`；runtime（React、Recharts、lucide-react、sucrase、@tailwindcss/browser、`kunsu` 模組，
  `ui/public/canvas-runtime.js`）整份內聯進 srcdoc。srcdoc 的 head 第一個元素是 `<base href="about:srcdoc">`（在 CSP 之前，
  CSP 的 `base-uri 'none'` 照樣擋畫布自己加的 `<base>`）：srcdoc 文件預設以上層頁面網址為基準，畫布裡的 `#id`、`#`、
  `index.html` 會解析成 Kunsu 自己的頁面；改以 `about:srcdoc` 為基準後，`#id` 連結在畫布內捲動（同文件），相對網址
  （`index.html`、`?x`、`//host/x`）解析失敗、不會導覽（`location.href = 'index.html'` 丟 SyntaxError，照一般執行期錯誤回報）
  （2026-10-09 原生實驗確認；SVG 的 `url(#id)` 照常）。只接受同一 iframe、同一次渲染 token 的 `resize`／`ready`／`error`
  （`inv_canvas_sandboxed`）；2 秒內沒 ready 或出錯（畫完之後的錯誤也算）→ 隱藏並顯示「圖表載入失敗：原因」，
  並以 `canvas_failure` 記 `canvas_failed` 事件（同一題／版本／顯示只記一次；runtime 載入失敗也記，而且只記這一次——
  runtime 沒載入成功就不問殼層 `canvas_active`，看門狗不會追蹤、不會誤判卡住）。**畫布框被換掉**（畫布把自己導到
  `about:blank`、`blob:` 或重新載入）時，加固腳本在 window 以 capture 監聽的 `pagehide` 送出這次 token 的 `error`
  （`canvas_navigation`「畫布換頁了」），面板隱藏畫布、顯示「圖表載入失敗：畫布換頁了」並記 `canvas_failed`；註冊檢查回報
  `canvas_navigation`。同文件的 `#id` 捲動不算換頁（WebKit 對它也會再觸發一次 iframe `load`，所以不以 `load` 次數判斷）。
  這則訊息送到時，WebKit 的 `event.source` 仍是這個 iframe（2026-10-09 原生實驗）；Chromium 已把來源文件丟掉、`source` 是
  null——訊息白名單（`messages.ts` `frameMessage`）唯一的例外：來源是 null、token 是這一次、而且每一條 issue 都是
  `canvas_navigation` 的 `error` 照收（只能讓這張畫布失敗，畫布本來就能自己回報錯誤）；iframe 已卸下時什麼都不收。
  **引擎層關閉清單**（`inv_canvas_sandboxed`）：每個 Kunsu webview（main、capsule、overlay、canvas-check）都經
  `windows::build_webview` 建立（導覽白名單、看門狗追蹤清理、引擎設定一次套用），建立後立刻在主執行緒以
  `_setPeerConnectionEnabled:NO`，並從 `+[WKPreferences _features]` 找出下列 key 以 `_setEnabled:NO forFeature:` 關閉（有該
  方法／feature 才呼叫，缺的記 warn）：`PeerConnectionEnabled`（WebRTC）、`LinkPreconnect`、`LinkDNSPrefetchEnabled`、
  `LinkPreloadEnabled`、`LinkPrefetchEnabled`、`SpeculationRulesPrefetchEnabled`、`WebTransportEnabled`。設定存在 UI 行程，
  之後產生的文件、子框架、`blob:` 文件與終止後重新啟動的網頁行程都生效：畫布沒有 `RTCPeerConnection`，`<link rel=
  preconnect|dns-prefetch>` 不發 TCP 連線也不查 DNS。**引擎設定才是保證**；以下都是縱深防禦，各有已知繞過，不能單獨
  依賴。沙盒的加固腳本（`installCanvasGuard`，TSX 與 HTML 畫布都在任何畫布內容之前執行）移除 WebRTC 全域與
  `XSLTProcessor`，攔下以 DOM API（`rel` setter、`setAttribute(NS)`、`relList`、Attr 節點）把 `rel` 設成資源提示，並把
  字串解析的輸入裡的 `<link>`／`<前綴:link>`（含結尾標籤）改成無作用的元素名稱、`<!ENTITY` 改成解析錯誤（XML 內部實體展開
  能拼出標籤）：`innerHTML`、`outerHTML`、`insertAdjacentHTML`、`setHTMLUnsafe`、`Document.parseHTMLUnsafe`、
  `Range.createContextualFragment`、`DOMParser.parseFromString`（HTML 與 XML）、`execCommand('insertHTML')`、
  `document.write`／`writeln`——後者跨呼叫拆開的標籤（`write('<li')`＋`write('nk …>')`）：結尾沒寫完的標籤先留著，下一次
  write 時合併再過濾；到這段腳本結束（microtask）或 `document.close()` 還沒寫完，就當文字輸出（`&lt;…`），不會和後面的
  文件原始碼拼成標籤。**已知繞過（只剩引擎設定擋）**：畫布自己開的子框架（srcdoc／about:blank）裡的程式碼不經加固腳本；
  任何能不經上述 API 在執行期讓 parser 產生 `<link>` 的新寫法（加固腳本是字串過濾，不是完整的 HTML／XML parser）。另一條 CSP
  管不到、只靠引擎設定的通道（2026-10-09 原生實驗）：畫布點了被沙盒擋下的 `target=_blank`／`_top` 連結時，WebKit 仍會先對
  連結目標開一條 TCP 連線（不送資料）；套用引擎關閉清單後沒有連線。
  **沙盒文件的順序**（`buildSrcdoc`，2026-10-09 QA 第 6 輪改）：head＝`<base href="about:srcdoc">`、CSP、charset、viewport、
  基本樣式；body 依序是資料區塊（`<script type="application/json" id="kunsu-canvas">`）、加固腳本、runtime，最後才是
  `<div id="root">`。TSX／JSX 畫布的 root 是空的（`</div></body></html>` 結尾）；**HTML 畫布的內容放進 root、是整份文件的最後
  一段，後面沒有任何 Kunsu 的東西**——parser 讀到畫布 HTML 之前，Kunsu 的資料區塊、加固腳本與 runtime 都已經解析、執行完，
  畫布 HTML 怎麼寫（沒關的註解、raw text 元素、`<template>`、`<svg>`／`<math>` 外來內容與 integration point、`<plaintext>`、
  `<select>` 裡的標籤、`<frameset>`、沒寫完的標籤）都吞不掉它們，不依賴組裝的切法正確（QA 第 6 輪：近似的 tokenizer 和
  瀏覽器切法不同時，放在畫布之後的資料區塊與 runtime 曾變成 template 內容、SVG／MathML 的 script 或純文字，面板 2 秒逾時）。
  加固腳本緊接在資料區塊之後執行，安裝時就記下這次的 token 與畫布種類（`pagehide` 回報用；畫布之後刪掉資料區塊或以
  `<frameset>` 換掉 body 也照送）；runtime 開頭先記下上層視窗（訊息通道）並讀完資料區塊，之後畫布改掉 `window.parent`、
  資料區塊都不影響回報。
  **組裝 HTML 畫布**（`ui/src/canvas/srcdoc.ts`＋`htmlTokens.ts`）：換行先統一成 LF（HTML parser 的前處理本來就把 CRLF 與
  單獨的 CR 換成 LF，解析結果不變；拿掉標籤補的換行才不會和前面的 CR 併成一個 CRLF 而少算一行），`./assets/x.png`（或
  `assets/x.png`）換成快照 data URI，再用 tokenizer 走過一遍。tokenizer 照 HTML 規格的 data／RCDATA／RAWTEXT／script data〔含 escaped 與 double escaped〕／
  PLAINTEXT 狀態、註解、`<!…>`／`<?…>`、屬性值引號切 token；外來內容（`<svg>`／`<math>`、integration point、CDATA）與
  `<template>` 只是**近似**的樹建構，不追蹤一般 HTML 元素的堆疊與 `<select>`／`<table>` 等插入模式——少見的寫法會和瀏覽器切得
  不一樣（QA 第 6 輪的 parse5 差分測試 40 萬筆找出 258 筆），所以組裝只把它用在切錯也無害的地方：真正的 `<link>` 與帶
  `http-equiv` 的 `<meta>` 拿掉（保留換行，行號不變）；傳統 inline script 轉成 data: 腳本（見下）；`<plaintext>` 改成
  `<pre>`＋跳脫後的內容；沒寫完的標籤拿掉。`<script>`、`<style>` 等 raw text 內容、註解、屬性值與 CDATA 裡的 `<link`、
  `<meta http-equiv`、`</script` 字樣不當成標籤處理。切錯的最壞結果是某個 `<script>` 沒轉成 data: 腳本（錯誤只剩
  `Script error.`、沒有行號）或某段被誤轉；吞不掉 Kunsu 的東西（都在前面），也開不出 `<link>`（下一段）。畫布 HTML 不再補
  結尾（後面沒有東西要保護，沒關的結構交給 parser 在文件結尾收）。
  **`<link>` 的最後一道**（不依賴 tokenizer）：組裝結果裡每一個 `<link` 後面接空白、`/`、`>` 或結尾（ASCII 不分大小寫）一律
  改成 `&lt;link`。parser 只會從字面的 `<link` 開出 link 元素，所以畫布 HTML 原始碼不論瀏覽器怎麼切、拿掉的標籤前後怎麼拼接
  （`<<link>link rel=preconnect …>`），都產生不了 `<link>` 元素。屬性值、`<title>`／`<textarea>` 裡解碼後的文字不變；
  `<style>`、`<xmp>`、註解、CDATA 與非 JavaScript 的 `<script>`（含 `type=module`）內容裡的字面文字多出 `&lt;`（沒有作用）；
  傳統 JavaScript 已轉成 data: 腳本（內容 URL 編碼），不受影響。`<meta http-equiv>` 由 server 的靜態檢查保證（畫布原始碼出現
  `http-equiv` 就不給註冊；屬性名稱不解碼字元參照，只能字面寫出），組裝拿掉是縱深防禦。
  TSX 與 HTML 的靜態檢查都把 WebRTC 與資源提示 `rel` 列為 `network_api`（HTML 畫布逐行比對、不看是不是註解，與組裝的
  tokenizer 互相獨立）。HTML 畫布**先解碼字元參照再比對**（`network_api` 與 `external_url` 都是）：數字參照（十進位、十六進位，
  有沒有 `;` 都算）與解碼成 ASCII 的具名參照（`&Tab;`、`&NewLine;`、`&colon;`、`&sol;`、`&fjlig;` 等），解碼出的空白一律當
  一個空格（不改變行號、欄位指回原文）——`rel="&#112;reconnect"`、`http&colon;//` 一樣抓得到。靜態檢查仍是逐行的文字比對：
  跨行寫的值、JavaScript 執行期組出來的值（`'pre' + 'connect'`）抓不到，由加固腳本與引擎設定擋。
  **HTML 畫布的檢查與錯誤**：資料區塊對 HTML 畫布也帶原始碼與 `line_offset`（畫布 HTML 第一行之前的行數，含資料區塊、加固
  腳本與 runtime；換行以 LF、CR、CRLF 各算一行，與 parser 相同）。runtime 在畫布內容之前就先跑靜態檢查（`network_api`、
  `external_url`，有就立刻回 `error`），並以 capture 監聽 `error`／`unhandledrejection`：畫布 `<script>` 的例外、語法錯誤、
  未處理的 Promise 拒絕當下回 `error`（`canvas_runtime`）。文件 `load`（capture 監聽）時再讀一次加固腳本收集的錯誤作後備，
  都沒有才等版面穩定回 `ready`；之後的錯誤一樣回報。沙盒文件時間（`performance.now()`，從畫布文件開始載入算起）到 1.7 秒
  還沒 `load`、也還沒回報任何結果時（計時器跑得到，表示主執行緒沒有卡住）回 `error` `canvas_incomplete`「畫布 HTML 沒有載入
  完成（文件狀態 loading／interactive／complete）」，提示檢查 `window.stop()`、沒有 `close()` 的 `document.open()`、載入不到
  的資源與過重的計算——不是看不出原因的 2 秒逾時；真的卡住主執行緒（無窮迴圈）時 runtime 什麼都送不出，面板照舊 2 秒逾時、
  註冊檢查回 `canvas_timeout`。TSX／JSX 畫布：runtime 等 `DOMContentLoaded`（root 已解析）才編譯、掛載。
  註冊檢查與正式顯示走同一條路。WebKit 會把無同源文件裡 inline script 的錯誤遮成 `Script error.`（沒有訊息與
  行號，2026-10-09 原生實驗確認），所以畫布 HTML 裡沒有 `src`、型別是 JavaScript 的傳統 `<script>` 在組裝時改成
  `<script src="data:text/javascript;kunsu-line=N;charset=utf-8,…">`（`N`＝內容第一行在 canvas.html 的行號；原本的換行
  放進 `data-kunsu-lines` 屬性，行號不變；仍是 parser 依序執行的阻塞腳本，`document.write`、`currentScript` 照常；
  `async`／`defer` 對 inline script 本來無效，轉換時拿掉以維持原本的執行順序；只轉 HTML 命名空間裡的 `<script>`，`<svg>`
  裡的 script 與 `type=module`、非 JavaScript 型別不動）——data: 腳本的錯誤帶完整訊息。行號換算：data: 腳本＝`N`＋錯誤行號－1；
  其他（文件本身）＝文件行號－`line_offset`；換算不出來就不帶行號，不亂指。
  **面板無回應偵測**：面板執行畫布前先以 IPC `canvas_active {question_id, revision}` 問殼層，回 `false`（這一版已被停用）
  就不執行、直接顯示「圖表載入失敗：畫布沒有回應」（卸下時 `canvas_inactive`）；許可以「題目＋版本」為 key，換版後新版
  拿到許可前不執行；IPC 失敗視同拒絕，顯示「圖表載入失敗：無法確認畫布可以執行」並記 `canvas_failed`；
  畫布作用中殼層每秒 eval 心跳、面板以 IPC `surface_pong` 回應，連續 3 秒沒回應 → 殼層記 `canvas_failed`（「畫布沒有
  回應」）、把該題該版本加入停用清單，以 `_webProcessIdentifier` 終止該 webview 的網頁行程（SIGKILL，只影響那個視窗）
  後重新載入；蓋板只在卡住當下 session 還開著時重新 request_show（Esc 後備、勿擾、kill-switch 已收掉就不叫回）；卡住當下
  有尚未被網頁確認的 Esc（`escape_at`）時，`hang_recovery` 改送 `NativeDismiss`＋`Hide`（與 Esc 原生後備相同的 dismissal），
  不重新呈現。看門狗只追蹤活著的同一個網頁：視窗 `Destroyed` 或網頁 `PageLoadEvent::Started`（重新載入）時清掉該視窗的
  追蹤；判定卡住前再確認視窗還在，不在就只丟掉追蹤，不停用、不記 `canvas_failed`；`get_evidence` 對停用版本回 `{canvas: null,
  canvas_disabled: "畫布沒有回應"}`，面板顯示「圖表載入失敗：畫布沒有回應」。停用清單存在記憶體，app 重開即清空。
  **導覽白名單分主框架與子框架**（`inv_canvas_sandboxed`）：每個 webview（main、capsule、overlay、canvas-check）的主框架經
  `on_navigation` 只放行 `tauri://localhost`、`http(s)://tauri.localhost`、`about:srcdoc`、`about:blank`、`blob:`（開發版
  另放行 vite）；**子框架**（畫布 iframe 與它自己開的巢狀框架）只能停在 `about:srcdoc`、`about:blank`（兩者可帶同文件
  片段，WebKit 回報成 `#…` 或 `%23…`）與 `blob:`——app 自己的頁面、`data:`、任何網址一律擋，iframe 留在原本的畫布。
  wry 的 `on_navigation` 只給網址、分不出框架，所以殼層在 `webview_native::harden` 裡對 wry 的 navigation delegate 類別
  包一層 `webView:decidePolicyForNavigationAction:decisionHandler:`（整個行程一次）：`targetFrame` 存在且不是主框架時先以
  `navigation::allowed_subframe_navigation` 判斷，不准就直接 `Cancel` 並記 warn，其餘交回 wry 原本的實作（主框架白名單照舊）；
  delegate 沒有這個方法時記 warn（只剩 base URL 與 `pagehide` 兩層）。焦點不留在 iframe 內（Esc 與鍵盤永遠回到原生介面）。草稿題的
  所有 Markdown 欄位（內文、why、證據、選項）圖片只顯示同版本快照（data URI），外部圖片不載入、只留替代文字；sanitizer
  不允許 `picture`／`source`；HTML 畫布的 `<meta http-equiv>` 在
  srcdoc 組裝時濾掉。內文連結第一次點只提示，4 秒內再點同一連結才以 `open_deep_link` 用系統瀏覽器開（只限 http(s)）；
  沒有 href 的連結（sanitizer 拿掉 `javascript:` 等之後）點了無作用、永不顯示提示。
- **決策控制永遠原生**（`inv_controls_native`）：選項、送出與錢參數只來自 frontmatter／建題欄位；註冊永不產生答案。
- CLI：`kunsu ask <草稿資料夾> [--dry-run]` → `ask_register`；身分只帶偵測到的欄位；issues 逐行印到 stderr，
  不通過結束碼 3。

## 13. 排程督導（PRD §21；BDD routine-check-in）

- 建題時 `source.trigger` 沒填 → 有 `routine_id`（且不是 `kunsu-` 保留前綴）為 `scheduled`、否則 `interactive`；
  預設在 `create_inner` 套用，Kunsu 自己建的題（doctor 冒煙 `kunsu-doctor`、Kongko 啟用 `kunsu-kongko`）同樣適用；
  `kunsu-` 前綴即使 agent 自填 `scheduled` 也改成 `interactive`。CLI `--routine-name`／`--every`／`--routine-kind` 沒給值、
  或沒搭配 `--routine` 時報錯（結束碼 64）、不送出。MCP 建題參數型別錯誤時，錯誤訊息帶欄位路徑（例如
  `source.expected_every_hours`）。
- `source.expected_every_hours` 必須是有限正數，否則 400 `bad_routine_cadence`；CLI `--every` 解析失敗即報錯、不送出。排程身分（routine_id、routine_name、
  routine_kind、expected_every_hours）存在題目 `source`，跟著 ledger 快照，可由 `rebuild-db` 還原。
- 交付狀態改變都有 `delivery` 帳本行（§1 第 7 條），`rebuild-db` 重放後依題目上的 `answer.delivery` 還原 `deliveries`
  索引：pending／held 於重建時刻重檢、resuming 交給卡住規則（15 分鐘）、其餘已定案不再交付。
- 督導頁（主 app 側欄「督導」，`ui/src/routines/`）由 `list_questions(include_terminal)` 純前端整理，只取排程 check-in
  （有 routine_id、非 internal、非 `kunsu-` 前綴、`trigger = scheduled`，而且宣告了 `routine_name` 或
  `expected_every_hours`——只帶 routine_id 的營運排程決策題不算）：每個 routine 一列
  （名稱、來源圖示、最近報到、頻率、最近狀態〔待回應／已回應／未回應／退回／已取消〕、近 30 天已回應／未回應／退回）；
  「退回」＝題目有 `rejected`（使用者按「這題不該問我」），歷史寫「你退回了這題：<原因>」；超過宣告頻率
  兩倍沒報到只在該列標「排程可能沒在跑：已 N 小時沒報到」，不推播、不進瀏海或蓋板。點進去由新到舊列出每次 check-in：
  時間、題目、完整回答、AI 回饋（sanitized Markdown；沒有時「AI 尚未回報」）、交付狀態、打開原對話；沒有回覆框。
  `answer.channel = auto_policy`（逾時依預設自動選定）不算回答，顯示「沒有回答（逾時依預設：<選項>）」。


## 14. 題組（PRD §25；BDD batch-review）

- **型別**：`Kind::Batch`（`"batch"`）。`Question.items` / `CreateRequest.items: Vec<BatchItem>`（其他題型為空、不序列化）：
  ```rust
  BatchItem {               // deny_unknown_fields：打錯欄位名 400
    id: String,             // 題內唯一；路徑安全白名單同題目 id（ASCII [A-Za-z0-9._-]、≤120、不以 . 開頭）
    title: String,          // 非空、≤80 字（寄件人＋主旨、留言者…）；UI 以純文字顯示
    subtitle: Option<String>,
    context_md: String,     // 原文脈絡（來信全文、留言串、申請內容）；Markdown，左欄跟著選中項目顯示
    proposal: String,       // AI 的草稿或建議；非空；≤5,000 字（只核可的建議也一樣，回原對話的訊息才塞得進命令列參數）；UI 以純文字顯示
    editable: bool,         // true＝可改草稿（回信、留言）；false＝只核可（退款、分類、隱藏留言）
    default: approve|skip,  // 省略＝approve；skip 必須帶 default_reason（AI 建議不處理的原因）
    default_reason: Option<String>,
    content_hash: Option<String>,  // server 算：sha256（欄位正規化 JSON）；agent 帶了也會被覆寫
  }
  ItemDecision { item_id, action: approve|skip|revise, text?: String, note?: String, content_hash?: String }
  ```
  `kunsu_core::batch`：`item_hash`／`seal_items`（建題與草稿升版時重算每項雜湊）、`round_of`／`next_round_id`
  （`x` → `x-r2`、`x-r2` → `x-r3`）、`BatchCounts`（採用／你改過／不處理／請 AI 改）。
- **建題驗證**（`validate_create`，錯誤 400，訊息寫「items[i]（第 i+1 項，id）的 <欄位>」）：
  `batch_items_out_of_range`（1..=20）、`items_only_for_batch`（非題組帶 items）、`batch_no_options`（題組帶 options）、
  `batch_money_forbidden`（`money.is_money_op`；先於錢題的其他檢查）、`batch_item_id_invalid`、`batch_item_id_duplicate`、
  `batch_item_title_empty`、`batch_item_title_too_long`、`batch_item_proposal_empty`（例外：`editable` 且 `default: skip`
  的項目可以是空草稿——AI 建議不處理時給空草稿或真的回覆，不准佔位文字，2026-10-09 Ray）、`batch_item_proposal_too_long`
  （>5,000 字，含只核可的建議）、`batch_item_subtitle_too_long`（副標 >200 字）、`batch_item_skip_needs_reason`、`batch_kongko_ref`（題組帶 `kongko_ref`：Kongko 發佈事件沒有
  逐項決定，不能結案題組）、`batch_id_too_long`（`round_of(id).0` 超過 `MAX_BATCH_BASE_ID_LEN`＝116 字：要留 `-r2`～`-r99`
  的空間，題目 id 上限 120 字）。紅燈題本來就必須 choice（`red_gate_needs_choice`）。
  `kongko_event` 命中的題若是題組（建題規則之前的舊資料）：不結案、`tracing::warn` 留痕，回 `closed_question_id: None`。
- **Markdown 安全**：`context_md` 沿用草稿的 Markdown 檢查規則（禁用標籤、`on*`／style／class／srcset 屬性、
  `javascript:`、外部與 data: 圖片）。草稿註冊：問題逐條回報（行號對到該項 `- id:` 那一行），`./assets/` 圖片照常收進
  版本快照；JSON `ask_create`：任一問題 400 `batch_item_markdown`（指出第幾項），JSON 題組的原文不能引用圖片（要附圖用草稿註冊）。
- **草稿註冊**：frontmatter 的 `items:` 與 JSON 相同；同 id 重新註冊升版時一併換掉 items（重算每項雜湊）。
- **ask_update**：題組的 `options` 一律拒收（題組沒有選項）；items 只能隨草稿重新註冊改變，JSON 題組建立後不可改。
- **作答**（`AnswerInput.items: Option<Vec<ItemDecision>>`；題組的 `free_text`＝可選整體指示）：
  - 題組必須帶 `items`，不得帶 `choice_id`／`edited_params`；非題組帶 `items` 拒收。
  - 每個項目恰好一次（不得缺、重複或出現不存在的 item_id）。
  - `approve`：可編輯項目必須帶非空 `text`（trim 後非空，≤5,000 字，這就是要送出的最終文字；空的錯誤訊息「第 N 項沒有
    內容，先寫好再採用」）；只核可項目不得帶 `text`。
  - `skip`：不得帶 `text`；`revise`：必須帶非空 `note`（≤2,000 字，錯誤「第 N 項的指示（請 AI 改）超過 2,000 字」），不得帶
    `text`；整體指示 ≤2,000 字，**先於逐項檢查**（錯誤「整體指示（套用到所有項目的那段）超過 2,000 字」，全部退回時不會被
    誤報成第 1 項）。
  - `channel=ui` 每項都要帶面板顯示時的 `content_hash`，與目前該項雜湊不同即拒收（請重新開啟面板）；草稿題另外照 §12
    綁 `revision`／`content_hash`。CLI／Discord 載荷沒有逐項決定，題組只能在面板作答。
  - 違反時 400 `bad_answer`，題目維持未答。答案依題目項目順序存 `answer.items`，每項寫入當下的 `content_hash`；
    `answered` 帳本快照、`ask_get`／`ask_list`／`ask_wait`／`ask_answers_since` 都原樣帶出逐項決定（題組沒有錢欄位要去敏）。
- **IPC**：`answer_question {type: "batch", question_id, items: [{item_id, action, text?, note?, content_hash}],
  instruction?, revision?, content_hash?}` → `AnswerInput { items, free_text: instruction, channel: ui }`。
- **延續原對話**（`continuation_prompt`）：題組改列「第 N 輪，共 M 項」與總數、整體指示、逐項決定（`item_id`、決定、
  使用者是否改過；採用的最終文字放在 fenced block 原樣保留；請 AI 改附指示），並寫明規則：採用的文字一字不改送出、
  不處理的不要動、請 AI 改的先依指示重寫，再以 `next_round_id` 建第 2 輪（或下一輪）題組，只含要重看的項目、
  沿用 item_id、標明第幾輪與原題 id；`ask_report` 寫明已執行幾項、重寫幾項與新題組 id。開新對話的 prompt 除了題目 JSON（含
  `answer.items`）也附上同一份「逐項決定／接下來」段落；prompt 一律走 stdin（§8 第 5 步），最大題組也交得出去。
- **UI**（`ui/src/surface/BatchColumn.tsx`＋純函數 `batch.ts`）：`DecisionSurface` 依 `kind === 'batch'` 切換；左欄＝
  為什麼問你＋選中項目的原文（`context_md` 走 `RichContent`）＋題目證據；右欄＝兩個一鍵按鈕、項目卡清單（list）、
  總結列與送出。鍵盤 J／K（↑／↓）換項目、E 編輯、Esc 離開輸入框（只有焦點在 textarea／input／contenteditable 才算；
  其他地方＝稍後再問）、A／X／R 決定、⌘Enter 送出。面板 < 1100 px（與決策面板同一個 `@container surface`）單欄時整個
  面板一起捲、送出列貼底，選中的卡片內顯示原文；卡片標題換行不省略。
  送出條件（`sendBlocker`）：採用的可編輯項目文字 trim 後是空的 →「第 N 項沒有內容，先寫好再採用」；全部退回
  （`redoAllPayload`／`redoAllBlocker`）指示 trim 後超過 2,000 字不組載荷、按鈕停用並寫「整體指示超過 2,000 字（N 字）」。
  草稿框（`AutoGrowTextarea`）值改變與寬度改變（ResizeObserver）都重算高度。草稿重新註冊（項目雜湊改變）時
  `rebaseDrafts(oldItems, newItems, drafts)`：AI 草稿（`proposal`）沒變的項目保留決定、文字與指示；變了的項目標
  `updated`（卡片寫「AI 更新了這一項」），使用者改過的文字保留、沒改過的換成新版本，「還原 AI 版本」還原成新版本。
  蓋板與主視窗的作答流程都把最新清單交給面板（§9 `latest`），所以兩邊都會 rebase。
  回原對話追加（`claude -p <prompt> --resume`）的 prompt 在命令列參數：靠建題上限（建議 ≤5,000 字、副標 ≤200 字、標題 ≤80 字、
  20 項）保證最大題組的追加訊息遠小於 macOS ARG_MAX（`qa2_br06_resume_prompt_for_the_largest_valid_approve_only_batch_fits_the_command_line`）。
  回執：題目走 `RichContent inline`；送出後焦點移到「下一題／完成」；Enter 依 `focusKind`，焦點在連結或其他按鈕上不跳題。

## 15. 主視窗設定與狀態（PRD §27；BDD settings、application-shell；Target）

> 進度（2026-10-09）：後端已實作、待 QA——core `Policy::layered`／`SettingsView`／`quiet_enabled`、server
> `ServerHandle::settings_*`／`reload_policy_files`／`app_status`（`kunsu_server::settings`）、shell IPC 六個 command、啟動與熱載讀兩檔、
> 主視窗外框、`kunsu doctor` 的設定行。主視窗設定頁 UI 另一路實作中；QA 通過前本節仍是 Target。

**Policy 疊層。** `kunsu_core::policy::Policy::layered(base: &str, local: Option<&str>) -> Result<Layered, LayerError>`（純函數）：
以 TOML 表逐鍵把 local 疊在 base 上（local 只能出現下列可設定鍵，其他鍵忽略並回報），再驗證範圍；`Layered { policy, overridden,
ignored }` 是生效 policy、被覆寫的鍵與被忽略的鍵（都是 `"force.cycle_min"` 這種路徑）。`LayerError` 是 `Syntax(訊息)`（local 不是
合法 TOML）或 `Invalid([{key, message}])`（每個不合法的鍵一筆人話訊息，帶範圍）。base 本身不合法時照舊退回 `Policy::default()`。
整數鍵只收整數；`skip_rate_downgrade` 收整數或小數；`quiet_hours` 只收兩個 24 小時制兩位數 `HH:MM` 字串。範圍（含端點）：

| 鍵 | 範圍 | 預設 |
|---|---|---|
| `force.cycle_min` | 5–120 分鐘 | 15 |
| `force.warn_before_min` | 1 到 `cycle_min − 1` 分鐘 | 3 |
| `windows.quiet_enabled` | true／false | true |
| `windows.quiet_hours` | `["HH:MM","HH:MM"]`，兩者不同，可跨午夜（台北時間） | `["23:00","09:00"]` |
| `expiry.t1_default_after_h` | 1–336 小時 | 48 |
| `expiry.t2_supersede_after_d` | 1–14 天（INV-14 上限）；設定頁叫「題目最長等待天數」：所有題目的壽命上限，策略題（T2）到期由新題取代、其他題到期結案 | 14 |
| `quota.per_source_daily_create` | 1–100 題 | 12 |
| `fatigue.skip_rate_window_d` | 1–30 天 | 7 |
| `fatigue.skip_rate_downgrade` | 0.1–1.0 | 0.5 |

`windows.quiet_enabled = false` 時 `in_quiet_hours` 恆 false。`deferred_retry_s`、`t1_batch`、`discord.webhook` 不開放設定頁修改。

**檔案。** server 端 `ServerHandle::settings_*` 負責讀寫 `~/.kunsu/policy.local.toml`：寫入是 tmp＋rename 原子替換、只寫可設定鍵、
保留檔頭註解「由 Kunsu 設定頁管理」；寫入前先用新值計算 `Policy::layered`，不合法就整筆拒絕、檔案不變。成功後立即
`set_policy`（不等熱載）。熱載輪詢同時看兩檔的 mtime（5 秒）：base 讀不到或解析失敗時沿用上一次的生效值（與原本
policy.toml 熱載相同）；local 不合法時只用 base。路徑由 shell 注入（`ServerHandle::set_settings_paths`；
`KUNSU_HOME_DIR` 覆寫時跟著改），沒注入時 `settings_*` 回錯誤、不碰任何檔案。設定改動永不回答、取消或重設任何題目的週期，既有題目的
`cycle_started_at`／`warn_at`／`force_at` 不重算；新週期（作答後的下一題、稍後再問、新題）才用新值（INV-26）。到期參數
（`expiry.*`）在建題時固定在題目上（§2 `expiry_after_h`／`max_pending_d`、§3），所以調低到期時間只影響之後新建的題，已在等的題
不會因此提早到期或被依預設作答（2026-10-09 主線代 Ray 決定；取代 QA 第 1 輪記錄的「下一次 tick 就讓舊題到期」）。

**IPC（主視窗）。**
- `settings_get` → `{ effective: SettingsView, defaults: SettingsView, overridden: [key], local_path, local_error? }`；
  `SettingsView = { force: {cycle_min, warn_before_min}, windows: {quiet_enabled, quiet_start, quiet_end}, expiry: {t1_default_after_h,
  t2_supersede_after_d}, quota: {per_source_daily_create}, fatigue: {skip_rate_window_d, skip_rate_downgrade} }`。
- `settings_set { changes: { "<section.key>": value } }` → 同 `settings_get`；不合法回 `{ errors: [{key, message}] }`（人話、帶範圍），
  不寫入任何值。`changes` 的鍵只能是上表的鍵（`windows.quiet_hours` 是 `["HH:MM","HH:MM"]` 陣列；`SettingsView` 的
  `quiet_start／quiet_end` 只是讀取面）；不在表上的鍵、型別不對或 `null` 都整筆拒絕並回該鍵的錯誤。IPC 以 Promise reject 回傳
  `{ errors }`；檔案層的錯誤（local 檔語法壞掉、讀寫失敗）的 `key` 是 `"policy.local.toml"`。local 檔只有某些鍵超出範圍時，
  `changes` 把它們改回範圍內就能存。
- `settings_reset { keys: [key] }` → 從 local 檔移除這些鍵（回到預設），回 `settings_get`。`keys: []`＝全部回復預設；不在表上的鍵
  拒絕（同上 `errors`）；local 檔語法壞掉時任何回復都清空整個檔（只留檔頭）；移除後剩下的組合不合法（例如回復週期後留下的提前
  警告 ≥ 預設週期）整筆拒絕。
- `app_status` → `{ version, uptime_s, server_addr, vault_root, db_path, ledger: {backlog, open_json_dirty, stuck_s},
  integrations: [{ id: "claude_mcp"|"codex_mcp"|"cli"|"skill", status: "ok"|"missing"|"unknown", detail, fix? }] }`。
  檢查與 `kunsu doctor` 同源：`~/.claude.json` 的 `mcpServers.kunsu`、`~/.codex/config.toml` 的 `[mcp_servers.kunsu]`
  （與 AC-LP-05 的 doctor 規則一致：直接連線沒寫 `enabled` 視為啟用、`enabled = false` 不算；啟用的本機外掛
  `[plugins."kunsu@…"]` 也算）、`~/.kunsu/bin/kunsu` 可執行、`~/.claude/skills/kunsu/SKILL.md` 存在；`fix` 是給使用者複製到終端機
  執行的指令（不由 app 執行）：Claude MCP 是 doctor 印的 `claude mcp add …` 指令、Codex MCP 是安裝本機外掛
  `python3 ~/Desktop/kunsu/plugins/kunsu/scripts/manage.py install`（§16；doctor 印同一行，2026-10-09 取代貼 `[mcp_servers.kunsu]`
  設定段——那段還要自己補 token 設定，不能直接貼上就用）、CLI 是重跑 `scripts/build-install.sh`；skill 沒有 repo 內正本，不給 `fix`。`detail`
  寫出實際檢查的地方：Claude 是「Claude Code（~/.claude.json）」，主視窗的說明也寫 Claude Code，不說 Claude App。檔案存在但讀不到或
  格式壞掉回 `unknown`。只讀檔案存在與鍵是否存在，不讀、不回傳任何 token 或設定內容。
- `edge_reset_position` → 刪掉保存的瀏海位置（`~/.kunsu/edge-notch.json`；沒有檔案也算成功），記住的螢幕改回主螢幕，刪檔成功後
  發 `kunsu://edge_position_reset`（只送瀏海視窗；`edge::reset_saved_position`，2026-10-09 QA 第 5 輪起取代借用
  `kunsu://edge_stage_changed`），舞台可見時立即貼回主螢幕（貼合的螢幕換了照 §9 另發 `kunsu://edge_stage_changed`）。網頁收到
  `edge_position_reset` 時取消待寫的存檔、清掉記憶體位置、回到預設位置（§9「資訊與位置」）；刪檔失敗回錯誤、不發事件、瀏海不動。
- `open_privacy_settings` → 只打開 `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`（完全取用磁碟）；
  不接受任何參數。
- 會改東西的 `settings_set`、`settings_reset`、`edge_reset_position` 只收 main 視窗（瀏海、蓋板網頁呼叫一律拒絕；設定 command 回
  `{errors: [{key: "policy.local.toml", message}]}`）；`settings_get`、`app_status` 不限視窗。
- 外觀（`kunsu:theme:v1` = `system|light|dark`）與動態（`kunsu:motion:v1`）是每台 Mac 的呈現偏好，存 localStorage，不進 policy。

**主視窗外框。** main window 用 `TitleBarStyle::Overlay`＋隱藏標題，預設 1180×760（內容面板一打開就是寬版）、最小 900×600；側欄頂端 44 px 是拖曳區
（`data-tauri-drag-region`，需要 `core:window:allow-start-dragging`，只授權給 main 視窗）。

## 16. 本機 Codex 外掛（Current，2026-10-09）

- Source：`plugins/kunsu/`，portable Agent Plugins 1.0 與同步 `.codex-plugin` compatibility manifest，版本 0.2.0。
- 安裝：`~/plugins/kunsu/` → personal marketplace → Codex plugin add。MCP server 叫 `kunsu-inbox`；舊獨立
  `mcp_servers.kunsu` 停用，名稱分開避免舊 server 的 enabled=false 套到外掛。
- stdio：bare executable `python3 -u ${PLUGIN_ROOT}/scripts/mcp_bridge.py`；stdlib only，stdout 只有 JSON-RPC。
- transport 固定 `http://127.0.0.1:4777/mcp`，每次從 `~/.kunsu/token` 讀 Bearer；不跟隨 proxy／redirect，
  不把 token 寫進外掛或新設定。來源身分由 CLI 環境補齊或 MCP caller 顯式提供，bridge 不猜共用行程身分。
- bridge 僅接受 initialize、ping、tools/list、tools/call 與 CONTRACT §4 的 14 tools；unknown tool／非字串
  name 回 error，不轉送。token 缺失／app 不通回可操作錯誤、不回傳 exception／憑證／原請求；下一次可恢復。
- 本機 Kunsu 的 plugin server 預先允許工具呼叫；只有建題、讀題與呈現 guard，不新增作答／批准方法。
  其他外掛與 MCP 的批准設定不變。Claude 既有 HTTP 註冊保留，Claude/Codex 共用 Kunsu skill 操作 SOP。
- doctor 的 Codex 註冊檢查接受 enabled 的直接 kunsu MCP 或 `kunsu@personal` 外掛；兩者停用時失敗。
  這是設定檢查；真實握手／宿主工具呼叫另依 BDD 驗證，不把設定值當成已執行證據。
- 原生 icon、favicon、plugin logo 由 DESIGN 指定同一原圖打包；menu-bar template 與來源 AI 圖示不變。
- 維護入口 `python3 scripts/manage.py install|doctor|draft`，Python 3.11+、stdlib only。install 先驗證 package／TOML／personal marketplace／loopback，備份設定、source 與共用技能，再以宿主 plugin add 安裝；只改 Kunsu 啟用／批准欄位與自己的 marketplace entry，不直接改 cache。兩次實際安裝皆通過 source/cache 比對；共用技能額外檔案保留。
- 設定／marketplace 備份為 0600，備份根為 0700。失敗回復此次本機檔案並保留嘗試 source；若內容與本次寫入值不符則保留外部並行修改、回 error。宿主部分 cache 需透過原生 plugin add 回復／確認，不宣稱本機檔案 rollback 等於宿主 cache rollback。
- doctor 只做 initialize、tools/list 與 dnd_get，14 tools 不完整或服務不通即失敗。draft 只建立 ~/.kunsu/drafts/<id>/question.md，拒絕非法 pa-/rt- id 與既有草稿；五種模板通過 installed-app dry-run，沒有建題或作答。

## 16. 私人助理草稿（Current, 2026-10-10）

Batch 支援 1–20 項；沿用 BatchItem 欄位與內容雜湊，subtitle 的固定平台前綴屬於 hash 保護內容。UI 來源解析只讀 subtitle，不推測客戶姓名。未知來源列「其他」。本機草稿快取存 question id、items 基線與逐項文字/action/note/reviewed；載入時驗證 shape、依 item id 對齊並 rebase。AI 更新任何來源／上下文／草稿後 reviewed 失效，使用者編輯保留並標出更新。終態不復用可送出的草稿。UI 快取不是真實答案，MCP 不提供作答。只有正式 batch answer 可成為執行依據。

### Codex queue fallback（Current；端到端接收待驗證，2026-10-10）

`resume` 的 Locked 分支先嘗試相同 CLI `queue --thread <uuid> --message <bounded control prompt>`。exit=0 且 stdout 有 `Queued message … for thread <uuid>.` 才記 delivery=resumed、reason「已排入原對話，等待 AI 接手」；這代表交付接受，不代表執行或完成。UI 對此 reason 顯示等待且 done=false，收到 report 後才停止等待。queue 不支援／失敗維持 held；任何外部執行依各工作流 durable ledger 去重。不上新 daemon、不解除 App writer lock。

## 17. 手機作答通道與配對（Current，2026-10-12；PRD §30.8、docs/design/mobile-relay.md）

- **通道**：`Channel::Mobile`（serde `mobile`）。`Channel::is_ray_device()`＝ui 或 mobile：錢題作答、草稿題版本檢查、approval
  verify、錢鏈適用判準與重新授權都認這兩條（INV-15）。MCP 面仍然沒有作答寫入（INV-7）。
- **Mac 橋接**：`kunsu_server::mobile::MobileBridge`，Tauri 開機時載入並常駐；主動連 `KUNSU_RELAY_URL`（預設
  `wss://api.kunsu.app/v1`；舊的 `wss://kunsu-relay.zeabur.app/v1` 是同一個服務，已存的設定照常可用）。檔案都在 `~/.kunsu/`、權限 0600：`mobile-keys.json`（Mac 的 X25519／Ed25519）、
  `mobile-devices.json`（裝置清單：公鑰、名稱、配對時間、最後 seq、撤銷）、`mobile.json`（中轉站網址、中轉 AI）、
  `relay-token`（Mac 上線中轉站的 Bearer）。不用鑰匙圈。
- **手機指令**（解密後的 JSON）：`sync` → `snapshot {questions[], history[], relay_agent}`；`answer {question_id, choice_id?, text?,
  money_confirmed, revision?, content_hash?}` → 以 channel=mobile 作答，錢題沒有 `money_confirmed: true` 拒絕，錢題帶與 UI 相同的
  `ui_answer_signature`，題組拒絕；`settings {relay_agent}`；`followup`／`forward`／`chat`／`start_task`（見下方「中轉 AI」）。
  `permission`／`reply`（等你回話與權限批准的對話控制）回 `ack {ok: false}`（下一階段）。快照另帶 `projects[]`（題目來源資料夾的
  名稱，最近的在前，最多 12 個）。
  題目變化時（server 事件，400 ms 合併）推快照給所有未撤銷裝置。
- **主視窗 IPC**（只收主視窗）：`mobile_status`、`mobile_start_pairing`（→ `{payload, svg, expires_at}`）、
  `mobile_decide_pairing {id, allow}`、`mobile_revoke {device_id}`、`mobile_set_relay_agent {agent}`；狀態變化時對主視窗發
  `mobile_changed` 事件。
- **中轉 AI（2026-10-12，PRD §30.7）**：`kunsu_server::relay_ai`。Mac 上以 CLI 跑一次性的唯讀工作階段，人格是「軍師」：
  - Claude：`claude -p --output-format text --permission-mode dontAsk --allowedTools Read Grep Glob --strict-mcp-config
    --mcp-config {"mcpServers":{}} --append-system-prompt <軍師守則>`，提示從 stdin 給。
  - Codex：`codex exec --sandbox read-only --skip-git-repo-check -C <cwd> -o <暫存檔> -`，提示從 stdin 給。
  - 沒有任何 MCP、不能寫檔或跑指令（INV-32）；工作目錄是題目的 `source.cwd`（找不到就家目錄）；逾時 180 秒；子行程清掉
    `CLAUDE_CODE_*` 等繼承身分（同 reactor）。用哪一個看 `mobile.json` 的 `relay_agent`。
  - `followup` → 提示含題目、為什麼、證據、選項與這題之前的追問；回 `followup_answer {by: relay}`。
  - `chat` → 提示含 Kunsu 現況摘要（待決題、48 小時內的答案與結果）與同一支手機最近 12 則聊天（只在記憶體）；回 `chat_reply`。
  - 守則寫明：只解釋與整理，不替使用者作答、不批准、不建議繞過 Kunsu；這是說明而不是安全邊界，安全邊界是 INV-7／INV-32。
- **問本人（2026-10-12）**：`forward` → 只有原 AI 正在等這題（`waiter_status` 在線或 10 秒租約內）才收下，放進記憶體的追問佇列；
  否則立刻回 `followup_answer {ok: false, error: "<專案> 現在沒在等這題，只能問軍師"}`。`ask_wait` 遇到還沒交出的追問就立刻回
  `{status: "followup", followup: {id, text}, hint}`（每則只交一次）；原 AI 用 MCP `ask_followup_reply {question_id, followup_id, text}`
  回答（≤4,000 字），Kunsu 轉成 `followup_answer {by: agent}` 推給所有裝置。`ask_followup_reply` 只存文字，永不改題目狀態、倒數或
  答案（INV-31）。15 分鐘沒回，手機收到「原 AI 15 分鐘內沒回」。`kunsu wait` 收到追問以結束碼 6 結束並印出追問與回覆指令
  `kunsu followup-reply <題目 id> <追問 id> "<回覆>"`，回完再背景執行 `kunsu wait <id> --timeout 0`。
- **開新任務（2026-10-12）**：`start_task {agent, project}` 的 `project` 必須是快照 `projects[]` 裡的一個（對回 Mac 上的資料夾），
  否則拒絕。Claude：`claude -p --permission-mode auto <text>`；Codex：`codex exec --approve-for-me -C <cwd> <text>`（PRD §30.3
  預設 Auto，不提供繞過權限）。在背景跑、30 分鐘逾時，開始回 `task_started`，結束回 `task_done`。任務本身需要使用者時照常用 Kunsu 建題。
- **中轉站**：`relay/server.js`，部署在 Zeabur 專案 `kunsu-relay`（專屬伺服器 Linode Tokyo），環境變數 `RELAY_MAC_TOKEN`。
- **驗證台**：`cargo run -p kunsu-server --example mobile_e2e`（暫存資料庫＋另一組 Mac 金鑰，不碰真的 Kunsu）。


## 18. 帳號與多使用者（Current，2026-10-13；PRD §31.2、§31.3；設計 docs/design/accounts.md）

- **帳號服務**：`relay/cloud.js`＋`relay/main.js`（服務名 kunsu-cloud），部署在 Zeabur 專案 kunsu-relay，與中轉站同一個網址
  （`https://api.kunsu.app`，舊網址 `https://kunsu-relay.zeabur.app` 同一服務）；資料在同專案的 Zeabur PostgreSQL（`accounts/pg.js`，啟動時建表並補欄位）。
  資料表與欄位只准 `accounts/store.js` 的 `ALLOWED_COLUMNS`（INV-33 待升格，測試 `AC-AC-08`）。
- **登入**：Email 一次性連結（`/auth/email/start` → 信 → `/auth/verify` 頁 → `/auth/email/verify`），App 用 `/auth/poll` 等使用者在任何裝置點連結；
  Google／Apple 以 id_token 驗簽（`/auth/{google,apple}/native`），正式用戶端 id 尚未設定。寄信用 Resend（`RESEND_API_KEY`，尚未設定時不寄出）。
- **Mac**：主視窗「設定 › 帳號」（IPC `account_status`、`account_start_login`、`account_poll`、`account_logout`，只收主視窗）。登入後把這台 Mac
  註冊到帳號、`~/.kunsu/relay-token` 換成這台 Mac 自己的中轉站憑證、`~/.kunsu/account.json` 存 session（0600，不用鑰匙圈），並立刻重連；
  登出會從帳號移除這台 Mac，中轉站立刻斷線。相容期：中轉站仍接受 `RELAY_MAC_TOKEN`。
- **手機**：沒有登入也沒有配對時先顯示登入畫面（可改看示範資料）；已配對的手機不強迫登入。設定裡有帳號、登出、刪除帳號。
- **資料位置**：`kunsu_core::paths::resolve_vault_root`——`KUNSU_VAULT_ROOT` → `~/.kunsu/config.json` 的 `data_dir` → 作者公開前的舊資料夾 `paths::LEGACY_VAULT`（存在時才沿用）
  → `~/Kunsu`；Meta 廣告設定改在資料夾內的 `projects/meta-ads/config.json`，沒有就不發 Discord。
