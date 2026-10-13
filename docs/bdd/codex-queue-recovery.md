# Codex queue recovery

- AC-CQ-01 Given resume 被 active writer 阻擋、queue 確認同 thread When reactor 接续 Then 僅排入一次、delivery reason 顯示等待，不當完成。
- AC-CQ-02 Given queue 不支援或回覆另一個 thread When fallback Then 保留 held，不假成功、不解鎖、不開新 daemon。
- AC-CQ-03 Given queue accepted 但沒有 ask_report When UI 回執 Then 等待；不顯示完成。
- AC-CQ-04 Given 先前人工答案已存在但即時交付失敗 When 客服巡檢 Then 從正式 ask_get 與本機逐項帳本補查；sent 不重寄、sending/unknown 先核對、漏 report 只補 report。

驗證：fake CLI integration + UI test + 實際 CLI queue readback；真正接續在下一輪收到測試訊息後才能確認。

## 2026-10-10 QA

- [x] AC-CQ-01：continuity integration `codex_locked_thread_queues_once_without_claiming_work_done`；完整 Rust 500 passed。
- [x] AC-CQ-02：`codex_queue_wrong_thread_is_not_accepted` 與既有 locked fallback；queue 失敗仍 held，沒有解锁或启动 daemon。
- [x] AC-CQ-03：UI 全 852 passed；localhost 隔離 mock 回執可見省略號「已排入原對話，等待 AI 接手」，沒有完成勾、長文字不溢出、Escape 可退出。mock fixture 已還原。截圖在 customer-support/.local/qa/kunsu-queue-waiting.png。未改 motion、sanitizer 或 answer guard，既有全測試通過。
- [x] AC-CQ-04：客服 scripts/recovery.py 九項測試，涵蓋中斷禁止重寄、不可透過 blocked 洗掉 unknown、遺失 report 只補 report、旧 done 缺紀錄不得重寄。真實兩項 UI 正式答案由 held 復原，平台已讀回，逐項 sent → ask_report done → picked_up → sync work=[]。
- [x] build-install.sh、doctor 8 項 PASS；plugin 0.3.1 cache 相符。
- [ ] 真實 queue 後宿主自動接續：CLI 已回報相同 thread 的排入成功，但本輪結束前尚未收到 probe；不能宣稱端到端通過。

由同一 Agent 第二輪核對，未使用獨立 QA Agent。UI 與原生部署分開驗證；舊 app 保存在 /Applications/Kunsu.app.bak，policy/playbooks/CLI 有時間戳備份。不需要資料 migration。
