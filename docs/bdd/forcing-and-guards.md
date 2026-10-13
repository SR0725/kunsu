# 強制與情境守門（forcing-and-guards）— BDD 驗收規格

> 依據：`docs/CONTRACT.md` §3、§9、§10；`docs/PRD.md` §5–6。
> 不變量對應：INV-3、INV-5。
> 界外：dismiss 重啟與時間恢復細節見 `forced-question-cycle.md`。

## 概述

所有合格 actionable question 都會在 15 分鐘內進入強制詢問循環；tier 與舊每日預算不再排除
任何題。情境守門只攔下 forced overlay——**無例外、非可調參數**；warning 照常發生，逾時題
保持 deferred、永不取消。一次錯誤時刻的引爆足以破壞信任，所以分享螢幕、錄影、會議、
全螢幕展示、quiet hours、DND 與 kill switch 永遠優先。

Guards：`{ screen_shared, camera_on, mic_on, frontmost_fullscreen, quiet_hours, dnd,
kill_switch_until }`。

## 行為規格

### 場景 1：所有合格題都進 force cycle（INV-3）

**Given** 一題非 internal、非終態、通過 actionable schema
**When** core 建立週期
**Then** T0/T1/T2/T3 都保存 12/15 分鐘時間戳；舊每日 forced budget 不參與 admission；只有
forced 顯示當下仍必須 guards 全綠

### 場景 2：舊每日預算不得阻止強制詢問

**Given** 今日已有任意數量問題走過 forced
**When** 另一題到達本輪 `warn_at` / `force_at`
**Then** 仍正常 warned，guards 全綠時仍可 forced；health 以 `overdue_count` 表達積壓，不再提供
forced budget

### 場景 3：螢幕分享／錄影期間永不蓋板（INV-5）

**Given** `screen_shared = true`
**When** 任一題到達 warning 或 force 門檻
**Then** 可以進 warned 並在 app/capsule 顯示倒數，但不蓋板、不公開題目內容；到期保持
deferred，分享結束後立即恢復 forced

偵測限制：`screen_shared` 只由 `CGDisplayIsCaptured`（獨佔擷取）判定，一般螢幕錄影與 ScreenCaptureKit 分享偵測不到
（macOS 沒有公開 API；評估見 `edge-notch.md` 場景 12 已知限制）。這類情況要靠攝影機／麥克風／全螢幕 guard 或勿擾。

### 場景 4：攝影機或麥克風佔用期間永不蓋板（INV-5）

**Given** `camera_on = true` 或 `mic_on = true`
**When** 任一題到達 force 門檻
**Then** 可 warned、不得 forced、順延不取消；解除後已逾時題立即恢復

### 場景 5：前景全螢幕 app 期間永不蓋板（INV-5）

**Given** `frontmost_fullscreen = true`
**When** 任一題到達 force 門檻
**Then** 不打斷展示，週期時間戳保持不動；離開全螢幕後已逾時題立即恢復

### 場景 6：quiet hours 期間永不蓋板（INV-5）

**Given** 本機時間落在 policy quiet hours（預設 23:00–09:00，跨午夜正確處理）
**When** 任一題到達 force 門檻
**Then** 題目保持 warned/deferred；時窗結束後立即恢復，不重新贈送 15 分鐘

### 場景 7：macOS Focus／勿擾期間永不蓋板（INV-5）

**Given** `dnd = true`
**When** 任一題到達 force 門檻
**Then** 同場景 6

### 場景 8：kill switch 四小時免打擾（INV-5）

**Given** 使用者啟用 kill switch
**When** 四小時內任一題到達 force 門檻
**Then** 可 warned 但不得 forced；kill switch 落 fatigue event；到期後仍有效的逾時題立即恢復

### 場景 9：守門解除後立即恢復

**Given** 一題因 guard 留在 warned/deferred，`force_at` 已過
**When** 最後一個 guard 解除
**Then** 下一輪 timer 立即進 forced；不重設 cycle timestamps、不增加 cycle_number

### 場景 10：蓋板視窗行為安全

**Given** overlay 被觸發
**When** 視窗顯示
**Then** 它是獨立 WebviewWindow、尺寸等於螢幕 frame、`NSWindow.level = .popUpMenu`、非 macOS
fullscreen mode；不切 Space、不蓋系統對話框，並維持半透明黑色與 blur

### 場景 11：多題共用單一 overlay queue

**Given** 多題跨 tier 同時逾時且 guards 全綠
**When** timer 處理它們
**Then** shell 只顯示一個 overlay window，依最早逾時、風險、建立時間穩定排序並在同一 flow 連答

### 場景 12：policy.toml 熱載

**Given** `~/.kunsu/policy.toml` 被修改
**When** app watcher 偵測到變更
**Then** 新 policy 用於之後新建／dismiss 的週期；已在倒數中的題保留既有 persisted timestamps，
不追溯重算

## 驗收清單

- [ ] AC-FG-01：所有合格跨 tier 題都進 15 分鐘循環，不受舊每日預算排除（INV-3，測試 `inv_force_cycle_eligible`）
- [ ] AC-FG-02：`/health.overdue_count` 正確，不再暴露 forced budget
- [ ] AC-FG-03：`screen_shared` 命中時可 warned、不可 forced（INV-5，測試 `inv_guards_defer_force`）
- [ ] AC-FG-04：`camera_on` / `mic_on` 命中時可 warned、不可 forced
- [ ] AC-FG-05：`frontmost_fullscreen` 命中時可 warned、不可 forced
- [ ] AC-FG-06：`quiet_hours`（含跨午夜）命中時可 warned、不可 forced
- [ ] AC-FG-07：`dnd` 命中時可 warned、不可 forced
- [ ] AC-FG-08：kill switch 生效期間不可 forced，動作落 fatigue event，到期後逾時題恢復
- [ ] AC-FG-09：guard 命中一律 deferred；解除後立即恢復且不重設 cycle/increment cycle_number
- [ ] AC-FG-10：overlay level=.popUpMenu、非 fullscreen mode、不蓋系統對話框、不切 Space
- [ ] AC-FG-11：多題跨 tier 逾時仍只有一個 overlay queue，排序穩定
- [ ] AC-FG-12：policy 熱載不追溯改寫既有 cycle timestamps
- [ ] AC-FG-13：guards 是狀態機純函數輸入，core 測試逐一枚舉七種 guard
