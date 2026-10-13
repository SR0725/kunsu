//! 手機橋接（PRD §30.8；docs/design/mobile-relay.md；INVARIANTS 7、15、28、29）。
//!
//! Mac 主動連使用者的 Zeabur 中轉站；手機的封包經裝置簽章驗證後，以 channel=mobile 作答。
//! 金鑰、裝置清單與設定都是 `~/.kunsu/` 底下的 0600 檔案，不用鑰匙圈（2026-10-12 作者：避免每次跳授權）。
//! `handle_relay_message` 是純流程（收一個中轉站訊框 → 回要送出的訊框），方便在測試裡不連網路驗證。

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};
use kunsu_core::types::{AgentKind, Channel, Kind, Question, State};
use kunsu_mobile::{
    open_from_device, pair_accepted, qr_payload, qr_svg, seal_from_mac, Device, Envelope, MacKeys, PairRequestWire,
    PairingBook, PendingPair, Registry,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::approval::KeyProvider;
use crate::relay_ai::{self, AiRunner, CliRunner, RelayAgent};
use crate::service::{AnswerInput, ServerHandle};

pub const DEFAULT_RELAY_URL: &str = "wss://api.kunsu.app/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MobileConfig {
    relay_url: String,
    relay_agent: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingInfo {
    pub id: String,
    pub device_name: String,
    pub received_at: i64,
}

/// 主視窗「手機」卡片顯示的狀態。
#[derive(Debug, Clone, Serialize)]
pub struct MobileStatus {
    pub relay_url: String,
    pub connected: bool,
    pub last_error: Option<String>,
    pub mac_id: String,
    pub relay_agent: String,
    pub devices: Vec<DeviceInfo>,
    pub pending: Vec<PendingInfo>,
}

/// 這台 Mac 登入的帳號（PRD §31.2；BDD multi-user AC-MU-03）。session 存在 `~/.kunsu/account.json`（0600）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountFile {
    email: String,
    session: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingQr {
    pub payload: String,
    pub svg: String,
    pub expires_at: i64,
}

struct Paths {
    keys: PathBuf,
    devices: PathBuf,
    config: PathBuf,
    token: PathBuf,
    account: PathBuf,
}

impl Paths {
    fn in_dir(dir: &Path) -> Self {
        Self {
            keys: dir.join("mobile-keys.json"),
            devices: dir.join("mobile-devices.json"),
            config: dir.join("mobile.json"),
            token: dir.join("relay-token"),
            account: dir.join("account.json"),
        }
    }
}

/// 只有使用者讀得到（0600）的寫檔：先寫暫存檔再改名。
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)
}

struct BridgeState {
    registry: Registry,
    book: PairingBook,
    out_seq: HashMap<String, u64>,
    connected: bool,
    last_error: Option<String>,
    config: MobileConfig,
}

struct Inner {
    handle: ServerHandle,
    key: Arc<dyn KeyProvider>,
    keys: MacKeys,
    paths: Paths,
    state: Mutex<BridgeState>,
    changes: broadcast::Sender<()>,
    outbox: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    /// 換了憑證：叫醒在等待重連的迴圈。
    wake: tokio::sync::Notify,
    ai: Mutex<AiState>,
}

/// 中轉 AI 的記憶（只在記憶體）。
struct AiState {
    runner: Arc<dyn AiRunner>,
    /// 每支手機最近的聊天（含這一句，最多 `CHAT_MEMORY` 則）。
    chat: HashMap<String, VecDeque<(String, String)>>,
    /// 每題對軍師的追問與回覆。
    asked: HashMap<String, Vec<(String, String)>>,
    /// 問本人：追問 id → 哪支手機。
    forwards: HashMap<String, String>,
}

/// 問本人等多久沒回就告訴手機。
const FORWARD_TIMEOUT_MIN: i64 = 15;

#[derive(Clone)]
pub struct MobileBridge {
    inner: Arc<Inner>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Sync,
    Answer {
        question_id: String,
        #[serde(default)]
        choice_id: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        money_confirmed: bool,
        #[serde(default)]
        revision: Option<u32>,
        #[serde(default)]
        content_hash: Option<String>,
    },
    Settings {
        relay_agent: String,
    },
    Followup {
        id: String,
        question_id: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        text: Option<String>,
    },
    Forward {
        id: String,
        question_id: String,
        text: String,
    },
    Chat {
        id: String,
        text: String,
    },
    StartTask {
        id: String,
        agent: String,
        project: String,
        text: String,
    },
    #[serde(other)]
    Unsupported,
}

