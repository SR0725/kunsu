# 私人助理：先準備草稿，使用者看一眼後才送出

適用：課程客服、Meta 品牌／私人訊息與留言、Threads、LINE。範圍由當前任務授權與管道設定決定，不因本文件自行擴張帳號。

## 每輪收件

- Agent 自己讀上下文、查事實、判断是否需要回覆，直接寫一份最佳草稿；不要求使用者選版本或逐案教你回。
- 只呈現可行動且未回覆的對話。已讀不等於已回覆；同一訊息指紋不要重建待審項目。
- `kind=batch`，1–20 項，每項 `editable=true`、`default=approve`、`proposal` 為可直接送出的完整文字。預設採用只是 UI 狀態，永遠不算對外發送授權。
- `subtitle` 固定以 `課程｜`、`Facebook｜`、`Instagram｜`、`Threads｜`、`LINE｜` 開頭，後接自己帳號、接收者與公開／私訊。未知来源不用猜，列其他。這些前綴觸發原生圖示與分類件數。
- `context_md` 簡短列最新對方訊息、必要往返、來源連結、觀測時間；身份與訊息指紋保存在本機 manifest。來源不確定時不能做成可送項目。
- `why_md` 一句「已整理 N 則回覆，直接修改後全部送出即可」。不重複長篇操作說明。
- `default_action.on_expire=noop_log`，不使用 auto_policy/pick_option；不放動錢操作，退款意願的文字回覆不等於執行退款。
- LINE 通知是片段，不是完整聊天室。收不到寫 unknown/blocked，不寫「沒有未回覆」。缺少確定接收者／上下文時先查；發送能力未驗證的 LINE 案件先留本機待補證據，不混入能一鍵送出的題組。

## 使用者的操作與授權

輸入框直接修改，右側的「確認／已確認」只是可撤銷的閱讀註記。即使沒有逐項確認，使用者仍可按「全部送出 N 則」。Agent 不能代按這個按鈕、代答、從 localStorage 取草稿當授權，或把沉默、逾時、排程觸發當授權。

只有 Kunsu 正式 `ask_get` 回傳的 batch answer 是執行依據。先核對此問題屬於本任務、尚未處理，對應固定 manifest、account、recipient、原入站指紋；approve 的 `answer.items[].text` 逐字送出。送前重讀對話，使用者已回、對方有新訊息、身份不符、平台警告或發送結果 unknown 時停止該項，不盲重試。

`revise`：依 note 重擬，保留 item id，以 `<原題>-r2` 開新題，單項也 batch；舊提案不再發送。`skip` 不發送，也不擅自封鎖或刪除平台原文。每項記錄 sent/blocked/unknown，最後 ask_report。確認正式答案和平台讀回是不同證據。

## 排程

使用宿主 Codex automations，不在 plugin 重建排程引擎。排程自動收集、查證、寫草稿與建題，不能免審外發。若任務已授權恢復未完成的正式答案，每輪先補查本任務已知題目：queued/resumed/held 都不是完成證據。取得使用者正式答案後，可由即時原對話接續或下輪補查恢復；先核對逐項執行帳本與來源現況。sending/unknown 先查平台結果，不重試；sent 只補報告。客服專案使用 projects/customer-support/scripts/recovery.py sync 與 RUNBOOK 的寫前紀錄流程。建立草稿永远沒有發送副作用。休息時段不建題、不發通知；睡眠／離線缺口記 unknown，下次補掃。
