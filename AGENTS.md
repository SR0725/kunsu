# Kunsu（軍師）— Agent 建造與維護契約

Kunsu 是給 AI Agent 使用者的強制決策收件匣桌面 app。產品的核心不是聊天或一般通知，而是：當 AI Agent
因缺少人類決策而無法安全繼續時，可靠地取得正式回答。

任何進入本 repo 的 Agent 都必須遵守本文件。這是工作流程的持久化存檔，不因單次對話、上下文
壓縮或執行工具不同而失效。

## 1. 文件正本與優先順序

開始工作前，依需求範圍閱讀下列正本：

1. [`docs/PRD.md`](docs/PRD.md)：產品目的、範圍、目標行為與成功標準。可描述已核准但待實作的 Target。
2. [`DESIGN.md`](DESIGN.md)：顏色、字體、間距、動效、內容 grammar 與三個產品 surface。
3. [`docs/CONTRACT.md`](docs/CONTRACT.md)：目前已實作的工程契約、API、狀態機與資料責任。
4. [`INVARIANTS.md`](INVARIANTS.md)：永遠不能被破壞的安全／資料性質；每條必須有同名自動化測試。
5. [`docs/bdd/`](docs/bdd/README.md)：使用者可觀察的 Given/When/Then 與逐條 AC。
6. 設計歷史：初版系統設計（2026-07-31）放在 repo 以外的作者筆記，只供脈絡與決策沿革；
   一律以 repo 內文件標示的 Current／Target 邊界為準。

### Current 與 Target 不得混淆

- PRD 與標示 **Target** 的 BDD 可以描述已核准、尚未實作的目標。
- CONTRACT、INVARIANTS 與測試只能宣稱目前真的成立的行為。
- 不得只改 CONTRACT 假裝功能完成，也不得因程式還沒做就把已核准 Target 從 PRD 刪掉。
- 發現文件互相矛盾時，立即停止 code change，先指出衝突並修正正本。

## 2. 強制工作流：Documentation → BDD → Test → Code → QA

任何會改變產品行為、狀態、UI/UX、API、資料、policy 或安全邊界的需求，都必須依序完成以下
階段。**沒有文件 diff，不得開始產品 code。**

### Phase 0 — 盤點與分類

1. 執行 `git status --short`，保護使用者既有變更，不覆蓋不相關工作。
2. 閱讀本文件與所有相關正本。
3. 將需求分類為：產品行為、UI/UX、契約/API、invariant、安全、policy、bug 或純文件。
4. 寫出 Current 與 Target 的差異；不能直接從口語需求跳到實作。

### Phase 1 — 文件先行

依變更範圍更新：

- 產品目的、使用者流程、優先級、scope → `docs/PRD.md`
- 視覺、interaction、motion、responsive、accessibility → `DESIGN.md`
- 已核准且準備實作的 API、狀態、資料模型 → `docs/CONTRACT.md`
- 永遠成立的性質 → `INVARIANTS.md`
- 可觀察行為與 acceptance criteria → `docs/bdd/<feature>.md`

非微小功能必須先把文件 diff 呈交使用者確認，再進 Phase 2。若使用者只要求規劃／文件，到 Phase 1
完成即停止，不得自行實作。

### Phase 2 — BDD 規格

每個功能域建立或更新一份 BDD，至少包含：

- 背景、適用範圍與界外。
- 正常路徑、錯誤路徑、邊界、復原、冪等與安全場景。
- 每個場景使用明確的 **Given / When / Then**。
- 每條 acceptance criterion 有穩定 ID，例如 `AC-FC-05`。
- 說明要由 unit、integration、UI 或 end-to-end 哪一層驗證。

不能把「看起來正常」「應該絲滑」當成 AC；要轉成可觀察條件，例如時間、位置、焦點、狀態、
事件數量、錯誤回執或 layout shift。

### Phase 3 — 測試先紅（Red）

1. 先建立與 AC 對應的失敗測試，再改產品 code。
2. INVARIANTS 每新增／修改一條，必須先新增／修改同名測試。
3. 時間行為使用可注入 clock，不以真實 sleep 讓測試等待 15 分鐘。
4. 狀態機優先用純函數與 table-driven tests；guards 必須逐一枚舉。
5. 記錄測試在實作前確實因缺少目標行為而失敗；不能寫永遠通過的裝飾性測試。

### Phase 4 — 最小實作（Green）

只實作讓已確認 BDD 通過的最小變更，不順手擴張 scope。保持層級責任：

- 打斷預算／時窗／snooze 參數 → `config/policy.toml`（熱載）
- reactor 行為 → `playbooks/*.md`
- UI → `ui/`（React，build 後熱換）
- Rust core 只放穩定核心：狀態機、HTTP/MCP、持久化協定與視窗管理

如果實作揭露規格缺口，回到 Phase 1/2 修改文件與 AC；禁止讓 code 靜默成為新規格。

### Phase 5 — Refactor 與一致性

在測試保持綠色的前提下整理重複與命名。完成後逐項確認：

- PRD/BDD 的 Target 是否已可改標為 Current。
- CONTRACT 是否與實作一致。
- INVARIANTS 與測試名是否一致。
- DESIGN 的所有狀態、motion 與 accessibility 是否已落地。
- policy 預設值、migration、診斷與 rollback 是否齊全。

### Phase 6 — QA 與交付

1. 先跑針對性測試，再跑完整 smoke。
2. 完工後應由乾淨 context 的獨立 QA Agent 逐條核對 BDD；只有在使用者已授權 agent delegation
   且環境支援時才啟動。否則由目前 Agent 做隔離的第二輪稽核，並明確告知未使用獨立 Agent。