impl MobileBridge {
    /// 讀（或第一次產生）Mac 金鑰、裝置清單與設定。
    pub fn load(handle: ServerHandle, key: Arc<dyn KeyProvider>, dir: &Path, default_relay: &str) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("建立 {} 失敗：{e}", dir.display()))?;
        let paths = Paths::in_dir(dir);
        let keys = match std::fs::read_to_string(&paths.keys).ok().and_then(|s| MacKeys::from_json(&s)) {
            Some(k) => k,
            None => {
                let k = MacKeys::generate(&mut OsRng);
                write_private(&paths.keys, &k.to_json()).map_err(|e| format!("寫入手機金鑰失敗：{e}"))?;
                k
            }
        };
        let registry = std::fs::read_to_string(&paths.devices).ok().and_then(|s| Registry::from_json(&s)).unwrap_or_default();
        let config = std::fs::read_to_string(&paths.config)
            .ok()
            .and_then(|s| serde_json::from_str::<MobileConfig>(&s).ok())
            .unwrap_or(MobileConfig { relay_url: default_relay.to_string(), relay_agent: "claude".to_string() });
        let (changes, _) = broadcast::channel(16);
        Ok(Self {
            inner: Arc::new(Inner {
                handle,
                key,
                keys,
                paths,
                state: Mutex::new(BridgeState {
                    registry,
                    book: PairingBook::default(),
                    out_seq: HashMap::new(),
                    connected: false,
                    last_error: None,
                    config,
                }),
                changes,
                outbox: Mutex::new(None),
                wake: tokio::sync::Notify::new(),
                ai: Mutex::new(AiState {
                    runner: Arc::new(CliRunner),
                    chat: HashMap::new(),
                    asked: HashMap::new(),
                    forwards: HashMap::new(),
                }),
            }),
        })
    }

    /// 主視窗要知道：有新的待允許配對、裝置清單或連線狀態變了。
    pub fn subscribe_changes(&self) -> broadcast::Receiver<()> { self.inner.changes.subscribe() }
    fn changed(&self) { let _ = self.inner.changes.send(()); }

    pub fn status(&self) -> MobileStatus {
        let s = self.inner.state.lock().expect("mobile state");
        MobileStatus {
            relay_url: s.config.relay_url.clone(),
            connected: s.connected,
            last_error: s.last_error.clone(),
            mac_id: self.inner.keys.mac_id(),
            relay_agent: s.config.relay_agent.clone(),
            devices: s
                .registry
                .devices
                .iter()
                .map(|d| DeviceInfo { id: d.id.clone(), name: d.name.clone(), paired_at: d.paired_at, last_seen: d.last_seen, revoked: d.revoked })
                .collect(),
            pending: s
                .book
                .pending()
                .iter()
                .map(|p: &PendingPair| PendingInfo { id: p.id.clone(), device_name: p.device_name.clone(), received_at: p.received_at })
                .collect(),
        }
    }

    /// 「配對新手機」：產生一次性配對碼與 QR。
    pub fn start_pairing(&self, now: i64) -> PairingQr {
        let mut s = self.inner.state.lock().expect("mobile state");
        s.book.prune(now);
        let code = s.book.issue(now, &mut OsRng);
        let payload = qr_payload(&s.config.relay_url, &self.inner.keys, &code);
        PairingQr { svg: qr_svg(&payload), payload, expires_at: now + kunsu_mobile::CODE_TTL_SECS }
    }

    /// 使用者在主視窗按允許或拒絕（INV-29）。回要送給手機的訊框。
    pub async fn decide_pairing(&self, pending_id: &str, allow: bool, now: chrono::DateTime<chrono::Utc>) -> Result<Vec<String>, String> {
        let ts = now.timestamp();
        let frames = {
            let mut s = self.inner.state.lock().expect("mobile state");
            let dev = s.book.pending().iter().find(|p| p.id == pending_id).map(|p| p.device_id.clone()).ok_or("找不到這個配對請求（可能已過期）")?;
            if !allow {
                s.book.reject(pending_id);
                vec![json!({"to": dev, "body": {"t": "pair_rejected", "reason": "電腦上拒絕了這次配對"}}).to_string()]
            } else {
                let device = s.book.approve(pending_id, ts).map_err(|e| e.to_string())?;
                let accepted = pair_accepted(&self.inner.keys, &device, ts);
                s.registry.upsert(device.clone());
                write_private(&self.inner.paths.devices, &s.registry.to_json()).map_err(|e| format!("寫入裝置清單失敗：{e}"))?;
                vec![json!({"to": device.id, "body": accepted}).to_string()]
            }
        };
        self.changed();
        self.send_all(&frames);
        Ok(frames)
    }

    pub fn revoke(&self, device_id: &str) -> Result<(), String> {
        let mut s = self.inner.state.lock().expect("mobile state");
        if !s.registry.revoke(device_id) {
            return Err("找不到這台裝置".into());
        }
        write_private(&self.inner.paths.devices, &s.registry.to_json()).map_err(|e| format!("寫入裝置清單失敗：{e}"))?;
        drop(s);
        self.changed();
        Ok(())
    }

    /// 中轉 AI 用 Claude 還是 Codex（手機設定與主視窗同一個值）。
    pub fn set_relay_agent(&self, agent: &str) -> Result<(), String> {
        if agent != "claude" && agent != "codex" {
            return Err("中轉 AI 只能是 claude 或 codex".into());
        }
        let mut s = self.inner.state.lock().expect("mobile state");
        s.config.relay_agent = agent.to_string();
        write_private(&self.inner.paths.config, &serde_json::to_string_pretty(&s.config).expect("serializable"))
            .map_err(|e| format!("寫入手機設定失敗：{e}"))?;
        drop(s);
        self.changed();
        Ok(())
    }

    // ------------------------------------------------------------------ 收中轉站訊框

    /// 中轉站送來的一個訊框：`{from, body}`。回要送出的訊框（`{to, body}`）。驗證失敗一律不回、不作答（INV-28）。
    pub async fn handle_relay_message(&self, frame: &str, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
        let Ok(v) = serde_json::from_str::<Value>(frame) else { return vec![] };
        let Some(from) = v.get("from").and_then(Value::as_str).map(str::to_string) else { return vec![] };
        let body = v.get("body").cloned().unwrap_or(Value::Null);
        let ts = now.timestamp();

        if body.get("t").and_then(Value::as_str) == Some("pair_request") {
            let Ok(wire) = serde_json::from_value::<PairRequestWire>(body) else { return vec![] };
            let result = {
                let mut s = self.inner.state.lock().expect("mobile state");
                s.book.receive(&self.inner.keys, &wire, ts)
            };
            return match result {
                Ok(_) => {
                    self.changed();
                    vec![]
                }
                Err(e) => vec![json!({"to": from, "body": {"t": "pair_rejected", "reason": e.to_string()}}).to_string()],
            };
        }

        let Ok(env) = serde_json::from_value::<Envelope>(body) else { return vec![] };
        if env.dev != from {
            return vec![];
        }
        let opened = {
            let mut s = self.inner.state.lock().expect("mobile state");
            let r = open_from_device(&self.inner.keys, &mut s.registry, &env, ts);
            if r.is_ok() {
                let _ = write_private(&self.inner.paths.devices, &s.registry.to_json());
            }
            r
        };
        let plain = match opened {
            Ok(p) => p,
            Err(kunsu_mobile::Reject::Revoked) => {
                return vec![json!({"to": from, "body": {"t": "revoked"}}).to_string()];
            }
            Err(e) => {
                tracing::warn!("手機封包被拒（{from}）：{e}");
                return vec![];
            }
        };
        let kind = serde_json::from_slice::<Value>(&plain).ok().and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string)).unwrap_or_default();
        tracing::info!("手機指令（{from}）：{kind}");
        let reply = match serde_json::from_slice::<Command>(&plain) {
            Ok(Command::Sync) => self.snapshot(now).await,
            Ok(Command::Answer { question_id, choice_id, text, money_confirmed, revision, content_hash }) => {
                self.answer(&question_id, choice_id, text, money_confirmed, revision, content_hash, now).await
            }
            Ok(Command::Settings { relay_agent }) => match self.set_relay_agent(&relay_agent) {
                Ok(()) => json!({"type": "ack", "for": "settings", "ok": true}),
                Err(e) => json!({"type": "ack", "for": "settings", "ok": false, "error": e}),
            },
            Ok(Command::Followup { id, question_id, kind, text }) => {
                self.followup(&from, id, question_id, kind, text);
                return vec![];
            }
            Ok(Command::Forward { id, question_id, text }) => match self.forward(&from, &id, &question_id, &text, now).await {
                None => return vec![],
                Some(reply) => reply,
            },
            Ok(Command::Chat { id, text }) => {
                self.chat(&from, id, text);
                return vec![];
            }
            Ok(Command::StartTask { id, agent, project, text }) => self.start_task(&from, id, &agent, &project, text).await,
            Ok(Command::Unsupported) | Err(_) => {
                json!({"type": "ack", "ok": false, "error": "這個功能還沒接上 Mac（下一版）"})
            }
        };
        self.seal_to(&from, &reply, ts).into_iter().collect()
    }

    fn seal_to(&self, device_id: &str, body: &Value, ts: i64) -> Option<String> {
        let mut s = self.inner.state.lock().expect("mobile state");
        let device: Device = s.registry.get(device_id)?.clone();
        if device.revoked {
            return None;
        }
        let ms = (chrono::Utc::now().timestamp_millis()).max(0) as u64;
        let seq = s.out_seq.get(device_id).map_or(ms, |last| (*last + 1).max(ms));
        s.out_seq.insert(device_id.to_string(), seq);
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let env = seal_from_mac(&self.inner.keys, &device, seq, ts, body.to_string().as_bytes(), nonce)?;
        Some(json!({"to": device_id, "body": env}).to_string())
    }

    // ------------------------------------------------------------------ 帳號（PRD §31.2；BDD multi-user AC-MU-03）

    /// 帳號服務的網址：跟中轉站同一台（wss://host/v1 → https://host）。
    fn api_base(&self) -> String {
        let relay = self.inner.state.lock().expect("mobile state").config.relay_url.clone();
        let http = relay.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
        http.trim_end_matches('/').trim_end_matches("/v1").to_string()
    }

    fn read_account(&self) -> Option<AccountFile> {
        std::fs::read_to_string(&self.inner.paths.account).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn account(&self) -> Option<AccountInfo> {
        self.read_account().map(|a| AccountInfo { email: a.email })
    }

    /// 寄登入連結；回一個等待 id，之後用 [`Self::account_poll`] 等使用者點連結（在哪台裝置點都可以）。
    pub async fn account_start_login(&self, email: &str) -> Result<String, String> {
        let res = reqwest::Client::new()
            .post(format!("{}/auth/email/start", self.api_base()))
            .json(&json!({ "email": email.trim(), "client": "mac" }))
            .send()
            .await
            .map_err(|e| format!("連不上帳號服務：{e}"))?;
        if res.status().as_u16() == 400 {
            return Err("請輸入正確的 email".into());
        }
        let v: Value = res.json().await.map_err(|e| format!("帳號服務回應看不懂：{e}"))?;
        v.get("poll").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "帳號服務沒有回等待 id".into())
    }

    /// 等登入：還沒點連結回 `Ok(false)`；點了就登入、把這台 Mac 註冊到帳號、存好自己的中轉站憑證並重新上線，回 `Ok(true)`。
    pub async fn account_poll(&self, poll: &str) -> Result<bool, String> {
        let http = reqwest::Client::new();
        let api = self.api_base();
        let res = http.post(format!("{api}/auth/poll")).json(&json!({ "poll": poll })).send().await.map_err(|e| format!("連不上帳號服務：{e}"))?;
        match res.status().as_u16() {
            202 => return Ok(false),
            200 => {}
            410 => return Err("登入連結已失效，請重新寄一次".into()),
            s => return Err(format!("帳號服務錯誤（{s}）")),
        }
        let session = res.json::<Value>().await.ok().and_then(|v| v.get("session").and_then(Value::as_str).map(str::to_string)).ok_or("帳號服務沒有回 session")?;
        let account: Value = http.get(format!("{api}/account")).bearer_auth(&session).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
        let email = account.get("email").and_then(Value::as_str).unwrap_or_default().to_string();
        let keys = &self.inner.keys;
        let reg = http
            .post(format!("{api}/account/macs"))
            .bearer_auth(&session)
            .json(&json!({
                "mac_id": keys.mac_id(),
                "name": mac_name(),
                "kx_pub": kunsu_mobile::b64::encode(&keys.kx_public()),
                "sig_pub": kunsu_mobile::b64::encode(&keys.sig_public()),
            }))
            .send()
            .await
            .map_err(|e| format!("註冊這台 Mac 失敗：{e}"))?;
        if !reg.status().is_success() {
            return Err(format!("註冊這台 Mac 失敗（{}）", reg.status().as_u16()));
        }
        let relay_token = reg.json::<Value>().await.ok().and_then(|v| v.get("relay_token").and_then(Value::as_str).map(str::to_string)).ok_or("帳號服務沒有回中轉站憑證")?;
        write_private(&self.inner.paths.token, &relay_token).map_err(|e| format!("存中轉站憑證失敗：{e}"))?;
        let file = AccountFile { email, session };
        write_private(&self.inner.paths.account, &serde_json::to_string_pretty(&file).expect("serializable")).map_err(|e| format!("存帳號失敗：{e}"))?;
        self.reconnect();
        self.changed();
        Ok(true)
    }

    /// 登出：從帳號移除這台 Mac（中轉站立刻斷線）、刪掉本機的 session 與憑證。
    pub async fn account_logout(&self) -> Result<(), String> {
        if let Some(acc) = self.read_account() {
            let _ = reqwest::Client::new()
                .delete(format!("{}/account/macs/{}", self.api_base(), self.inner.keys.mac_id()))
                .bearer_auth(&acc.session)
                .send()
                .await;
        }
        let _ = std::fs::remove_file(&self.inner.paths.account);
        let _ = std::fs::remove_file(&self.inner.paths.token);
        self.reconnect();
        self.changed();
        Ok(())
    }

    /// 換憑證後重新連中轉站：丟掉送訊通道，連線迴圈會結束這一輪、用新憑證重連。
    fn reconnect(&self) {
        *self.inner.outbox.lock().expect("outbox") = None;
        self.inner.wake.notify_one();
    }

    /// 等一段時間再重連；換了憑證就立刻醒來。
    async fn pause(&self, secs: u64) {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {}
            _ = self.inner.wake.notified() => {}
        }
    }

    // ------------------------------------------------------------------ 中轉 AI（PRD §30.7；CONTRACT §17）

    /// 換掉跑 AI 的方式（測試用假的）。
    pub fn set_runner(&self, runner: Arc<dyn AiRunner>) {
        self.inner.ai.lock().expect("ai state").runner = runner;
    }

    /// 把要送出的訊框接到這個通道（連線時由 `run` 接上；測試直接接）。
    pub fn attach_outbox(&self, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        *self.inner.outbox.lock().expect("outbox") = Some(tx);
    }

    fn relay_agent(&self) -> RelayAgent {
        RelayAgent::parse(&self.inner.state.lock().expect("mobile state").config.relay_agent).unwrap_or(RelayAgent::Claude)
    }

    fn runner(&self) -> Arc<dyn AiRunner> {
        self.inner.ai.lock().expect("ai state").runner.clone()
    }

    /// 背景跑完後把結果送給那支手機。
    fn send_later(&self, device: &str, body: Value) {
        if let Some(f) = self.seal_to(device, &body, chrono::Utc::now().timestamp()) {
            self.send_all(&[f]);
        }
    }

    fn scratch_file() -> PathBuf {
        let mut n = [0u8; 8];
        OsRng.fill_bytes(&mut n);
        std::env::temp_dir().join(format!("kunsu-relay-{}.txt", n.iter().map(|b| format!("{b:02x}")).collect::<String>()))
    }

    fn work_dir(cwd: &str) -> PathBuf {
        let p = PathBuf::from(cwd);
        if !cwd.is_empty() && p.is_dir() {
            p
        } else {
            std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
        }
    }

    /// 追問軍師：在背景跑唯讀 AI，回 `followup_answer {by: relay}`（AC-RA-01、02）。
    fn followup(&self, device: &str, id: String, question_id: String, kind: String, text: Option<String>) {
        let me = self.clone();
        let device = device.to_string();
        tokio::spawn(async move {
            let q = match me.inner.handle.get_question(&question_id).await {
                Ok(Some(q)) => q,
                _ => {
                    me.send_later(&device, json!({"type": "followup_answer", "id": id, "ok": false, "by": "relay", "error": "找不到這題（可能已經在別處回答）"}));
                    return;
                }
            };
            let earlier = me.inner.ai.lock().expect("ai state").asked.get(&question_id).cloned().unwrap_or_default();
            let prompt = relay_ai::followup_prompt(&q, &kind, text.as_deref(), &earlier);
            let plan = relay_ai::relay_command(
                me.relay_agent(),
                &Self::work_dir(&q.source.cwd),
                &prompt,
                &crate::reactor::default_claude_bin(),
                &crate::reactor::default_codex_bin(),
                &Self::scratch_file(),
            );
            let body = match me.runner().run(plan, relay_ai::RELAY_TIMEOUT).await {
                Ok(answer) => {
                    let ask = text.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| match kind.as_str() {
                        "why" => "為什麼？".into(),
                        "context" => "前因後果？".into(),
                        _ => "請幫我看懂這題".into(),
                    });
                    me.inner.ai.lock().expect("ai state").asked.entry(question_id.clone()).or_default().push((ask, answer.clone()));
                    json!({"type": "followup_answer", "id": id, "ok": true, "by": "relay", "text": answer})
                }
                Err(e) => json!({"type": "followup_answer", "id": id, "ok": false, "by": "relay", "error": e}),
            };
            me.send_later(&device, body);
        });
    }

    /// 問本人：原 AI 在等才收下（AC-RA-03、04）。回 None＝收下了，回覆稍後由事件送出。
    async fn forward(&self, device: &str, id: &str, question_id: &str, text: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Value> {
        let refuse = |e: String| Some(json!({"type": "followup_answer", "id": id, "ok": false, "by": "agent", "error": e}));
        let q = match self.inner.handle.get_question(question_id).await {
            Ok(Some(q)) if !q.state.is_terminal() => q,
            _ => return refuse("這題已經結束，不能再問本人".into()),
        };
        let (live, lease) = self.inner.handle.waiter_status(question_id, now);
        if !live && !lease {
            return refuse(format!("{} 現在沒在等這題，只能問軍師", relay_ai::project_name(&q)));
        }
        // 先訂閱再放進佇列：原 AI 回得再快也不會漏
        let mut rx = self.inner.handle.subscribe();
        self.inner.ai.lock().expect("ai state").forwards.insert(id.to_string(), device.to_string());
        if let Err(e) = self.inner.handle.followup_ask(question_id, id, text, now) {
            self.inner.ai.lock().expect("ai state").forwards.remove(id);
            return refuse(e.to_string());
        }
        let me = self.clone();
        let fid = id.to_string();
        tokio::spawn(async move {
            let limit = std::time::Duration::from_secs(60 * (FORWARD_TIMEOUT_MIN as u64 + 1));
            let _ = tokio::time::timeout(limit, async {
                loop {
                    match rx.recv().await {
                        Ok(crate::events::ServerEvent::Followup { followup_id, reply: Some(text), .. }) if followup_id == fid => {
                            me.deliver_forward_reply(&fid, &text);
                            return;
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                        _ => {
                            // 已逾時（expire_followups 拿掉了）就不用再等
                            if !me.inner.ai.lock().expect("ai state").forwards.contains_key(&fid) {
                                return;
                            }
                        }
                    }
                }
            })
            .await;
        });
        None
    }

    /// 原 AI 回了追問 → 送給問的那支手機。
    fn deliver_forward_reply(&self, followup_id: &str, text: &str) {
        let device = self.inner.ai.lock().expect("ai state").forwards.remove(followup_id);
        if let Some(device) = device {
            self.send_later(&device, json!({"type": "followup_answer", "id": followup_id, "ok": true, "by": "agent", "text": text}));
        }
    }

    /// 問本人 15 分鐘沒回：從佇列拿掉並告訴手機。回要送出的訊框。
    pub fn expire_followups(&self, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
        let expired = self.inner.handle.followup_expire(now, chrono::Duration::minutes(FORWARD_TIMEOUT_MIN));
        let mut out = vec![];
        for (_, fid) in expired {
            let device = self.inner.ai.lock().expect("ai state").forwards.remove(&fid);
            if let Some(device) = device {
                let body = json!({"type": "followup_answer", "id": fid, "ok": false, "by": "agent", "error": format!("原 AI {FORWARD_TIMEOUT_MIN} 分鐘內沒回，可以改問軍師")});
                out.extend(self.seal_to(&device, &body, now.timestamp()));
            }
        }
        out
    }

    /// 答完後跟軍師聊天（AC-RA-06）。
    fn chat(&self, device: &str, id: String, text: String) {
        let history: Vec<(String, String)> = {
            let mut ai = self.inner.ai.lock().expect("ai state");
            let h = ai.chat.entry(device.to_string()).or_default();
            h.push_back(("使用者".into(), text.clone()));
            while h.len() > relay_ai::CHAT_MEMORY {
                h.pop_front();
            }
            h.iter().take(h.len() - 1).cloned().collect()
        };
        let me = self.clone();
        let device = device.to_string();
        tokio::spawn(async move {
            let open = me.inner.handle.list_questions(false).await.unwrap_or_default();
            let open: Vec<Question> = open.into_iter().filter(|q| !q.internal && !q.state.is_terminal()).collect();
            let all = me.inner.handle.list_questions(true).await.unwrap_or_default();
            let since = chrono::Utc::now() - chrono::Duration::hours(48);
            let recent: Vec<Question> = all
                .into_iter()
                .filter(|q| !q.internal && q.answer.as_ref().is_some_and(|a| a.answered_at >= since))
                .collect();
            let prompt = relay_ai::chat_prompt(&relay_ai::state_summary(&open, &recent), &history, &text);
            let home = Self::work_dir("");
            let plan = relay_ai::relay_command(
                me.relay_agent(),
                &home,
                &prompt,
                &crate::reactor::default_claude_bin(),
                &crate::reactor::default_codex_bin(),
                &Self::scratch_file(),
            );
            let body = match me.runner().run(plan, relay_ai::RELAY_TIMEOUT).await {
                Ok(reply) => {
                    let mut ai = me.inner.ai.lock().expect("ai state");
                    let h = ai.chat.entry(device.clone()).or_default();
                    h.push_back(("軍師".into(), reply.clone()));
                    while h.len() > relay_ai::CHAT_MEMORY {
                        h.pop_front();
                    }
                    json!({"type": "chat_reply", "id": id, "ok": true, "text": reply})
                }
                Err(e) => json!({"type": "chat_reply", "id": id, "ok": false, "error": e}),
            };
            me.send_later(&device, body);
        });
    }

    /// 快照裡的專案：題目來源資料夾，最近的在前，最多 12 個（名稱 → 路徑）。
    async fn projects(&self) -> Vec<(String, PathBuf)> {
        let mut all = self.inner.handle.list_questions(true).await.unwrap_or_default();
        all.sort_by_key(|q| std::cmp::Reverse(q.created_at));
        let mut out: Vec<(String, PathBuf)> = vec![];
        for q in all {
            let cwd = PathBuf::from(&q.source.cwd);
            if q.source.cwd.is_empty() || !cwd.is_dir() {
                continue;
            }
            let name = relay_ai::project_name(&q);
            if out.iter().any(|(n, _)| *n == name) {
                continue;
            }
            out.push((name, cwd));
            if out.len() == 12 {
                break;
            }
        }
        out
    }

    /// 從手機開新任務（AC-RA-07）：只在快照裡的專案資料夾、Auto 權限。
    async fn start_task(&self, device: &str, id: String, agent: &str, project: &str, text: String) -> Value {
        let fail = |e: &str| json!({"type": "task_started", "id": id, "ok": false, "error": e, "agent": agent, "project": project});
        let Some(agent_kind) = RelayAgent::parse(agent) else { return fail("只能選 Claude 或 Codex") };
        if text.trim().is_empty() {
            return fail("要做什麼是空的");
        }
        let Some((_, cwd)) = self.projects().await.into_iter().find(|(n, _)| n == project) else {
            return fail("Mac 上找不到這個專案");
        };
        let plan = relay_ai::task_command(
            agent_kind,
            &cwd,
            &text,
            &crate::reactor::default_claude_bin(),
            &crate::reactor::default_codex_bin(),
            &Self::scratch_file(),
        );
        let me = self.clone();
        let device_id = device.to_string();
        let task_id = id.clone();
        tokio::spawn(async move {
            let body = match me.runner().run(plan, relay_ai::TASK_TIMEOUT).await {
                Ok(out) => {
                    let chars: Vec<char> = out.chars().collect();
                    let tail: String = chars[chars.len().saturating_sub(1200)..].iter().collect();
                    json!({"type": "task_done", "id": task_id, "ok": true, "text": tail})
                }
                Err(e) => json!({"type": "task_done", "id": task_id, "ok": false, "text": e}),
            };
            me.send_later(&device_id, body);
        });
        json!({"type": "task_started", "id": id, "ok": true, "agent": agent, "project": project})
    }

    // ------------------------------------------------------------------ 指令

    #[allow(clippy::too_many_arguments)]
    async fn answer(
        &self,
        question_id: &str,
        choice_id: Option<String>,
        text: Option<String>,
        money_confirmed: bool,
        revision: Option<u32>,
        content_hash: Option<String>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Value {
        let fail = |e: String| json!({"type": "ack", "for": "answer", "question_id": question_id, "ok": false, "error": e});
        let q = match self.inner.handle.get_question(question_id).await {
            Ok(Some(q)) => q,
            Ok(None) => return fail("找不到這題（可能已經在別處回答）".into()),
            Err(e) => return fail(e.to_string()),
        };
        if q.kind == Kind::Batch {
            return fail("題組請在電腦上逐項處理".into());
        }
        if q.is_money_op() && !money_confirmed {
            return fail("動錢的題目要先在手機的核對畫面按「批准並執行」".into());
        }
        let signature = match (&choice_id, q.is_money_op()) {
            (Some(cid), true) => {
                let params = q.params_line.clone().unwrap_or_default();
                self.inner.key.hmac_key().and_then(|k| crate::approval::ui_answer_signature(&k, question_id, cid, &params)).ok()
            }
            _ => None,
        };
        let input = AnswerInput {
            choice_id,
            free_text: text.filter(|t| !t.trim().is_empty()),
            edited_params: None,
            channel: Channel::Mobile,
            signature,
            revision,
            content_hash,
            items: None,
        };
        match self.inner.handle.answer_question(question_id, input, now).await {
            Ok(_) => json!({"type": "ack", "for": "answer", "question_id": question_id, "ok": true}),
            Err(e) => fail(e.to_string()),
        }
    }

    async fn snapshot(&self, now: chrono::DateTime<chrono::Utc>) -> Value {
        let open = self.inner.handle.list_questions(false).await.unwrap_or_default();
        let questions: Vec<Value> = open.iter().filter(|q| !q.internal && !q.state.is_terminal()).map(question_dto).collect();
        let all = self.inner.handle.list_questions(true).await.unwrap_or_default();
        let since = now - chrono::Duration::hours(48);
        let mut answered: Vec<&Question> =
            all.iter().filter(|q| !q.internal && q.state == State::Answered && q.answer.as_ref().is_some_and(|a| a.answered_at >= since)).collect();
        answered.sort_by_key(|q| q.answer.as_ref().map(|a| a.answered_at));
        let history: Vec<Value> = answered.iter().rev().take(40).map(|q| history_dto(q, now)).collect();
        let relay_agent = self.inner.state.lock().expect("mobile state").config.relay_agent.clone();
        let projects: Vec<String> = self.projects().await.into_iter().map(|(n, _)| n).collect();
        json!({"type": "snapshot", "questions": questions, "history": history, "relay_agent": relay_agent, "projects": projects})
    }

    // ------------------------------------------------------------------ 連線

    fn send_all(&self, frames: &[String]) {
        if let Some(tx) = self.inner.outbox.lock().expect("outbox").as_ref() {
            for f in frames {
                let _ = tx.send(f.clone());
            }
        }
    }

    /// 題目有變化時推一份新的快照給所有已配對、未撤銷的裝置。
    async fn push_snapshots(&self) {
        let now = chrono::Utc::now();
        let snap = self.snapshot(now).await;
        let ids: Vec<String> = self.inner.state.lock().expect("mobile state").registry.devices.iter().filter(|d| !d.revoked).map(|d| d.id.clone()).collect();
        let frames: Vec<String> = ids.iter().filter_map(|id| self.seal_to(id, &snap, now.timestamp())).collect();
        self.send_all(&frames);
    }

    /// 背景常駐：連中轉站（斷線退避重連）、題目變化時推快照。
    pub fn spawn(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.run().await });
        self.spawn_expiry();
        let me = self.clone();
        tokio::spawn(async move {
            let mut rx = me.inner.handle.subscribe();
            loop {
                match rx.recv().await {
                    // 追問的事件由每則問本人自己等（forward），不推快照
                    Ok(crate::events::ServerEvent::Followup { .. }) => {}
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        // 一陣變化只推一次
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        while rx.try_recv().is_ok() {}
                        me.push_snapshots().await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 每 30 秒清一次逾時的問本人。
    fn spawn_expiry(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let frames = me.expire_followups(chrono::Utc::now());
                me.send_all(&frames);
            }
        });
    }

    fn set_conn(&self, connected: bool, err: Option<String>) {
        let mut s = self.inner.state.lock().expect("mobile state");
        let changed = s.connected != connected || s.last_error != err;
        s.connected = connected;
        s.last_error = err;
        drop(s);
        if changed {
            self.changed();
        }
    }

    async fn run(&self) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
        let mut backoff = 1u64;
        loop {
            let token = std::fs::read_to_string(&self.inner.paths.token).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            let Some(token) = token else {
                self.set_conn(false, Some(format!("沒有中轉站 token（{}）", self.inner.paths.token.display())));
                self.pause(30).await;
                continue;
            };
            let url = format!("{}/connect?role=mac&mac={}", self.inner.state.lock().expect("mobile state").config.relay_url, self.inner.keys.mac_id());
            let mut req = match url.as_str().into_client_request() {
                Ok(r) => r,
                Err(e) => {
                    self.set_conn(false, Some(format!("中轉站網址不對：{e}")));
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }
            };
            if let Ok(h) = format!("Bearer {token}").parse() {
                req.headers_mut().insert("authorization", h);
            }
            match tokio_tungstenite::connect_async(req).await {
                Ok((ws, _)) => {
                    backoff = 1;
                    self.set_conn(true, None);
                    let (mut sink, mut stream) = ws.split();
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                    *self.inner.outbox.lock().expect("outbox") = Some(tx);
                    loop {
                        tokio::select! {
                            out = rx.recv() => match out {
                                Some(text) => if sink.send(Message::Text(text)).await.is_err() { break },
                                None => break,
                            },
                            msg = stream.next() => match msg {
                                Some(Ok(Message::Text(text))) => {
                                    for f in self.handle_relay_message(&text, chrono::Utc::now()).await {
                                        if sink.send(Message::Text(f)).await.is_err() { break }
                                    }
                                }
                                Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; }
                                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                                _ => {}
                            },
                        }
                    }
                    *self.inner.outbox.lock().expect("outbox") = None;
                    self.set_conn(false, Some("和中轉站的連線中斷，正在重連".into()));
                }
                Err(e) => self.set_conn(false, Some(format!("連不上中轉站：{e}"))),
            }
            self.pause(backoff).await;
            backoff = (backoff * 2).min(60);
        }
    }
}

