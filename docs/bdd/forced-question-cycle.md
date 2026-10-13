# 15 分鐘強制詢問循環（forced-question-cycle）— BDD 驗收規格

> 狀態：**Delivered / 已交付（2026-08-05）**
> 產品依據：`docs/PRD.md` §5–6、§15–17；視覺依據：`DESIGN.md`。
> 驗證：`cargo test --workspace`、`pnpm test`、`pnpm build`、`pnpm lint` 與
> `scripts/build-install.sh` doctor 全綠；另以 Playwright headed browser 驗收 capsule、overlay、
> dismissal 與無待決時隱藏。

## 概述

每個合格的 actionable AI question 都會進入一個持久化的 15 分鐘處理週期。剩餘 3 分鐘時警告，
到期且 guards 全綠時開啟全螢幕強制詢問。使用者若關閉但未回答，立即開始新的完整 15 分鐘
週期，無 dismissal 次數上限，直到 answer、cancel、supersede 或 valid expiry。

本規格把「強制詢問」定義為可靠地重新呈現問題，不宣稱能物理強迫使用者在 15 分鐘內作答。

## 行為規格

### 場景 1：建題立即開始第一輪 15 分鐘週期

**Given** 一個通過驗證、仍有效且會阻塞 Agent 的 actionable question 首次建立
**When** core 持久化該題
**Then** 同一 transaction 保存 `cycle_started_at = now`、`warn_at = now + 12m`、
`force_at = now + 15m`、`cycle_number = 1`；UI 顯示待決膠囊，不能等待 UI 開啟後才開始計時；
不得因舊 tier 或每日 forced budget 將合格問題排除在循環外

### 場景 2：圓環由 wall-clock 單調縮減

**Given** 一題處於有效週期，左側 Agent 圖像可見
**When** wall clock 從 `cycle_started_at` 前進至 `force_at`
**Then** 圓環從 100% 單調縮減到 0%，不旋轉、不造成圖像或文字 layout shift；進度由持久化
timestamp 推導，render pause／切題／視窗重開不能重設

### 場景 3：剩餘三分鐘進入警告

**Given** 一題未回答且本輪尚未進入 warning
**When** wall clock 到達 `warn_at`（剩餘時間小於等於 3 分鐘）
**Then** 右側穩定顯示「3 分鐘後將開啟強制詢問」及可選的實際剩餘時間；不閃爍、不推動
版面；screen reader 只在跨入門檻時公告一次

### 場景 4：到期且 guards 全綠時強制詢問

**Given** 一題未回答、wall clock 已到 `force_at`，且所有 guards 為綠色
**When** 下一次狀態檢查執行
**Then** app 只建立／顯示一個 overlay，聚焦該題；force event 冪等落盤；顯示 overlay 不得被記為
answer，也不得自動觸發 Agent 執行

### 場景 5：關閉未回答 overlay 會開始新週期

**Given** overlay 正顯示一題且尚無正式 answer receipt
**When** 使用者按關閉、Esc 或其他合法 dismissal 動作
**Then** dismissal event 先落盤；該題保持未解決；立即保存新的 `cycle_started_at = now`、
`warn_at = now + 12m`、`force_at = now + 15m`、`cycle_number += 1`；overlay 關閉、膠囊保留、
圓環回到新一輪 100%

### 場景 6：未回答循環沒有 dismissal 次數上限

**Given** 同一題已被關閉兩次、十次或更多次，且仍有效、仍未回答
**When** 每輪 15 分鐘到期且 guards 全綠
**Then** 每次都再次開啟強制詢問；不能因 dismissal 次數自動降級、歸檔、視為略過或停止循環

### 場景 7：正式回答立即停止後續循環

**Given** 一題處於任一 pending／warning／forced／deferred 週期
**When** 使用者透過合法通道送出回答，且 app 成功持久化 answer receipt
**Then** 題目進入 answered terminal state；取消所有排定的 warning／force；隱藏該題倒數；任何
延遲 timer、重試或舊 UI event 都不能再次開啟它

### 場景 8：其他合法 terminal condition 停止循環

**Given** 一題仍在週期中
**When** 原始 Agent 合法 cancel、較新版本 supersede，或題目到達符合 policy 的 valid expiry
**Then** 題目進入對應 terminal state，並停止後續 warning／force；極短 expiry 或未授權來源不能用來
繞過循環

### 場景 9：guards 命中時逾時但不顯示 overlay

**Given** 一題到達 `force_at`，但 screen share、錄影、會議、前景全螢幕、quiet hours、DND 或
kill switch 任一 guard 命中
**When** 狀態檢查執行
**Then** 不顯示 overlay、不公開題目內容、不開始新週期；題目保持 overdue/deferred；瀏海說出是哪個 guard 擋住、
什麼時候會跳出來（例如「時間到了，分享螢幕結束就跳出來」，對照見 edge-notch 場景 10c）

### 場景 10：guards 解除後立即恢復逾時問題

**Given** 一題因 guard 被 deferred，原 `force_at` 已經過去且題目仍有效未回答
**When** 最後一個 guard 解除
**Then** 下一次狀態檢查立即顯示 overlay，不重新贈送 15 分鐘、不增加 `cycle_number`；只有使用者
實際關閉該 overlay 才開始新週期

### 場景 11：restart 與 sleep/wake 保留 wall-clock 語意

