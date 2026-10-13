# 反疲勞機制（anti-fatigue）— BDD 驗收規格

> 依據：docs/CONTRACT.md §3（批次窗）、§4（per-source 配額）、§10（policy fatigue 鍵）；
> 設計正本 §2.1（強制是稀缺資源）、§5（反疲勞機制彙總七條）。
> 不變量對應：INV-3、INV-4、INV-14（狀態機面的正式規格在 question-lifecycle.md 與
> forcing-and-guards.md，本檔驗收其反疲勞語義與遙測落盤）。
> 設計哲學：強制循環不因 dismissal 停止，但使用者的煩躁仍是上游題目品質訊號；系統要改善
> 出題品質、配額與預授權，不能用靜默停止提醒假裝解決。

## 概述

反疲勞改由出題品質、出生宣告、建題配額、品質計分、遙測降載與冷啟動隔離負責。舊 forced
budget 與 snooze 封頂已由 15 分鐘無限循環取代。所有 dismissal、kill-switch、退回重擬與過期仍
落盤為 fatigue/quality events 供週報統計。

## 行為規格

### 場景 1：所有合格題皆可強制，但上游建題有配額（INV-3）

**Given** 多題通過 actionable schema 與 per-source quota
**When** 各題到達 15 分鐘門檻
**Then** 不因 tier 或每日 forced budget 排除；全域單一 overlay queue 避免視窗散彈，上游配額與品質分防止爛題灌入

### 場景 2：單一 overlay queue 取代散彈

**Given** 一次累積 5 題跨 tier 的逾時問題
**When** 系統強制詢問
**Then** 只開一個 overlay 並 Typeform 式依序處理，不建立 5 個獨立視窗

### 場景 3：出生即宣告預設，卡片明示

**Given** 任一題進入卡片
**When** 使用者檢視卡片尾部
**Then** 一行小字宣告不作答的後果與時限（如「不答 → 48h 後：維持現狀並歸檔」），內容與 `default_action`/`deadline_at` 一致——使用者永遠知道不理它會發生什麼；配合 INV-14（pending 壽命 ≤14 天），「積壓 30 天」機制上不存在

### 場景 4：per-source 每日建題配額

**Given** policy `per_source_daily_create = 12`，某 source（同 agent/routine）今日已建 12 題
**When** 該 source 第 13 次 `ask_create`（非 upsert touch 既有題）
**Then** server 回錯誤（可讀原因：今日配額已滿，請合併題目）——把壓力逼回 agent 端合併，而不是倒給使用者；同 id upsert touch 不計入配額；隔日配額重置

### 場景 5：退回重擬計入出題品質分

**Given** 使用者對某 source 的題連續點「這題不該問我／退回重擬」
**When** 品質分累積
**Then** 每次退回落盤 fatigue/quality event（含 source、題 id、時間）；source 出題品質分下降可查——爛問題是疲勞最大來源，計分供分級器與週報使用

### 場景 6：分級準確率計分——亂標 T0 的 source 降權

**Given** 某 source 的 `requested_tier` 屢次為 t0 但被 app 守門降級
**When** 分級準確率統計累積到懲罰門檻
**Then** 該 source 之後的 t0 請求預設進 T1（降權記錄可查、可由使用者重置）；`requested_tier` 與生效 `tier` 的配對永久留存供計分

### 場景 7：kill-switch 與 dismissal 落 fatigue event

**Given** 使用者按 kill-switch（4h 免打擾）或關閉一題未回答的 overlay（INV-4）
**When** 事件發生
**Then** 各落一筆 fatigue event（type、題 id/題型、時間戳、cycle_number）；dismissal 會重啟 15 分鐘而非降級，但訊號可餵回出題品質分析

### 場景 8：疲勞遙測自動降載

**Given** policy `skip_rate_window_d = 7`、`skip_rate_downgrade = 0.5`；某題型 7 天滑動窗的略過率（snooze/退回/放到過期）超過 50%
**When** 降載判定觸發
**Then** 該題型自動降級（T0→T1 或 T1→T2）並在 Discord 心跳承認「已自動降載〈題型〉，因 7 天略過率 X%」——系統公開認錯，不無聲硬撐

### 場景 9：冷啟動遷移禁止倒灌

**Given** 既有 28 條 pending-approvals 積壓（最老 30 天）
**When** Kunsu 上線做一次性遷移
**Then** 積壓只進「一次性大掃除」pack（週末一次清完的專用視圖），永不進每日強制流；上線第一天不出現 28 題轟炸——冷啟動體驗決定 app 存活

### 場景 10：答題複利減題（standing policy 提案）

**Given** 同型題同方向被使用者回答 2 次
**When** app 偵測到重複模式
**Then** 主動提案 standing policy（T3 條件式預授權，使用者白紙黑字簽署才生效；簽署時附影響範圍模擬）；T3 生效後同型題零打擾、執行留痕 + 24h 否決窗——目標曲線第 1 週 5-6 題/日 → 第 4 週 2-3 題/日

## 驗收清單

- [ ] AC-AF-01：所有合格題可強制，舊每日 forced budget 不再排除；per-source quota 仍防灌題（INV-3）
- [ ] AC-AF-02：多題只用單一 overlay queue 連答，不建立多個視窗
- [ ] AC-AF-03：每張卡片尾部顯示與 default_action/deadline 一致的預設宣告行
- [ ] AC-AF-04：per-source 日配額 12，第 13 題回可讀錯誤；upsert touch 不計入；隔日重置
- [ ] AC-AF-05：退回重擬逐筆落 fatigue/quality event，source 品質分可查
- [ ] AC-AF-06：亂標 T0 的 source 分級降權（之後預設進 T1），requested_tier/生效 tier 配對留存
- [ ] AC-AF-07：kill-switch 與每次 dismissal 各落 fatigue event，包含 cycle_number
- [ ] AC-AF-08：7 天略過率 >50% 的題型自動降載 + Discord 承認（policy 兩鍵驅動）
- [ ] AC-AF-09：28 條積壓只進一次性大掃除 pack，每日強制流零倒灌
- [ ] AC-AF-10：同型題同方向答 2 次觸發 standing policy 提案；T3 生效後零打擾 + 留痕 + 24h 否決窗
- [ ] AC-AF-11：dismiss 無上限循環與 14 天合法壽命上限同時成立（INV-4、INV-14）
