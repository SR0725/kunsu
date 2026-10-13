# 私人助理草稿收件匣 — Current UI / 外部端到端待真人批准

Scope: batch 草稿 UI、單項 batch、plugin、部署與宿主排程。界外：無人核准的外發、平台私人 API、完整 LINE 歷史。

- AC-AD-01 Given 1–20 個合法項目 When 建題 Then 接受；0/21 拒絕。core/server tests。
- AC-AD-02 Given 多平台 subtitle When 顯示 Then 圖示、分類與件數精確且不重複計入 Meta；分類按鈕只定位、不隱藏。UI/pure tests。
- AC-AD-03 Given 草稿未送出 When 確認、撤銷確認、編輯 Then 不產生答案；修改後確認失效。inv_review_bookmark_never_authorizes_send。
- AC-AD-04 Given 未逐項確認且存在修改／略過／重擬 When 使用者全部送出 Then approve 保留最終文字，skip/revise 不帶待發文字；不自動重試未知外發。pure/server/UI mock tests。
- AC-AD-05 Given 本機修改 When 關閉重開或 AI 重送 Then 恢復使用者文字與決定，來源改變清除已確認；損壞 cache fallback，保存失敗可見。pure/UI tests。
- AC-AD-06 Given 一項重擬 When Agent 接回答案 Then 已採用照原文送出、重擬以 r2 回來；單項也 batch。plugin tests／操作文件。
- AC-AD-07 Given 已安裝版本 When 建置部署 Then cargo tests、UI tests、build-install doctor 通過；原生 UI可用，正式客戶答案不得由 QA 代點。
- AC-AD-08 Given Codex schedule When 每日 08/11/14/17/20/23/01 巡檢 Then 寫草稿入 Kunsu、不外發；02–08 不觸發；08:00 先驗證 LINE事件檔。tool readback。

## 2026-10-10 驗收證據

由實作者在隔離 Mock 做第二輪稽核，未使用獨立 QA Agent。

- AC-AD-01–05：先紅記錄 `/tmp/kunsu-assistant-red-ui.log`、`/tmp/kunsu-assistant-red-core.log`；Green UI 66 檔 851 tests、完整 cargo test 通過。來源變更清除 bookmark、保留使用者改字、skip/revise 無待發文字都由純函數測試覆蓋。
- Chrome localhost 隔離 Mock 實測：10 項顯示 5 個來源各 2；未改項顯示待送出；修改保留空白與換行；確認後再改字即取消確認；離開／重新開題保留文字與 bookmark；未逐項確認按「全部送出 9 則」得到正式 Mock 回執，另 1 項不處理。沒有接觸正式題目的送出。
- AC-AD-06：plugin 文件已更新為 1–20；plugin 測試 20 tests OK、1 skipped。真人 r2 回來並外發尚未發生，不宣稱外部端到端通過。
- AC-AD-07：`scripts/build-install.sh` 成功安裝 /Applications/Kunsu.app，doctor 8 項全綠；原生視窗以實際草稿再驗。舊 app 與 policy/playbooks/CLI 已備份；安裝腳本用 SIGTERM 正常停精確 executable，不用 AppleScript；保留過往 app 備份。
- AC-AD-08：Codex heartbeat `kunsu` 更新工具成功、設定回讀 ACTIVE，01/08/11/14/17/20/23。首次排程觸發與 LINE 自然入站驗證仍 pending，不能把設定當實際跑過。
- 既有完整 UI 測試包含 keyboard、guard、sanitizer、長內容／多輪；此次新控制使用原生 button、aria-pressed 與既有 focus，不增加 motion。實機 VoiceOver 與跨螢幕重新驗收未做。

AC-AD-07 原生補驗：01:27 安裝版 Kunsu Capsule 開啟 Overlay，正式首批 2 則顯示課程 1／Instagram 1、兩個可編輯框、可選確認與「全部送出 2 則」。僅查看，未作答；正式題維持 pending。截圖留 customer-support/.local/qa，不納入 repo。
