# 帳號服務（kunsu-cloud）設計

> PRD §31.2；BDD `docs/bdd/accounts.md`。Target，2026-10-13。

## 原則

雲端只存帳號基本資料與裝置清單（PRD §31.2）。任何題目、答案、證據、對話、檔案路徑都不進雲端；中轉站只轉密文（INV-30）。

## 服務

一個 Node 服務 `cloud/`（部署在 Zeabur，專案 kunsu-relay），同一個網域提供：

- `https://api.kunsu.app/auth/*`、`/account/*`：帳號。
- `wss://api.kunsu.app/v1/connect`：中轉站（沿用 `relay/server.js`，Mac 上線改成驗每台 Mac 自己的憑證）。
- 資料庫：Zeabur Postgres。測試用記憶體實作（同一個儲存介面）。

## 資料表（INV-33：只准這些欄位）

| 表 | 欄位 |
|---|---|
| `users` | id、email、display_name、created_at、deleted_at |
| `identities` | user_id、provider（email／google／apple）、subject、created_at |
| `login_tokens` | token_hash、kind（link／code／poll）、email、user_id、client、pair_hash、expires_at、used_at |
| `sessions` | token_hash、user_id、client（web／mac／ios）、created_at、last_used_at、revoked_at |
| `macs` | id（mac_id）、user_id、name、kx_pub、sig_pub、relay_token_hash、created_at、last_seen_at、revoked_at |
| `devices` | id（device_id）、user_id、name、created_at、last_seen_at、revoked_at |

所有秘密（登入連結、session、中轉站憑證）只存 SHA-256 雜湊。

## 登入

- **Email 一次性連結**：`POST /auth/email/start {email, client}` → Resend 寄出 `https://kunsu.app/auth/verify?token=…`；15 分鐘有效、只能用一次；
  同一個 email 每 10 分鐘最多 5 封、同一個 IP 每小時最多 20 封；回應永遠是 202（不透露帳號存不存在）。
- **Google**：`GET /auth/google/start?client=` → Google OAuth（openid email profile、PKCE、state）→ `/auth/google/callback`。
- **Apple**：網頁 `GET /auth/apple/start` → `POST /auth/apple/callback`（form_post）；iPhone 原生 `POST /auth/apple/native {identity_token}`。
  id_token 一律用 Apple／Google 的 JWKS 驗簽、驗 aud／iss／exp。
- 同一個已驗證 email 用不同方式登入 → 同一個帳號。
- **App 等待登入**（AC-AC-09）：`/auth/email/start` 一律回 `{ok, poll}`；App 每 2 秒 `POST /auth/poll {poll}`，使用者在任何裝置點了信裡的連結後
  拿到 `{session}`（只能領一次、15 分鐘）。連結頁 `GET /auth/verify` 自動完成登入，告訴使用者回到 App（官網上線前由帳號服務提供）。
- **完成登入**：網頁設 `kunsu_session`（HttpOnly、Secure、SameSite=Lax、30 天）；Mac／iPhone 收到 `kunsu://auth?code=…`（一次性、2 分鐘），
  `POST /auth/exchange {code}` 換 bearer session。

## 帳號與裝置

- `GET /account` → `{email, display_name, providers[], macs[], devices[]}`；`PATCH /account {display_name}`。
- `DELETE /account` → 立刻撤銷所有 session 與中轉站憑證、標記刪除；30 天後清除列（INV-35）。
- `POST /account/macs {mac_id, name, kx_pub, sig_pub}` → `{relay_token}`（只回一次）；`DELETE /account/macs/:id` 撤銷。
- 手機登入後 `GET /account` 看得到自己的 Mac，點選後照舊送配對請求，**仍要在 Mac 上按允許**（INV-29）。

## 中轉站

Mac 上線帶 `Authorization: Bearer <relay_token>`，中轉站查 `macs.relay_token_hash`（未撤銷、帳號未刪除）。
相容期：`RELAY_MAC_TOKEN` 仍可用（作者現在這台 Mac），公開發表前移除。

## 寄信

Resend API，寄件人 `Kunsu <login@kunsu.app>`；信件只含登入連結，沒有追蹤像素。
