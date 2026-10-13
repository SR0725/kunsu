# 存活工程（survival）— BDD 驗收規格

> 依據：docs/CONTRACT.md §5（/health）、§7（Ledger 與 rebuild-db）、§0（launchd）、§11（doctor）；
> 設計正本 §11（無聲死亡家族傳統的對策）、§12（mkt-daily 改造）。
> 不變量對應：INV-6、INV-13。
> 界外：app 死掉時的建題降級（檔案 inbox）見 harness-roundtrip.md 場景 5。

## 概述

死人開關反向設計：不靠 app 自己報活，靠已存活的排程（mkt-daily）驗屍，且心跳報到使用者實際會看
的面（Discord 日報），不是又一個要記得看的儀表板。SQLite 是可重建的運行索引、vault ledger 是
真相——app 整顆刪掉，稽核軌跡與未消化答案必須完好無損。launchd 絕不指 repo build 路徑
（task-engine zombie plist 的直接死因）。

## 行為規格

### 場景 1：/health 無 auth 可達

**Given** Kunsu app 正在運行
**When** 本機任何進程 `GET http://127.0.0.1:4777/health`（不帶 token）
**Then** 回 `{version, uptime_s, pending_count, overdue_count, last_answer_at, oldest_pending_days}`；bind 僅 loopback，外部網路不可達

### 場景 2：mkt-daily 死人開關——死了要大聲

**Given** mkt-daily routine 每日 09:33 起跑
**When** Step 0 檢查 `/health` 失敗（連線拒絕），或成功但 `last_answer_at`/heartbeat 逾 24h
**Then** 當日 Discord 日報**頭條紅字**「⚠️ Kunsu 已死 N 天，黃燈正在積壓」；N 由 ledger 最後活動時間推算——app 無聲死亡最晚隔天 09:33 被驗屍

### 場景 3：mkt-daily 心跳行——活著也要留痕

**Given** `/health` 檢查通過
**When** 當日日報產出
**Then** 日報尾固定附一行人話心跳：「Kunsu ✅｜待決 3｜最老 1 天」（pending_count 與最老 pending 天數來自 health/OPEN.json）

### 場景 4：ledger append-only（INV-6）

**Given** `logs/asks/YYYY-MM-DD.jsonl` 已有歷史行
**When** 任何寫入路徑運作（終態 append、approval 消耗記錄、reconcile 補漏）
**Then** 只在檔尾 append 新行，既有行的位元組永不被修改或刪除；`OPEN.json` 是唯一允許重寫的檔（pending 快照，每次變更整檔重寫）；ledger 檔案 git 追蹤、絕不 gitignore

### 場景 5：rebuild-db 從 ledger 完整重建（INV-13）

**Given** ledger（`logs/asks/*.jsonl`）與 `OPEN.json` 完整，SQLite 整顆被刪除
**When** 跑 `kunsu rebuild-db`
**Then** SQLite 完整重建：pending 題全數恢復（含 state/snooze_count/first_proposed/next_transition_at 可推算值）、終態題稽核鏈完整可查、approval 消耗記錄恢復（單次性判定不因重建失效）；consumer 游標若重置，後果只是 at-least-once 重放，被消化端冪等吸收（harness-roundtrip.md 場景 10）

### 場景 6：doctor 冒煙全流程

**Given** 已安裝的 Kunsu（/Applications）
**When** 跑 `kunsu doctor`
**Then** 依序驗證並全綠才 exit 0：(1) 雙 CLI MCP 可達（Claude `claude mcp` 與 Codex config 掛載檢查）；(2) `/health` OK；(3) 自動建一條測試題 → 模擬作答 → 驗證 ledger append 與 reactor dry-run 消化 → 清除測試題；(4) launchd plist 存在且指向 `/Applications/Kunsu.app`；任一步失敗以非零 exit + 人話原因指出壞在哪一段

### 場景 7：doctor 測試題不污染正式流

