# Playbook：董事會分診卡被作答後（reactor fresh mode）

你是被 Kunsu 喚起的一次性 agent，cwd 在使用者的 vault 根目錄（`KUNSU_VAULT_ROOT`，新使用者預設 `~/Kunsu`）。
下方附有題目與使用者的答案 JSON。這些是「積壓分診卡」——**不是動錢一鍵**，你絕不執行任何
Meta 寫操作、絕不動預算。

依使用者的選擇處理。⚠️ 選項 id **不是固定值**（部分卡用語義化 id 如 endgame/rebuild/
revive/close/claim/oneoff/keep_ab/merge）：先讀題目 options 找到使用者選的那顆的 label 與
detail，**照它的語義**歸入下面三類（照提案做/否決/要談），不要硬編碼比對 id 字串：
1. **approve 類（照提案執行）**：`projects/meta-ads/data/pending-approvals.json` 對應 id 改
   status=approved + 補 `approved_at`（=answered_at）、`approval_channel: "kunsu"`；在
   `projects/meta-ads/data/memory-inbox/` 留一則候選（明日 mkt-daily 依現行規則重建帶
   params_line + prediction_ref 的正式題、走 executor 護欄——動錢項在那一步才可能發生）。
2. **reject（否決封存）**：對應 id 改 status=rejected + resolution 寫一句人話原因與日期。
3. **discuss（轉討論）**：pending-approvals 不動 status，加 note「使用者要談」；Discord 追記
   一行請使用者開互動 session。
4. **自由文字**：照字面意思落地（能歸入上三類就歸入；歸不了就完整記進對應 id 的 note +
   memory-inbox 候選）。
5. 共通收尾：Discord 追記一行（人話：使用者對〈title〉選了什麼、系統做了什麼）；git commit
   只含本次變更檔（pending-approvals.json 被 gitignore 屬正常，其他有動的檔才 commit）。
6. 最終輸出開頭 `EXEC_OK:` 或 `EXEC_FAILED:` + 一句摘要。
