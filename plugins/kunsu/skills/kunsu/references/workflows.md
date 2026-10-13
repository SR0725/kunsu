# 建題範本與故障處理

## 範本

在外掛根目錄執行（目前 source 通常是 ~/plugins/kunsu）：

```sh
python3 scripts/manage.py draft --template decision --id pa-你的穩定事件id
```

| 範本 | 使用情境 | 使用前必做 |
|---|---|---|
| decision | 真正需要使用者拍板的單一阻塞 | 查清事實、完整建議與後果 |
| urgent | 確實需要立即取得判斷 | 寫明真實期限；緊急不自動升 T0 |
| diary | 已授權日記督導 | 用 rt-日記排程id-日期，加既有 routine flags |
| retrospective | 課程或專案復盤 | 補上課次、真正產出與未驗證項目 |
| batch | 1–20 個同類待決項目 | 先讀 `references/batch.md`（規則、上限、空草稿、第 2 輪 `-r2`）；每項原文與完整提案；單項回覆也用 batch；不能放動錢的項目 |

命令只建草稿。修改其中的提示文字、選項與證據後，使用 `~/.kunsu/bin/kunsu ask <draft_dir> --dry-run`；查看 preview_png，再正式送出。
不要改寫 source 為範例 id，不要直接把「替換成…」等範本提示當成正式題目。canvas 需要時才讀 canvas.md。

## 診斷與回復

1. `python3 scripts/manage.py doctor`：實際唯讀握手／工具／dnd_get。
2. `~/.kunsu/bin/kunsu doctor </dev/null`：app、帳本與宿主設定。Agent 不得使用真人 answer 子命令。
3. 若外掛已安裝但舊對話沒有工具，開始新對話／行程；若使用一般 PATH CLI 時不認得當前模型，使用已安裝桌面版內建 CLI，不修改模型來掩蓋版本問題。
4. token 缺失：開啟 Kunsu，讓既有 app 初始化。不自行製造、印出、外傳 token。
5. app 離線：開啟已安裝 app；必要時 JSON 建題走既有 CLI inbox fallback。草稿預覽需在線，失敗不等於送出。
6. schema／validation issue：修正指明欄位、保持同 id，再 dry-run。不要反覆換 id 或取消問題來逃避錯誤。

安裝／更新：`python3 scripts/manage.py install`；版本、安裝 source/cache 與備份位置都在 JSON 回執中。完整回復程序見外掛 README。