3. QA 通過前不能勾選 BDD checklist；不能由實作者憑印象宣稱通過。
4. 回報變更檔案、測試證據、未完成 Target、風險與 rollback。

## 3. 自主變更三鐵則（違反即違規）

1. **必 git commit**：不 commit 的改動視為不存在；不跨層混 commit。
2. **變更後必過 smoke**：至少執行 `cargo test` 與 `scripts/build-install.sh`（內建 doctor）。失敗時
   不得提交為完成；依現有 recovery 流程 revert 並發 Discord 告警。
3. **INVARIANTS 各配一條測試**：改 invariant = 先改文件與測試，再改 code。

### Commit 分層

一般順序為：

1. `docs:` PRD／DESIGN／CONTRACT／BDD／INVARIANTS
2. `test:` 先紅的測試與 fixtures
3. `core:`／`ui:`／`reactor:` 最小實作
4. `build:`／`chore:` migration、build 或 release glue

純文件需求可以只有 `docs:` commit。禁止把大量文件、狀態機、UI 與無關 cleanup 混成一包。

## 4. Kunsu 產品不變的本質

- Kunsu 是「需要人類決策時的強制收件匣」，桌面端不是聊天介面。手機 App（PRD §30）可以檢視、接續與建立
  Claude／Codex 對話，但決策題永遠用決策卡作答，聊天訊息不會變成正式答案。
- Kunsu 是使用者的 AI 長官，像最嚴厲的父親：AI 判定需要他、或他交給排程督導的承諾到期時，不讓他假裝
  沒看見；但嚴厲只限於逼他面對與回應，永不代答、代授權或把他擋死（PRD §1.4）。2026-10-13 起是公開產品（PRD §31），
  內容只留在使用者自己的電腦。
- 回答要回到發問的那段對話接續；只有確定無法接續才另開新對話（PRD §20）。
- 未回答不等於已處理；關閉 UI 不能偷偷結案。
- 強制顯示不等於自動作答、授權或執行。
- 同一時間只處理一個清楚的決策 surface。
- 一般 FYI、背景進度與成功通知不進 Kunsu。
- 來源重送與 app restart 不得製造重複邏輯問題或重設關鍵時序。

## 5. 紅線（永不做）

第二條動錢通道｜重建排程引擎｜桌面端聊天介面｜收 FYI 通知｜真相搬進 app DB｜繞過
decision↔prediction｜Kongko 深度 API 綁定（只准 deep link + publish webhook + MCP reconcile）｜
發佈=啟用授權｜螢幕分享/錄影/會議中蓋板｜雲端保存內容（雲端只存帳號基本資料與裝置清單；中轉站只轉端對端加密封包與無內容推播，PRD §30、§31）｜團隊共用收件匣。
多使用者與對外公開產品已於 2026-10-13 核准（PRD §31）。

完整產品範圍見 `docs/PRD.md` §14；歷史清單見舊設計正本 §14。任何要求若碰紅線，先停止並
向使用者說明，不得用「只是 prototype」繞過。

## 6. Design system 與 UI QA

- 任何 UI 變更前先讀 `DESIGN.md`。
- Agent 生成的 Markdown 內文只能走 allowlist sanitizer；不得注入任意 CSS、script、form、iframe 或互動。唯一例外是
  證據畫布（PRD §24，Claude Artifact 模式）：只能在 `sandbox="allow-scripts"`＋禁網路 CSP 的 iframe 中執行，拿不到
  IPC、不能作答；題目、選項、送出與錢參數永遠由 Kunsu 原生呈現。
- UI QA 必須檢查：無待決時膠囊隱藏、拖曳與點擊分離、鍵盤焦點、screen reader、
  `prefers-reduced-motion`、長內容裁切／捲動、safe HTML sanitizer、不同螢幕／邊界位置。
- 現行膠囊固定 44 px 高；hover 不顯示 tooltip、不改變幾何、不妨礙拖曳。邊緣瀏海（PRD §19，Target）
  交付後改為：厚度固定 32 px、純黑貼邊；hover 只允許沿邊延長與跑馬燈，不顯示 tooltip、不改變厚度、
  開始拖曳即取消；命中區以外零攔截。
- 強制蓋板只在內容可操作後出現，Esc 與關閉在任何狀態下有效（PRD §22）。
- 動畫是 progressive enhancement。動畫或 WebGL 失敗時，回答流程仍須完整可用。
- 倒數、warning 與狀態不能只靠顏色表達，也不能造成 layout shift。

## 7. Definition of Done

一項行為變更只有在以下全部成立時才算完成：

- 相關 PRD、DESIGN、CONTRACT、INVARIANTS、BDD 已更新且互不矛盾。
- 每條新／改 AC 有自動化測試，且已看過 Red → Green。
- 所有既有與新增測試通過。
- `cargo test` 與 `scripts/build-install.sh` doctor 通過。
- UI 變更完成 keyboard、reduced-motion、長內容、sanitizer 與 guard QA。
- migration、failure recovery、diagnostics、rollback 已處理或明確標為界外。
- BDD checklist 有可追溯的 QA 證據。
- 改動依層級 commit，worktree 沒有被誤傷的使用者檔案。
- 最終回報清楚區分：已完成、待實作 Target、已驗證、未驗證。

如果任何一項尚未成立，只能回報「進行中／有阻塞」，不能宣稱完成。
