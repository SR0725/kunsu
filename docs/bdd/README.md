# docs/bdd — 驗收規格索引

完工後由乾淨 context 的獨立 QA agent 對照本目錄逐條稽核（Kongko 開發慣例平移）。
每檔一個行為域：`## 行為規格` 是 Given/When/Then 場景，`## 驗收清單` 是可逐條打勾的 AC
（QA 通過才打勾；prefix 依域區分，跨檔引用不混淆）。標示 **Target** 的檔案是已核准但待實作
的產品目標；判斷目前程式能力時仍以 `docs/CONTRACT.md` 為準，不能把 Target 當成已完成。

| 檔案 | 行為域 | AC prefix | 涵蓋不變量 |
|---|---|---|---|
| [question-lifecycle.md](question-lifecycle.md) | 建題/驗證/upsert/狀態機/過期/superseded | AC-QL | INV-1, 2, 8, 11, 14 |
| [forcing-and-guards.md](forcing-and-guards.md) | 跨 tier 強制資格、七種情境守門、蓋板視窗、單一 queue | AC-FG | INV-3, 5 |
| [answer-flow.md](answer-flow.md) | 作答四通道、選項編輯紅線、三態回執、24h 告警 | AC-AN | INV-7 |
| [money-approval.md](money-approval.md) | 點擊當下 prepare、HMAC 簽章、單次/TTL/param_hash、red 永不一鍵 | AC-MA | INV-1, 2, 9, 10, 11 |
| [kongko-integration.md](kongko-integration.md) | kongko_review、發佈 webhook 結案、發佈≠啟用、降級、reconcile | AC-KI | INV-12 |
| [harness-roundtrip.md](harness-roundtrip.md) | ask_wait/游標收割/inbox fallback/reactor/冪等/永不重現 | AC-HR | （INV-7 見 answer-flow） |
| [survival.md](survival.md) | /health 死人開關、心跳行、doctor、rebuild-db、launchd | AC-SV | INV-6, 13 |
| [anti-fatigue.md](anti-fatigue.md) | 預算/批次/配額/品質計分/遙測降載/冷啟動隔離 | AC-AF | INV-3, 4, 14（語義面） |
| [forced-question-cycle.md](forced-question-cycle.md) | **已交付：**15 分鐘循環、3 分鐘警告、dismiss 重計時、倒數環、恢復與單一 queue | AC-FC | INV-3, 4, 5, 8 |
| [do-not-disturb.md](do-not-disturb.md) | **已交付：**最長 24 小時完全靜默、持久化恢復與 AI 讀寫控制 | AC-DND | INV-5, 7, 17 |
| [overlay-presentation.md](overlay-presentation.md) | **Target：**蓋板精準覆蓋、內容就緒握手、Esc 原生後備、來源圖示 | AC-OP | INV-22, 23 |
| [edge-notch.md](edge-notch.md) | **第一版已實作；QA 第 1 輪已修正，待第 2 輪 QA：**邊緣瀏海外觀、周長拖曳力學、角落 L 形、hover 跑馬燈、點擊穿透、隱私守門 | AC-EN | INV-24 |
| [conversation-continuity.md](conversation-continuity.md) | **已實作待 QA：**原對話身分、交付階梯、不搶寫活對話、ask_report 回饋 | AC-CC | INV-7, 13, 15, 18, 19, 25 |
| [routine-check-in.md](routine-check-in.md) | **已實作待 QA：**排程 check-in 身分、督導頁歷史與 AI 回饋、逾期未報到 | AC-RC | INV-13 |
| [decision-surface.md](decision-surface.md) | **已實作待 QA：**瀏海張開轉場、證據｜決定版型、按鈕位置、選項＋補充、回執 | AC-DS | INV-7 |
| [evidence-canvas.md](evidence-canvas.md) | **已實作待 QA：**Artifact 模式畫布、沙盒、草稿註冊與逐行診斷、revision | AC-EC | INV-7, 15, 20, 21, 22 |
| [batch-review.md](batch-review.md) | **已實作待 QA：**題組（一題多項：回信、留言、逐項核可）、一鍵全部採用／退回、改稿迴圈 | AC-BR | INV-7, 21 |
| [settings.md](settings.md) | **Target：**主視窗設定（睡眠時間、強制詢問、到期與配額、疲勞、外觀、勿擾、瀏海、Agent 連線、帳本健康、關於）、個人值疊層與範圍 | AC-ST | INV-14, 17, 26 |
| [application-shell.md](application-shell.md) | **Target：**主視窗 dub.co 設計語言（外框、版面、側欄、表格、篩選、深色、窄視窗） | AC-AS | — |

INVARIANTS.md 每條不變量均須有至少一條場景與 AC 對應；每條不變量另須有同名 Rust 測試（見
INVARIANTS.md 括號內測試名），BDD 稽核與測試綠是兩道獨立閘。

- `local-agent-plugin.md` — 本機 Codex 外掛與已選正式 icon（Current，本機安裝／QA 通過，2026-10-09）。
- `local-plugin-lifecycle.md` — 外掛完整安裝、診斷與草稿維護流程（Current，0.2.0 本機 QA 通過，2026-10-09）。
