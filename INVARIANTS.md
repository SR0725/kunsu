# INVARIANTS — 每條配一條測試，測試名寫在括號裡

1. 錢類題（`money.is_money_op`）缺 `prediction_ref` 必被 server reject。（`inv_money_requires_prediction`）
2. 錢類題的 `default_action.on_expire` 永不允許 `pick_option`——錢不因超時自動動。（`inv_money_never_auto_expire`）
3. 每個非 internal、非終態的合格 actionable question 都有持久化 15 分鐘 force cycle；tier 與每日預算不得把它排除在 warned/forced 之外。（`inv_force_cycle_eligible`）
4. 未回答介面被關閉時必須先記 dismissal，再開始新的完整 15 分鐘週期；循環次數不限，不得因關閉次數降級、歸檔或停止 forced。（`inv_dismiss_restarts_cycle`）
5. guards 任一命中（螢幕分享/攝影機/麥克風/全螢幕/quiet hours/DND/kill-switch）時，狀態機不得產生 forced；已逾時題保持 warned/deferred，解除後立即 forced，且不得重設週期。（`inv_guards_defer_force`）
6. ledger 是 append-only：任何寫入路徑不得修改或刪除既有行。（`inv_ledger_append_only`）
7. answer 只能來自使用者本人的通道：UI IPC、已配對手機（channel=mobile，封包經裝置簽章驗證，見 28）、kongko 發佈事件、Discord 讀回、CLI isatty——AI agent 用的 MCP tools 面永不存在作答或批准寫入（AI 不能回答自己的問題）。（`inv_no_answer_via_mcp`；2026-10-12 Ray 決定拿掉封閉白名單、加入手機）
8. `ask_create` 同 id upsert 不重置倒數、不發新通知、不動 `first_proposed`。（`inv_upsert_no_reset`）
9. approval record 單次消耗：`/approvals/verify` 第二次對同 record 必回 `valid=false`；已有結果（`money_outcomes`，例如已記成 `EXEC_BLOCKED`）或被這題更新的簽發 nonce 取代的授權一律 `valid=false`（與記結果在同一段寫入鎖內判斷）。（`inv_approval_single_use`）
10. approval TTL：`answered_at` 起 15 分鐘後 verify 必拒。（`inv_approval_ttl`）
11. `gate=red` 的題必為多選項（choice），且永不進一鍵執行通道。（`inv_red_needs_options`）
12. 發佈事件（channel=kongko）只結案素材類題，隨後必生成獨立啟用題——發佈永不等於啟用。（`inv_publish_not_activate`）
13. SQLite 可整顆刪除後由 ledger + OPEN.json 完整重建（`rebuild-db`），含每個答案的交付狀態；重建永不讓已交付的答案再交付一次，也不遺漏還沒交付的答案；帳本寫不進去的變更排隊補寫，積欠未清時拒絕重建；重建先完整讀完帳本才清資料庫，讀不到、或資料庫裡有非 internal 題目在帳本與 OPEN.json 都找不到時，資料庫保持原樣。（`inv_rebuild_from_ledger`）
14. 任何題 pending 壽命 ≤ 14 天（T2 上限）；到期必轉終態。（`inv_max_pending_age`）
15. 錢類題（`money.is_money_op`）只能由 channel=ui（Tauri IPC、使用者物理點擊）或 channel=mobile（已配對手機、核對畫面按批准）作答並觸發批准簽章鏈；`/cli/answer`、`/ingest/discord-answer`、kongko、auto_policy 等其他通道對錢題一律拒絕作答（400＋可讀訊息「錢題只能在 Kunsu app 或已配對的手機批准」）。（`inv_money_chain_ui_only`）
16. edit_option / edited_params 改動金額時，`money.executor_prepare_cmd` 的數值引數必須同步改寫（只認 prepare-budget `--new-budget N` / prepare-bid `--new-value N` 兩型、整數驗證）；認不得或改寫失敗必須升 red 鎖一鍵（gate=red 持久化＋`tier_downgrade_reason` 說明）。（`inv_edited_params_rewrite_prepare_cmd`）
17. Kunsu 勿擾生效期間不得顯示系統通知、浮動膠囊或強制 overlay；設定／解除不得回答、取消、重設任何題目的 cycle timestamp 或增加 cycle_number，且單次不得超過 24 小時。（`inv_dnd_is_silent_and_bounded`）
18. 同一答案只有一條交付路徑生效：交付狀態離開 pending／held 後永不改變（唯一例外是 reactor 佔住交付權的過渡狀態 resuming，只能由它自己轉成 resumed／held／fresh／dead）；等待者取走、原對話追加、原對話自行回報或開新對話，副作用恰好一次——agent 已開始處理或已回報後，非零結束與逾時都不重試、不開新對話。（`inv_single_continuation`）
19. 永不對仍被活行程持有的原對話做背景追加（Claude 登記檔命中且 pid 存活、Codex thread 被鎖）；只能 held 到釋放後再接續。（`inv_never_resume_live_conversation`）
20. 證據畫布只在 `sandbox="allow-scripts"`（無同源）iframe 中執行，CSP 禁止一切網路、表單、外部子框架與上層導覽（srcdoc 子框架繼承同一份 CSP）；CSP 管不到的 WebRTC 與資源提示（`<link rel=preconnect／dns-prefetch／preload／prefetch>`）在所有 Kunsu webview 的引擎設定關閉——每個 webview 都經同一個建立路徑，建立後立刻套用整份關閉清單；畫布框（任何子框架）只能停在 `about:srcdoc`、`about:blank`（含同文件片段）與 `blob:`，不能被導到 Kunsu 自己的頁面、`data:` 或任何網址——殼層依 WebKit 回報的目標框架判斷，同一個建立路徑套用；Kunsu 只接受同一次渲染 token 的 `resize`／`ready`／`error` 訊息，其餘一律忽略。（`inv_canvas_sandboxed`）
21. 決策控制永遠原生：題目、選項、送出與錢參數只來自草稿 frontmatter／建題欄位並由 Kunsu 原生介面呈現；畫布內容不能產生答案、批准或改變參數，註冊草稿永不產生答案。（`inv_controls_native`）
22. 強制蓋板永遠不會以空白或半渲染的樣子出現：只有網頁確認「這一次轉場的題目、輸入與關閉都已渲染」（同一 id 的 mounted）才帶到前景；就緒逾時改開主視窗，不留模糊空殼。（`inv_overlay_never_blank`）
23. 強制蓋板顯示時完整覆蓋目標螢幕（膠囊所在螢幕 → 游標螢幕 → 主螢幕）：frame 與該螢幕完全一致（誤差 ≤0.5 pt），不得上緣超出或下方懸空。（`inv_overlay_covers_target_display`）
24. 邊緣瀏海以外完全點擊穿透：命中區＝實際畫出的黑色形狀（同一組多邊形，不加容差）；沒有畫出瀏海時（沒有待決題、只剩 internal 或終態題、幾何還沒算好）命中區為空；游標不在命中區（含 L 形內側與選單列）時舞台視窗一律 ignoresMouseEvents；唯一例外是在瀏海上按下滑鼠後滑鼠鍵還沒放開（含拖曳中）時暫時接管，但原生層一偵測到滑鼠鍵全部放開就立即結束接管，不依賴網頁收到 pointerup；命中區被清空（瀏海不再畫出，例如拖曳中所有題目在別處結案）時也立即結束接管，網頁同時清掉自己的拖曳狀態，之後瀏海再出現時不帶著舊拖曳接管；拖曳或按著時舞台被隱藏（勿擾、⌘⇧K）也一樣：原生層解除接管並通知網頁（`edge_drag_end`），網頁照沒畫出瀏海清掉拖曳狀態。（`inv_edge_notch_click_through`，UI 與 shell 各一條同名測試）
25. agent 讀取面（`ask_get`／`ask_list`／`ask_wait`，以及 `ask_create`／`ask_register` 重送已答題的回應）永不包含 UI 簽章或錢鏈原始輸出（approval 檔路徑、executor token、commit 命令）——不論 agent 是否已 `ask_report`；只有 Lane 1 的 `ask_answers_since` 保留原貌。（`inv_agent_reads_never_leak_money_secrets`）
26. 設定頁只改 `~/.kunsu/policy.local.toml` 與每台 Mac 的呈現偏好：永不回答、取消或重設任何題目的 cycle timestamp，也不增加 cycle_number；到期參數（一般題到期時間、題目最長等待天數）在建題時固定在題目上，之後改設定只影響新題，已在等的題不會因為設定改動而提早到期或被依預設作答；寫入的值一律在 CONTRACT §15 的範圍內（`expiry.t2_supersede_after_d ≤ 14` 等），不合法就整筆拒絕、檔案不變；錢的護欄、紅燈規則與其他不變量不能由設定放寬。（`inv_settings_never_touch_questions`）

