# 排程督導範本（Claude Routines／Codex automations）— 設定排程時才讀

Kunsu 不排程；排程交給 Claude 的排程任務或 Codex automations。排程跑起來時用 `kunsu ask --routine` 建一題
check-in，使用者必須回應（同一個 15 分鐘強制循環）；答案回到這次排程的對話，你處理完用 `ask_report` 回饋。
督導頁會列出每次的回答與你的回饋。

以下時間與檔案位置都是範例。先讀已存在的排程與授權，沿用它的時間、目標與記錄位置；不能因讀到範例就新增、改時段或臆造承諾。

## 共同規則

- 題目 id：`rt-<routine>-<YYYYMMDD>`（一天多次加 `-<HHmm>`），同一次重送不會重複。
- 建題：`kunsu ask --json q.json --routine <穩定 id> --routine-name <名稱> --every <小時>`。
- 建完照 SKILL §4：Claude 用 background Bash 掛 `kunsu wait <id> --timeout 0`；Codex 不用等。
- 拿到答案後做完你的工作（例如寫進 vault），一定要 `ask_report`：`summary_md` 寫你做了什麼、給使用者一句具體建議。
- 目標與承諾的正本放在 vault 或排程 prompt，不要塞進 Kunsu。

## 範本 1：每天寫日記（每天 21:30）

排程 prompt：

```
用 kunsu skill 問使用者今天的日記。題目 id rt-diary-$(date +%Y%m%d)，kind=text，tier=t1，
question_line「今天最重要的三件事？」，why 寫昨天日記裡沒完成的事（讀 vault journal/ 最近一篇）。
建題指令加 --routine diary --routine-name 每天寫日記 --every 24。
拿到回答後寫進 journal/<今天>.md，ask_report 回報寫入位置，並給一句明天的建議。
```

## 範本 2：每週目標檢討（每週日 20:00）

```
用 kunsu skill 問使用者這週的目標進度。題目 id rt-weekly-$(date +%Y%m%d)，kind=choice，
options：「三個都有進展」「有一個卡住」「這週偏離了」，why 列出 vault goals/current.md 的三個目標與上週的回答。
需要比較數據時用草稿＋畫布（同資料夾 canvas.md）。建題加 --routine weekly --routine-name 每週目標檢討 --every 168。
拿到回答後更新 goals/current.md 的進度欄，ask_report 回報更新內容與下週第一步。
```

## 範本 3：課程或專案復盤（沿用既有排程）

用 templates/retrospective/question.md 準備草稿。填入實際課次、上一輪承諾、產出與驗證邊界，再使用穩定 routine id／既有 cadence 建題。
收到正式回答後整理一個下一步，寫回原排程指定的記錄位置，再 ask_report；沒有授權的對外發布或花錢只留提案。
