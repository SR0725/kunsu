# 本機外掛完整維護流程

狀態：**Current，0.2.0 已安裝，2026-10-09 Red → Green 與 QA 通過。證據留在本機 QA 紀錄，不進 repo。**

背景：0.1.3 已安裝且可連線，本次補完可維護的產品交付。既有 API、真人作答、強制介面與排程引擎皆界外。沿用 local-agent-plugin 的 8 條 AC。

## 場景

1. **AC-PL-01 — 安裝與冪等**（Python integration＋host）
   Given 已有其他 marketplace 項目、MCP、模型與 Kunsu，When 執行 install 兩次，Then Kunsu 只有一個 marketplace entry，source/cache/manifest 版本一致；其他設定語意不變，共用技能的額外檔案保留。只允許 kunsu 外掛本機工具，停用舊獨立 kunsu MCP，不直接編輯 cache。
2. **AC-PL-02 — 備份與失敗**（Python integration）
   Given 既有 source/config/marketplace/shared skill，When 更新，Then 先存 0700 備份資料夾、0600 設定快照，再進 mutation。宿主安裝失敗則回復本次本機檔案，保留失敗 source 與備份，不刪題目或帳本、不輸出 secrets；非 personal marketplace、無效 TOML、symlink 或來源 manifest 不一致須在 mutation 前失敗。若外部同時改寫設定則停止自動還原該檔案，保留雙方資料並報衝突。
3. **AC-PL-03 — 真實唯讀診斷**（Python unit＋host）
   Given 已安裝 app，When 執行 doctor，Then 用固定 loopback 初始化、發現完整 13 tools、呼叫 dnd_get，報版本與 PASS；不建題、不設勿擾、不作答。缺 token／app 離線回 nonzero 與下一步，stdout 不含憑證或原始上游例外。
4. **AC-PL-04 — 草稿準備**（Python integration＋live dry-run）
   Given 一個有效且尚不存在的 id，When draft 選定 decision／urgent／diary／retrospective／batch，Then 只在 ~/.kunsu/drafts/<id>/ 寫 question.md，沒有送出問題；拒絕 traversal 與覆蓋現有草稿。五個範本都能經 app dry-run，無 issue，pending count 不變；來源由真實 caller 補齊。
5. **AC-PL-05 — 操作設計**（package QA＋live preview）
   Given Agent 載入技能，When 建題或處理正式回答，Then 可找到範本、schema、故障回復與結果回報指引；緊急不等於 T0；未知不偽裝成數字、示例不當證據；Codex 交還控制權，Claude 背景等候，沉默不當同意。唯讀診斷與草稿不叫醒使用者。
6. **AC-PL-06 — 發行完整性**（package integration）
   Given 新版 source，When 打包並安裝，Then portable/compat manifest 同版、ZIP 包含 hidden manifest 與所需 scripts/templates、沒有 secrets/依賴目錄/symlinks；已選 logo 不變。紀錄安裝版本、檢查結果、未驗證真人流程及 scoped rollback。

## QA checklist

- [x] AC-PL-01
- [x] AC-PL-02
- [x] AC-PL-03
- [x] AC-PL-04
- [x] AC-PL-05
- [x] AC-PL-06

未另行授權 delegation，本輪由目前 Agent 做隔離第二輪稽核。
