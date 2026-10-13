# Kunsu 軍師 — 本機外掛 0.2.0

需要使用者的正式決定時，讓 Claude／Codex 清楚地來問；正式回答回到原對話，工作完成後回報結果。已授權的日記／復盤也使用同一個收件匣。

外掛包括已選 K icon、操作技能、13-tool stdio bridge、五種草稿範本、安裝／更新與唯讀診斷。產品與操作設計見 [DESIGN.md](DESIGN.md)。

## 安裝與更新

需求：macOS 已安裝並開啟 Kunsu、Python 3.11+、相容的 Codex desktop／CLI。沒有額外 Python 套件或 npx 下載。從這個外掛資料夾執行：

```sh
python3 scripts/manage.py install
python3 scripts/manage.py doctor
~/.kunsu/bin/kunsu doctor </dev/null
```

install 驗證 source 與本機服務後，備份現有 source、Codex config、personal marketplace 與共用技能；逐項保留其他設定，用宿主 `plugin add` 安裝，不直接改 cache。只設定 `kunsu@personal` enabled、kunsu-inbox 工具批准與舊 kunsu MCP enabled=false；其他外掛、MCP、模型不變。已存在的 Claude 技能附加檔案保留，Claude HTTP 註冊不改。

JSON 回執含版本、備份位置與 cache/source 比對結果。重複執行不增加第二個 Kunsu entry。若 personal marketplace 被改名、TOML 不合法或目的地／package 使用 symlink，會停止而不覆寫。

開始新 Codex 對話／行程以載入新版外掛；舊對話不保證即時重載。Claude／Codex 的共用 Kunsu skill 也已更新，既有 Claude HTTP 連線繼續使用。

## 建題與實際使用

對 Agent 說「把這題送到 Kunsu」，讓它查清事實、填入建議、預覽後送出。一般問題優先用已安裝 CLI 自動保留來源身分；使用 MCP 必須帶真實來源對話 id 與 cwd，bridge 不猜共用行程身分。

```sh
python3 scripts/manage.py draft --template decision --id pa-你的穩定事件id
```

可用範本：decision、urgent、diary、retrospective、batch。命令只建立 `~/.kunsu/drafts/<id>/question.md`，不送出問題。修改提示文字與證據後：

```sh
~/.kunsu/bin/kunsu ask ~/.kunsu/drafts/pa-你的穩定事件id --dry-run
```

查看 preview_png，確認內容可理解且沒有 issue，再移除 --dry-run 正式註冊。既有 id 不覆蓋，重送正式問題保持同 id。日記／復盤沿用已有排程與時間，不因範本自動新增承諾。

## 連線與授權

- 固定 `http://127.0.0.1:4777/mcp`，每次從 `~/.kunsu/token` 讀 Bearer；禁 proxy／redirect。不把 token 寫入設定、package、草稿或聊天。
- tools：ask_create、ask_register、ask_wait、ask_answers_since、ask_list、ask_get、ask_update、ask_cancel、ask_supersede、ask_report、dnd_get、dnd_set、dnd_clear。
- 真人在 Kunsu 作答；沒有 answer／approve／execute。沉默、關閉與過期都不代表同意。金錢操作只走既有錢鏈，不新增通道。
- 勿擾只有使用者明確要求才改。一般 FYI、成功通知不建題；未知資料不當零。
- Codex 建題後交還控制權；Claude 背景 wait。完成後 ask_report，避免把「收到答案」當完成。

## 診斷與回復

doctor 真的初始化、發現 14 tools 並唯讀呼叫 dnd_get；不建題、不改勿擾。離線／缺 token 回 nonzero 與下一步，原始例外與憑證不輸出。開啟 app 後重試，詳細流程見 [workflows.md](skills/kunsu/references/workflows.md)。

安裝失敗時，已寫入的本機檔案會回復，嘗試 source 留在 backup/failed-source。若其他程序同時改寫設定，保留外部修改、報 conflict；不要整份覆蓋後續變更。宿主 cache 若有部分寫入，必須用受支援的 plugin add 重新確認，不能把本機檔案回復當成宿主 cache 已回復。

回復一個成功更新：先將 kunsu@personal 設為 disabled；保留目前 ~/plugins/kunsu，再從回執的 previous-source 複製回原位置。僅還原此次 Kunsu 的啟用／批准欄位與 marketplace entry，從 shared-skill 回復此次更新的技能檔案，再用 Codex 的 plugin add 載入上一版並執行兩個 doctor。若前版沒有外掛，保留 disabled 即可；其他設定、額外技能檔案與新產生的題目不能刪。

App 回復另用 `/Applications/Kunsu.app.bak`。不要刪除 ~/.kunsu 題目、token 或帳本；包含憑證的備份只留在受限本機資料夾，不入 package／repo。

## 開發驗證

repo 根目錄：`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p '*plugin*.py'`（asset gate 使用 Pillow）、`cargo test`、`pnpm --dir ui test`、`scripts/build-install.sh`。範本另經已安裝 App dry-run，不用 Agent 模擬真人作答。