**Given** doctor 建立測試題
**When** 測試題走完建→答→消化→清除
**Then** 測試題帶 internal 標記：不佔當日 T0 forced 預算、不推 Discord 鏡像、不觸發真實 reactor spawn（dry-run）；ledger 留的行帶 internal 標記，統計與週報排除

### 場景 8：launchd 衛生——絕不指 repo build 路徑

**Given** `scripts/build-install.sh` 執行
**When** 安裝流程跑完
**Then** 依序：build → 備份現有 /Applications/Kunsu.app → 拷貝新 build 到 /Applications → `launchctl bootout/bootstrap` 重載 `~/Library/LaunchAgents/me.ray.kunsu.plist` → 跑 `kunsu doctor` 冒煙；plist 內路徑指向 `/Applications/Kunsu.app`，任何情況下不得指向 `~/Desktop/kunsu` build 產物

### 場景 9：安裝失敗自動回滾

**Given** build-install.sh 走到 doctor 冒煙
**When** doctor 任一步失敗
**Then** 自動 rollback 到上一個 .app 備份、重新 bootstrap、再跑一次 doctor 確認舊版活著；rollback 結果（含失敗原因）大聲輸出

### 場景 10：app 死掉 = 退化為現狀，不是崩潰

**Given** Kunsu app 進程死亡一整週
**When** agents 與使用者照常工作
**Then** 系統退化為現狀等價物：agent 建題走檔案 inbox + Discord 鏡像（降級即現狀）；使用者與 agent 仍可讀 `OPEN.json` 知道欠什麼；ledger 既有稽核完好；復活後 inbox ingest + reconcile 補漏，答案最晚隔天 09:33 被 drain 消化——全程零資料遺失

### 場景 11：mkt-daily startup drain 保底

**Given** mkt-daily routine startup
**When** 硬性第一步 `kunsu drain --consumer mkt-daily`
**Then** 自上次游標以來的終態逐條落地：pending-approvals.json 對應項回寫 `approved_at/approval_channel/question_id/signature`；drain 到即標 `acked_at`；就算 Lane 2 推播死一整週，Lane 1 保證答案被消化

## 驗收清單

- [ ] AC-SV-01：/health 無 auth 回五欄位，僅 loopback 可達
- [ ] AC-SV-02：health 失敗或 heartbeat 逾 24h → mkt-daily 日報頭條紅字驗屍行
- [ ] AC-SV-03：正常日日報尾附一行 Kunsu 心跳（✅｜待決 N｜最老 N 天）
- [ ] AC-SV-04：ledger 全部寫入路徑純 append，既有行零修改；OPEN.json 每次變更重寫；git 追蹤不 ignore（INV-6，測試 `inv_ledger_append_only`）
- [ ] AC-SV-05：刪除 SQLite 後 `kunsu rebuild-db` 完整重建 pending/終態/approval 消耗狀態（INV-13，測試 `inv_rebuild_from_ledger`）
- [ ] AC-SV-06：`kunsu doctor` 覆蓋雙 CLI MCP／health／建題→答→消化→清除／launchd 指向四段，全綠 exit 0、失敗人話報段
- [ ] AC-SV-07：doctor 測試題 internal 標記：不佔 T0 預算、不推 Discord、reactor 只 dry-run、統計排除
- [ ] AC-SV-08：launchd plist 指向 /Applications/Kunsu.app，build-install.sh 全流程含備份與 launchctl 重載
- [ ] AC-SV-09：doctor 冒煙失敗自動 rollback 上一版 .app 並驗證舊版存活
- [ ] AC-SV-10：app 死一週的降級演練：inbox+Discord 建題、OPEN.json 可讀、復活後 ingest+reconcile+drain 零遺失
- [ ] AC-SV-11：mkt-daily startup drain 逐條落地並回寫 pending-approvals 結構化審批欄位
- [ ] AC-SV-12：mkt-daily 兩份 SKILL.md（routine prompt + vault skill）同步改，防斷鏈
