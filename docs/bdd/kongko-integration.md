# Kongko 整合（kongko-integration）— BDD 驗收規格

> 依據：docs/CONTRACT.md §5（POST /events/kongko）；設計正本 §2.2（發佈≠啟用）、§9（三條窄接面）。
> 不變量對應：INV-12。
> 界外：啟用題被批准後的錢流程見 money-approval.md；reactor playbook 執行見 harness-roundtrip.md。
> Kongko 端 `metaPublish.ts` webhook 發送與 Remote MCP 查詢由 integration agent 實作；
> 本檔以 Kunsu 收端行為 + 端到端旅程為驗收面。

## 概述

整合是三條窄接面：deep link、publish webhook、Remote MCP reconcile——不做深度 API 綁定。
鐵律：**發佈=同意素材上架，啟用=同意花錢，必須是兩次物理點擊**。Kongko 發佈一律建 PAUSED；
Kunsu 收到發佈事件結案素材題後，自動生成獨立的 T0 啟用題。Kongko 死掉時 Kunsu 全部非 Kongko
功能照常——降級是驗收條件。

## 行為規格

### 場景 1：kongko_review 題型建立

**Given** agent（如 mkt-daily）已在 Kongko 經 `atelier_batch_create / angles_propose` 點火一條產線
**When** 呼叫 `ask_create` 帶 `kind: "kongko_review"`、`kongko_ref: <batchId>`、`deep_link: <Kongko 批次工作台 URL>`
**Then** 題目正常入庫；卡片顯示「開 Kongko」動作（素材題才有）；kongko_review 題不含 pick/kill 選項——挑圖/殺稿永遠去 Kongko 本體，蓋板不重畫畫布

### 場景 2：deep link 開啟 Kongko

**Given** 一題 kongko_review 顯示在卡片或蓋板
**When** 使用者點「在 Kongko 開啟」
**Then** 以預設瀏覽器開 `http://localhost:3200/<批次工作台>` deep link；Kunsu 不嘗試在蓋板內嵌畫布

### 場景 3：發佈 webhook 結案素材題（INV-12）

**Given** 一題 pending 的 kongko_review 帶 `kongko_ref == "batch-123"`
**When** Kunsu 收到 `POST /events/kongko`（Bearer）payload `{type:"ad_publication_confirmed", publicationId, batchId:"batch-123", product, ads[], confirmedAt}`
**Then** 該題轉 `answered`、`channel = "kongko"`——發佈即同意（素材層），使用者不被同一件事問兩次；append ledger；喚 reactor 跑「發佈落地 playbook」（寫 decision + 配對 prediction、更新 coverage、記憶候選進 inbox、Discord 追記）

### 場景 4：發佈≠啟用——自動生成獨立啟用題（INV-12）

**Given** 場景 3 的素材題剛被發佈事件結案
**When** 結案處理完成
**Then** Kunsu 自動建一題**獨立的** T0 啟用題：參數從 AdPublication 預填（campaign/adset/日預算）、`money.is_money_op = true`、帶 `params_line` 與 `prediction_ref`、`gate` 依金額判定；使用者一點才開始花錢——每一次不可逆動作對應恰好一次人類點擊；啟用題走完整錢的授權鏈（money-approval.md）

### 場景 5：發佈事件只結案素材類題

**Given** 系統同時存在：kongko_ref 相符的素材題、kongko_ref 不同的其他素材題、同產品的動錢題與策略題
**When** 發佈事件抵達
**Then** 只有 `kongko_ref == batchId` 的題被結案；其他任何題（含同產品錢題）狀態不受影響；無任何題匹配時事件仍落 ledger 留痕（供 reconcile 對帳），不報錯、不誤結

### 場景 6：webhook 認證與壞 payload

**Given** `POST /events/kongko` 抵達
**When** (a) 缺 Bearer token 或 token 錯誤；(b) payload 非法（缺 type/batchId、未知 type）
**Then** (a) 回 401 不處理；(b) 回 400 帶可讀原因，不結案任何題、不 crash；合法但重複的事件（同 publicationId 重送）冪等處理——已 answered 的題不重複結案、不重複生成啟用題

### 場景 7：Kongko 死掉降級不壞

**Given** Kongko 進程死掉或 `localhost:3200` 404
**When** Kunsu 照常運作一整天（上線前實測拔掉 Kongko 跑一天）
**Then** 所有非 Kongko 功能照常：建題/強制/作答/錢鏈/ledger/reactor 全部不受影響；kongko_review 相關題只少畫布連結並標「Kongko 失聯」；deep link 點擊失敗顯示人話錯誤不 crash

### 場景 8：reconcile 補漏（webhook 打在 app 死掉的瞬間）

**Given** 一次 Kongko 發佈的 webhook 因 Kunsu 死掉而漏接（Kongko 端 fire-and-forget、`KUNSU_WEBHOOK_URL` 未設則靜默跳過）
**When** 每晚排程或 mkt-daily startup 的 reconcile 跑起：經 Kongko Remote MCP 查最近 AdPublication 清單 diff ledger
**Then** 漏掉的發佈被補結案（channel=kongko、事件標 reconcile 來源）、補生成啟用題；已入 ledger 的發佈不重複處理（以 publicationId 冪等）

### 場景 9：切角前檢查（agent 側 SOP）

**Given** mkt-daily 或 ads 相關 skill 判定要生成新切角
**When** 建 kongko_review 題之前
**Then** SOP 硬性兩步：(1) `curl 127.0.0.1:4777/health` 確認 Kunsu 活著（死了走檔案 inbox + Discord 降級）；(2) 經 Kongko MCP 查該產品是否已有進行中 batch（避免重複點火產線）；identifier 全部沿用 `atelier_*` 正名

## 驗收清單

- [ ] AC-KI-01：`kind=kongko_review` + `kongko_ref` + `deep_link` 建題入庫，卡片有「開 Kongko」動作
- [ ] AC-KI-02：deep link 以瀏覽器開啟工作台 URL，蓋板不內嵌畫布、無 pick/kill 選項
- [ ] AC-KI-03：發佈事件結案 `kongko_ref==batchId` 的題，channel=kongko + append ledger + 喚發佈落地 playbook（INV-12，測試 `inv_publish_not_activate`）
- [ ] AC-KI-04：結案後自動生成獨立 T0 啟用題：AdPublication 參數預填、is_money_op、params_line、prediction_ref 齊備
- [ ] AC-KI-05：啟用題在使用者點擊前 Meta 端廣告維持 PAUSED，零花費
- [ ] AC-KI-06：發佈事件不誤結任何 kongko_ref 不符的題；無匹配時留痕不報錯
- [ ] AC-KI-07：webhook 無/錯 Bearer 回 401；壞 payload 回 400 可讀原因；同 publicationId 重送冪等
- [ ] AC-KI-08：拔掉 Kongko 跑一天：非 Kongko 功能全部照常，相關題標「Kongko 失聯」，無 crash（降級驗收）
- [ ] AC-KI-09：reconcile（每晚 + mkt-daily startup）補漏結案與補啟用題，以 publicationId 冪等不重複
- [ ] AC-KI-10：切角建題 SOP 含 health 檢查 + 進行中 batch 檢查，寫進 mkt-daily 兩份 SKILL.md
- [ ] AC-KI-11：Kongko 端 webhook 為 fire-and-forget、env 未設靜默跳過——Kongko 零耦合可獨立存活
