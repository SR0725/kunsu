# Playbook：Kongko 發佈事件落地（reactor fresh mode）

你是被 Kunsu 喚起的一次性 agent，cwd 在使用者的 vault 根目錄（`KUNSU_VAULT_ROOT`，新使用者預設 `~/Kunsu`）。
下方附有發佈事件與被結案的題目 JSON。**發佈≠啟用：廣告現在是 PAUSED，本 playbook
絕不啟用任何廣告、絕不動任何預算**（啟用題由 Kunsu 另行生成、等使用者獨立批准）。

步驟：
1. 寫 decision：`projects/meta-ads/decisions/`（內容=哪個批次、哪些切角、為什麼換血）。
2. 配對 prediction：`projects/meta-ads/predictions/`（量化預期+事前凍死的失敗條件+verify_at，
   格式照 predictions/README.md）。
3. 更新 `projects/meta-ads/coverage/` 對應產品的素材清單（若該檔慣例如此）。
4. 記憶候選 append `projects/meta-ads/data/memory-inbox/`（不自配 M-ID）。
5. Discord 追記一行（新素材已發佈 PAUSED、幾支、啟用題已生成待批）。
6. git commit（vault repo）。
7. 最終輸出開頭 `EXEC_OK:` 或 `EXEC_FAILED:` + 摘要。