**Given** 一題已有持久化週期時間戳
**When** app restart、macOS sleep 後喚醒或 renderer crash/reload
**Then** 從 timestamp 還原剩餘時間；若 wall clock 已逾 `force_at`，依 guards 立即 force 或 deferred；
不得以重啟／喚醒時間當成新的 `cycle_started_at`

### 場景 12：同 id upsert 不重設週期

**Given** 一題已處於第 N 輪且仍未回答
**When** 來源因重試或內容 touch 以相同 `question_id` upsert
**Then** 保留既有 `cycle_started_at`、`force_at`、`cycle_number` 與事件歷程；不產生第二題，也不
取得新的 15 分鐘

### 場景 13：多題獨立倒數、單一 overlay queue

**Given** 多題各自在倒數或已逾時
**When** 兩題以上同時具備 force 資格
**Then** 全域最多一個 overlay；預設選最早逾時者，同時逾時依風險與建立時間穩定排序；切題不
重設任何一題倒數；回答後可前往下一個已逾時問題

### 場景 14：reduced motion 仍保留完整時間語意

**Given** 系統啟用 `prefers-reduced-motion`
**When** 倒數、warning 或 capsule-to-overlay 狀態變更
**Then** 移除非必要的粒子移動與空間轉場，但保留靜態圓環進度、剩餘時間、warning、焦點與可操作
介面；功能與時序不改變

### 場景 15：倒數與警告不改變膠囊幾何

> 2026-10-08：邊緣瀏海（`edge-notch.md`，Target）交付後，本場景與 AC-FC-15 改由 AC-EN-02／10／11 驗收；
> 交付前仍適用於現行膠囊。

**Given** 膠囊位於螢幕任何邊界位置
**When** 它進入 warning、被 hover 或倒數刷新
**Then** 膠囊維持固定 44 px 高、不出現 tooltip、不飛離邊界、不因文字加入而位移；拖曳與點擊
命中區保持穩定

### 場景 16：強制顯示不能自動代答或執行

**Given** 題目到期並顯示 overlay
**When** 使用者尚未明確送出參考回答或自訂文字
**Then** 系統不得選取預設答案、送出回答、觸發 money/action prepare 或宣稱 Agent 已接手；參考回答
只供選擇

### 場景 17：系統時間異常不產生負進度或重複 force

**Given** 一題處於有效週期
**When** 系統 wall clock 大幅向前或向後調整
**Then** 圓環 clamp 在 0–100%，原始 timestamps 保持可查；同一 cycle 的 force event 保持冪等，
不因時間跳動反覆開啟多個 overlay，也不靜默改寫 `cycle_started_at`

## 驗收清單

- [x] AC-FC-01：所有合格 actionable question 的建題 transaction 同步寫入 15m/12m 時間戳與
  `cycle_number = 1`，不受舊 tier／每日 forced budget 排除
- [x] AC-FC-02：圓環由持久化 wall-clock timestamp 算出，100%→0% 單調且無 layout shift
- [x] AC-FC-03：剩餘 3 分鐘顯示穩定警告，screen reader 只公告一次
- [x] AC-FC-04：到期且 guards 全綠時只顯示一個 overlay，force event 冪等
- [x] AC-FC-05：dismiss 未回答 overlay 先落事件，再開始完整新 15m，`cycle_number + 1`
- [x] AC-FC-06：dismissal 次數不限；不因次數降級、歸檔或停止 force
- [x] AC-FC-07：answer receipt 持久化後取消所有後續 warning/force，舊 timer 不可復活
- [x] AC-FC-08：cancel/supersede/valid expiry 終止循環，未授權或極短 expiry 不可繞過
- [x] AC-FC-09：任一 guard 命中時只 deferred、不顯示 overlay、不重設週期
- [x] AC-FC-10：guards 全部解除後，已逾時題立即恢復，不贈送新 15m
- [x] AC-FC-11：restart、sleep/wake、renderer reload 都從時間戳正確還原
- [x] AC-FC-12：同 id upsert 不重設時間或 cycle，也不建立重複題
- [x] AC-FC-13：多題獨立倒數、全域單一 overlay，排序穩定且切題不重設
- [x] AC-FC-14：reduced motion 保留完整時間／狀態／操作語意
- [x] AC-FC-15：hover、warning、倒數刷新不產生 tooltip 或改變 44 px 膠囊幾何
- [x] AC-FC-16：force 只顯示問題，絕不自動選答、送出或執行
- [x] AC-FC-17：wall-clock 異常時進度 clamp、timestamps 可查且 force 保持冪等

## 已完成的測試映射

- Core unit tests：注入 fake clock，覆蓋 0m、11:59、12m、14:59、15m、跨多輪 dismissal。
- Core/UI boundary tests：任意時間推進下進度都在 0–100%，terminal 後永不回到 forceable。
- Persistence tests：restart、crash boundary、同 id upsert、answer/dismiss transaction ordering。
- Guard matrix：逐一及組合枚舉七種 guard，驗證 deferred 與解除恢復。
- Queue tests：多題同時逾時、穩定排序、單一 overlay ownership。
- UI tests：圓環、warning、無 tooltip、44 px 幾何、keyboard、screen reader、reduced motion。
- End-to-end：Agent 建題 → 15m → force → dismiss → 15m → answer → receipt → 永不再現。

## 功能界外

- 不決定未來是否提供不同 snooze 時長；任何新增都必須另行變更 PRD/BDD。
- 不移除既有 guards、安全簽章、回答 receipt 或內容 sanitizer。