// ---------------------------------------------------------------------- 給手機看的題目

fn agent_name(a: AgentKind) -> &'static str {
    match a {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Other => "other",
    }
}

fn project_of(q: &Question) -> String {
    Path::new(&q.source.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| q.product.clone())
        .unwrap_or_else(|| "Kunsu".into())
}

fn first_paragraph(md: &str) -> String {
    md.split("\n\n").map(str::trim).find(|p| !p.is_empty()).unwrap_or("").chars().take(400).collect()
}

fn question_dto(q: &Question) -> Value {
    let kind = match q.kind {
        Kind::Choice => "choice",
        Kind::Text => "text",
        Kind::Confirm => "confirm",
        Kind::KongkoReview => "kongko_review",
        Kind::Batch => "batch",
    };
    let money = q.money.as_ref().filter(|m| m.is_money_op).map(|m| {
        let mut rows = vec![];
        if let Some(a) = m.amount_twd {
            rows.push(json!({"label": "金額", "value": format!("NT${a}")}));
        }
        if let Some(e) = &m.entity_id {
            rows.push(json!({"label": "對象", "value": e}));
        }
        if let Some(p) = &q.params_line {
            rows.push(json!({"label": "參數", "value": p}));
        }
        rows
    });
    json!({
        "id": q.id,
        "agent": agent_name(q.source.agent),
        "project": project_of(q),
        "question": if q.question_line.trim().is_empty() { q.title.clone() } else { q.question_line.clone() },
        "why": q.why_md.clone().filter(|w| !w.trim().is_empty()).unwrap_or_else(|| first_paragraph(&q.body_md)),
        "context": q.evidence.iter().map(|e| e.text.clone()).collect::<Vec<_>>(),
        "kind": kind,
        "options": q.options.iter().map(|o| json!({"id": o.id, "label": o.label, "detail": o.detail, "recommended": o.recommended == Some(true)})).collect::<Vec<_>>(),
        "allow_text": q.allow_free_text || q.kind == Kind::Text,
        "money": money,
        "asked_at": q.created_at.timestamp(),
        "revision": q.draft.as_ref().map(|d| d.revision),
        "content_hash": q.draft.as_ref().map(|d| d.content_hash.clone()),
        "has_conversation": q.source.conversation_id.is_some(),
    })
}