27. 私人助理的「確認／已確認」只更新本機註記；預設採用與草稿快取不產生答案。正式送出不要求逐項確認，且採用項目傳送的文字必須與目前編輯框逐字相同，重擬／略過項目不帶待發文字。（`inv_review_bookmark_never_authorizes_send`）
28. 手機通道只接受已配對、未撤銷裝置的封包：裝置不在清單或已撤銷、裝置簽章錯、`seq` 不大於上一個、時間差超過 120 秒、解密失敗，一律拒絕且不產生任何答案。（`inv_mobile_envelope_verified`）
29. 配對碼 5 分鐘過期、只能用一次；正確的配對請求只進入「待允許」，使用者在主視窗按允許才寫入裝置清單。（`inv_pairing_requires_mac_confirm`）
30. 中轉站只轉送：手機與 Mac 之間的內容一律端對端加密，中轉站不持有任何解密金鑰、不把封包寫進硬碟。（`inv_relay_sees_only_ciphertext`，中轉站測試）
31. 追問回覆只是文字：`ask_followup_reply` 與 `kunsu followup-reply` 永不建立答案、不改題目狀態、倒數、cycle 或交付狀態。（`inv_followup_reply_never_answers`）
32. 中轉 AI 沒有作答或動手的路徑：它的子行程一律不載入任何 MCP（Claude `--strict-mcp-config` 空設定；Codex 唯讀沙盒），只准讀檔工具，不能寫檔或跑指令；它的輸出只會變成給手機看的文字，永不作為答案、批准或任務指令。（`inv_relay_ai_read_only`）
