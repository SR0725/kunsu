# 手機配對與 Zeabur 中轉協定（Target，2026-10-12）

PRD §30.5、§30.8；BDD `docs/bdd/mobile-pairing.md`。實作完成前，CONTRACT 與 INVARIANTS 不宣稱任何一條。

## 角色

```
iPhone（KunsuMobile） ⇄ wss 中轉站（Zeabur，只轉密文） ⇄ Mac 橋接（Kunsu app 內，主動往外連）⇄ Kunsu 核心
```

- Mac 不開任何對外 port；橋接主動連中轉站。
- 中轉站不持有任何解密金鑰，不寫硬碟。

## 金鑰

| 誰 | 金鑰 | 存放 | 用途 |
|---|---|---|---|
| Mac | X25519 `mac_kx`、Ed25519 `mac_sig` | macOS 鑰匙圈 | 金鑰交換、簽 Mac 送出的訊息 |
| 手機 | X25519 `dev_kx` | iOS 鑰匙圈（本機限定） | 金鑰交換 |
| 手機 | P-256 `dev_sig` | Secure Enclave（不可匯出） | 簽手機送出的每個請求 |
| Mac ↔ 中轉站 | `RELAY_MAC_TOKEN` | Zeabur 環境變數＋Mac 鑰匙圈 | 只有這台 Mac 能以 `mac_id` 上線 |

`device_id = base32(sha256(dev_sig 公鑰))[0..16]`；`mac_id = base32(sha256(mac_sig 公鑰))[0..16]`。

## 配對

1. 主視窗「配對新手機」產生 8 碼一次性配對碼（5 分鐘、只能用一次），QR 內容：
   `kunsu-pair:v1?relay=<wss URL>&mac=<mac_id>&code=<code>&kx=<mac_kx pub b64url>&sig=<mac_sig pub b64url>`
2. 手機產生金鑰，送 `pair_request` 給 `mac_id`：以 `mac_kx` 公鑰做 ECIES（手機臨時 X25519 → HKDF-SHA256 → ChaCha20-Poly1305）
   封裝 `{code, device_name, dev_kx_pub, dev_sig_pub, ts}`。
3. Mac 解開、以固定時間比對配對碼；碼過期或用過就回 `pair_rejected`。碼正確時主視窗跳出「<device_name> 要配對」，
   使用者按「允許」才把裝置寫進裝置清單（`~/.kunsu` 下的 devices 檔，只存公鑰、名稱、時間），回 `pair_accepted`（以 `mac_sig` 簽章）。
4. 手機驗 `mac_sig` 後儲存配對；之後每次連線都用這組金鑰。

## 訊息

- 對稱金鑰：`k = HKDF-SHA256(X25519(mac_kx, dev_kx), salt = device_id, info = "kunsu-relay-v1")`。
- 封包：`{v:1, dev, seq, ts, nonce, ct, sig?}`；`ct = ChaCha20-Poly1305(k, nonce, json, aad = v|dev|seq|ts)`。
- 手機 → Mac 的封包必有 `sig = P256-ECDSA(dev_sig, v|dev|seq|ts|nonce|ct)`。
- Mac 拒絕：裝置不在清單或已撤銷、簽章錯、`seq` 不大於上次、`|ts − now| > 120 s`、解密失敗。拒絕不寫任何答案，只記診斷。
- Mac → 手機的封包用 `mac_sig` 簽章，手機同樣驗證。

### 手機 → Mac

| type | 內容 | 說明 |
|---|---|---|
| `sync` | `{since?}` | 取待處理題目、對話狀態、紀錄 |
| `answer` | `{question_id, choice_id?, text?, money_confirmed?}` | 以 channel=`mobile` 作答；動錢必帶 `money_confirmed: true` |
| `permission` | `{session_id, allow}` | 批准或拒絕指令 |
| `reply` | `{session_id, text}` | 回覆等你回話的對話 |
| `followup` | `{id, question_id, kind: why\|context\|text, text?}` | 追問中轉 AI（Current 2026-10-12） |
| `forward` | `{id, question_id, text}` | 問原 AI 本人（Current 2026-10-12） |
| `chat` | `{id, text}` | 跟中轉 AI 聊天（Current 2026-10-12） |
| `start_task` | `{id, agent, project, text}` | 開新任務（Current 2026-10-12） |
| `settings` | `{relay_agent}` | 中轉 AI 設定（與主視窗同一個值） |

### Mac → 手機

`snapshot`（含 `projects[]`）、`event`（題目／對話變化）、`ack {for, ok, error?}`，以及（2026-10-12）：

| type | 內容 | 說明 |
|---|---|---|
| `followup_answer` | `{id, ok, text?, error?, by: relay\|agent}` | 追問或問本人的回覆；`id` 對到手機送出的那一則 |
| `chat_reply` | `{id, ok, text?, error?}` | 中轉 AI 聊天回覆 |
| `task_started` | `{id, ok, error?, agent, project}` | 新任務已在 Mac 上開始 |
| `task_done` | `{id, ok, text}` | 新任務跑完（最後一段輸出，最多 1,200 字）或失敗原因 |

## 中轉站

- `GET /v1/connect?role=mac&mac=<id>`，Header `Authorization: Bearer RELAY_MAC_TOKEN`。
- `GET /v1/connect?role=device&mac=<id>&dev=<id>`：不需 token，但只能跟指定 `mac_id` 互傳；Mac 對未知裝置一律丟棄。
- 依 `mac_id`／`device_id` 轉送 JSON 封包，不解析 `ct`。Mac 離線時每台裝置最多暫存 50 個封包、10 分鐘，只在記憶體。
- 每個 IP 每分鐘 120 個封包上限；單一封包 256 KB 上限。
- 只走 TLS（Zeabur 網域）。

## 撤銷與遺失

主視窗「手機」卡片列出裝置（名稱、配對時間、最後上線），「撤銷」立即生效：Mac 之後丟棄該裝置所有封包。手機遺失時撤銷即可，
私鑰在 Secure Enclave，無法被複製到其他裝置。