fn history_dto(q: &Question, now: chrono::DateTime<chrono::Utc>) -> Value {
    let a = q.answer.as_ref().expect("answered");
    let label = a
        .choice_id
        .as_ref()
        .and_then(|c| q.options.iter().find(|o| &o.id == c).map(|o| o.label.clone()))
        .or_else(|| a.free_text.clone())
        .unwrap_or_else(|| "已回答".into());
    let local = Local.from_utc_datetime(&a.answered_at.naive_utc());
    let today = Local.from_utc_datetime(&now.naive_utc()).date_naive();
    let day = match (today - local.date_naive()).num_days() {
        0 => "今天".to_string(),
        1 => "昨天".to_string(),
        _ => local.format("%-m 月 %-d 日").to_string(),
    };
    let result = a
        .execution_result
        .as_ref()
        .map(|r| if crate::reactor::is_money_result(r) { "錢鏈已處理，以 Kunsu 回執為準".to_string() } else { r.chars().take(160).collect() })
        .unwrap_or_else(|| "已交給 AI".into());
    json!({
        "id": q.id,
        "day": day,
        "time": local.format("%H:%M").to_string(),
        "agent": agent_name(q.source.agent),
        "project": project_of(q),
        "question": if q.question_line.trim().is_empty() { q.title.clone() } else { q.question_line.clone() },
        "answer": label,
        "result": result,
        "channel": format!("{:?}", a.channel).to_lowercase(),
    })
}

/// 這台 Mac 在帳號裡顯示的名稱。
fn mac_name() -> String {
    std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Mac".into())
}
