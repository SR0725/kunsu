//! 手機配對與 Zeabur 中轉協定（PRD §30.8；docs/design/mobile-relay.md；INVARIANTS 28、29）。
//!
//! 純函數：時間（unix 秒）與亂數由呼叫端傳入，不讀系統時鐘、不做 I/O。
//! Mac 端（kunsu-server 的手機橋接）與測試用的裝置端都用這一份；iOS 端以 CryptoKit 實作同一套位元格式
//! （互通測試向量見 `docs/design/mobile-relay-vector.json`）。

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use hkdf::Hkdf;
use rand_core::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// 配對碼有效時間（秒）。
pub const CODE_TTL_SECS: i64 = 300;
/// 手機封包允許的時間差（秒）。
pub const CLOCK_SKEW_SECS: i64 = 120;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

pub mod b64 {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    pub fn encode(bytes: &[u8]) -> String { URL_SAFE_NO_PAD.encode(bytes) }
    pub fn decode(s: &str) -> Option<Vec<u8>> { URL_SAFE_NO_PAD.decode(s).ok() }
    pub fn decode32(s: &str) -> Option<[u8; 32]> { decode(s)?.try_into().ok() }
}

/// sha256 → base32 小寫取 16 字：`mac_id`、`device_id`。
pub fn short_id(public_key: &[u8]) -> String {
    let digest = Sha256::digest(public_key);
    data_encoding::BASE32_NOPAD.encode(&digest).to_lowercase()[..16].to_string()
}

fn hkdf32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid HKDF length");
    out
}

fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], msg: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(nonce), Payload { msg, aad })
        .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
}

fn open(key: &[u8; 32], nonce: &[u8], aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    if nonce.len() != 12 {
        return None;
    }
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad }).ok()
}

// ---------------------------------------------------------------- 金鑰

/// Mac 的長期金鑰：X25519（交換）與 Ed25519（簽章）。存在 `~/.kunsu/mobile-keys.json`（0600），不用鑰匙圈（2026-10-12 作者）。
pub struct MacKeys {
    kx: x25519_dalek::StaticSecret,
    sig: ed25519_dalek::SigningKey,
}

#[derive(Serialize, Deserialize)]
struct MacKeysFile {
    kx: String,
    sig: String,
}

impl MacKeys {
    pub fn from_seed(kx: [u8; 32], sig: [u8; 32]) -> Self {
        Self { kx: x25519_dalek::StaticSecret::from(kx), sig: ed25519_dalek::SigningKey::from_bytes(&sig) }
    }
    pub fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut kx = [0u8; 32];
        let mut sig = [0u8; 32];
        rng.fill_bytes(&mut kx);
        rng.fill_bytes(&mut sig);
        Self::from_seed(kx, sig)
    }
    pub fn kx_public(&self) -> [u8; 32] { x25519_dalek::PublicKey::from(&self.kx).to_bytes() }
    pub fn sig_public(&self) -> [u8; 32] { self.sig.verifying_key().to_bytes() }
    pub fn mac_id(&self) -> String { short_id(&self.sig_public()) }
    pub fn to_json(&self) -> String {
        serde_json::to_string(&MacKeysFile { kx: b64::encode(self.kx.as_bytes()), sig: b64::encode(&self.sig.to_bytes()) })
            .expect("serializable")
    }
    pub fn from_json(s: &str) -> Option<Self> {
        let f: MacKeysFile = serde_json::from_str(s).ok()?;
        Some(Self::from_seed(b64::decode32(&f.kx)?, b64::decode32(&f.sig)?))
    }
    fn sign(&self, msg: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.sig.sign(msg).to_bytes()
    }
    fn shared(&self, peer_kx: &[u8; 32]) -> [u8; 32] {
        self.kx.diffie_hellman(&x25519_dalek::PublicKey::from(*peer_kx)).to_bytes()
    }
}

/// 裝置端金鑰（iOS 用 CryptoKit；這裡給測試與 Rust 測試客戶端用）：X25519 交換、P-256 簽章。
pub struct DeviceKeys {
    kx: x25519_dalek::StaticSecret,
    sig: p256::ecdsa::SigningKey,
}

impl DeviceKeys {
    pub fn from_seed(kx: [u8; 32], sig: [u8; 32]) -> Self {
        Self {
            kx: x25519_dalek::StaticSecret::from(kx),
            sig: p256::ecdsa::SigningKey::from_slice(&sig).expect("seed is a valid P-256 scalar"),
        }
    }
    pub fn kx_public(&self) -> [u8; 32] { x25519_dalek::PublicKey::from(&self.kx).to_bytes() }
    /// SEC1 未壓縮（65 bytes），等於 CryptoKit 的 `x963Representation`。
    pub fn sig_public(&self) -> Vec<u8> { self.sig.verifying_key().to_encoded_point(false).as_bytes().to_vec() }
    pub fn device_id(&self) -> String { short_id(&self.sig_public()) }

    /// 固定臨時金鑰與 nonce 的配對請求（互通測試向量用）。
    #[allow(clippy::too_many_arguments)]
    pub fn pair_request_with_eph(&self, eph: [u8; 32], nonce: [u8; 12], mac_kx_public: &[u8; 32], mac_id: &str, code: &str, device_name: &str, ts: i64) -> PairRequestWire {
        struct Fixed([u8; 32], [u8; 12], u8);
        impl RngCore for Fixed {
            fn next_u32(&mut self) -> u32 { 0 }
            fn next_u64(&mut self) -> u64 { 0 }
            fn fill_bytes(&mut self, dest: &mut [u8]) {
                if self.2 == 0 { dest.copy_from_slice(&self.0[..dest.len()]) } else { dest.copy_from_slice(&self.1[..dest.len()]) }
                self.2 += 1;
            }
            fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> { self.fill_bytes(dest); Ok(()) }
        }
        impl CryptoRng for Fixed {}
        self.pair_request(mac_kx_public, mac_id, code, device_name, ts, &mut Fixed(eph, nonce, 0))
    }

    /// 配對請求：以臨時 X25519 對 Mac 公鑰做 ECIES。
    pub fn pair_request<R: RngCore + CryptoRng>(
        &self,
        mac_kx_public: &[u8; 32],
        mac_id: &str,
        code: &str,
        device_name: &str,
        ts: i64,
        rng: &mut R,
    ) -> PairRequestWire {
        let eph = x25519_dalek::StaticSecret::random_from_rng(&mut *rng);
        let eph_pub = x25519_dalek::PublicKey::from(&eph).to_bytes();
        let shared = eph.diffie_hellman(&x25519_dalek::PublicKey::from(*mac_kx_public)).to_bytes();
        let key = hkdf32(&shared, b"kunsu-pair-v1", mac_id.as_bytes());
        let mut nonce = [0u8; 12];
        rng.fill_bytes(&mut nonce);
        let body = PairRequestBody {
            code: code.to_string(),
            device_name: device_name.to_string(),
            kx: b64::encode(&self.kx_public()),
            sig: b64::encode(&self.sig_public()),
            ts,
        };
        let json = serde_json::to_vec(&body).expect("serializable");
        let ct = seal(&key, &nonce, pair_aad(mac_id).as_bytes(), &json);
        PairRequestWire { t: "pair_request".into(), eph: b64::encode(&eph_pub), nonce: b64::encode(&nonce), ct: b64::encode(&ct) }
    }

    pub fn verify_pair_accepted(&self, mac_sig_public: &[u8; 32], a: &PairAccepted) -> bool {
        a.dev == self.device_id() && verify_ed25519(mac_sig_public, pair_accepted_input(&a.mac, &a.dev, a.ts).as_bytes(), &a.sig)
    }

    /// 裝置 → Mac 封包。
    pub fn seal(&self, mac_kx_public: &[u8; 32], seq: u64, ts: i64, plaintext: &[u8], nonce: [u8; 12]) -> Envelope {
        use p256::ecdsa::signature::Signer;
        let dev = self.device_id();
        let shared = self.kx.diffie_hellman(&x25519_dalek::PublicKey::from(*mac_kx_public)).to_bytes();
        let key = session_key(&shared, &dev, Direction::DeviceToMac);
        let ct = seal(&key, &nonce, envelope_aad(&dev, seq, ts).as_bytes(), plaintext);
        let mut env = Envelope { v: 1, dev, seq, ts, nonce: b64::encode(&nonce), ct: b64::encode(&ct), sig: String::new() };
        let sig: p256::ecdsa::Signature = self.sig.sign(signing_input(&env).as_bytes());
        env.sig = b64::encode(&sig.to_bytes());
        env
    }

    /// Mac → 裝置封包：驗 Mac 簽章後解開。
    pub fn open_from_mac(&self, mac_kx_public: &[u8; 32], mac_sig_public: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, Reject> {
        if env.dev != self.device_id() {
            return Err(Reject::UnknownDevice);
        }
        if !verify_ed25519(mac_sig_public, signing_input(env).as_bytes(), &env.sig) {
            return Err(Reject::BadSignature);
        }
        let shared = self.kx.diffie_hellman(&x25519_dalek::PublicKey::from(*mac_kx_public)).to_bytes();
        let key = session_key(&shared, &env.dev, Direction::MacToDevice);
        let (nonce, ct) = (b64::decode(&env.nonce).ok_or(Reject::Decrypt)?, b64::decode(&env.ct).ok_or(Reject::Decrypt)?);
        open(&key, &nonce, envelope_aad(&env.dev, env.seq, env.ts).as_bytes(), &ct).ok_or(Reject::Decrypt)
    }
}

fn verify_ed25519(public: &[u8; 32], msg: &[u8], sig_b64: &str) -> bool {
    use ed25519_dalek::Verifier;
    let Some(sig) = b64::decode(sig_b64).and_then(|b| <[u8; 64]>::try_from(b).ok()) else { return false };
    let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(public) else { return false };
    key.verify(msg, &ed25519_dalek::Signature::from_bytes(&sig)).is_ok()
}

fn verify_p256(public_sec1: &[u8], msg: &[u8], sig_b64: &str) -> bool {
    use p256::ecdsa::signature::Verifier;
    let Some(sig) = b64::decode(sig_b64).and_then(|b| p256::ecdsa::Signature::from_slice(&b).ok()) else { return false };
    let Ok(key) = p256::ecdsa::VerifyingKey::from_sec1_bytes(public_sec1) else { return false };
    key.verify(msg, &sig).is_ok()
}

// ---------------------------------------------------------------- 配對

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequestWire {
    pub t: String,
    pub eph: String,
    pub nonce: String,
    pub ct: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairRequestBody {
    code: String,
    device_name: String,
    kx: String,
    sig: String,
    ts: i64,
}

fn pair_aad(mac_id: &str) -> String { format!("pair_request|{mac_id}") }
fn pair_accepted_input(mac: &str, dev: &str, ts: i64) -> String { format!("pair_accepted|{mac}|{dev}|{ts}") }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAccepted {
    pub t: String,
    pub mac: String,
    pub dev: String,
    pub ts: i64,
    pub sig: String,
}

pub fn pair_accepted(mac: &MacKeys, device: &Device, ts: i64) -> PairAccepted {
    let input = pair_accepted_input(&mac.mac_id(), &device.id, ts);
    PairAccepted { t: "pair_accepted".into(), mac: mac.mac_id(), dev: device.id.clone(), ts, sig: b64::encode(&mac.sign(input.as_bytes())) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("配對碼不對、已過期或已經用過")]
    CodeInvalid,
    #[error("配對請求打不開（不是給這台 Mac 的）")]
    Undecryptable,
    #[error("配對請求格式不對")]
    BadRequest,
    #[error("找不到這個待允許的配對")]
    NoSuchRequest,
}

/// 等使用者在主視窗按允許的配對請求（INV-29）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingPair {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub kx: String,
    pub sig: String,
    pub received_at: i64,
    #[serde(skip)]
    expires_at: i64,
}

#[derive(Default)]
pub struct PairingBook {
    codes: Vec<(String, i64)>,
    pending: Vec<PendingPair>,
}

impl PairingBook {
    /// 產生一次性配對碼（舊的未用碼同時作廢：一次只有一個 QR 有效）。
    pub fn issue<R: RngCore + CryptoRng>(&mut self, now: i64, rng: &mut R) -> String {
        let code: String = (0..8).map(|_| CODE_ALPHABET[(rng.next_u32() as usize) % CODE_ALPHABET.len()] as char).collect();
        self.codes = vec![(code.clone(), now + CODE_TTL_SECS)];
        code
    }

    pub fn pending(&self) -> &[PendingPair] { &self.pending }

    /// 收到配對請求：配對碼正確才進入待允許；碼用過即作廢。
    pub fn receive(&mut self, mac: &MacKeys, wire: &PairRequestWire, now: i64) -> Result<PendingPair, PairError> {
        let eph = b64::decode32(&wire.eph).ok_or(PairError::BadRequest)?;
        let nonce = b64::decode(&wire.nonce).ok_or(PairError::BadRequest)?;
        let ct = b64::decode(&wire.ct).ok_or(PairError::BadRequest)?;
        let key = hkdf32(&mac.shared(&eph), b"kunsu-pair-v1", mac.mac_id().as_bytes());
        let json = open(&key, &nonce, pair_aad(&mac.mac_id()).as_bytes(), &ct).ok_or(PairError::Undecryptable)?;
        let body: PairRequestBody = serde_json::from_slice(&json).map_err(|_| PairError::BadRequest)?;
        let idx = self
            .codes
            .iter()
            .position(|(c, exp)| bool::from(c.as_bytes().ct_eq(body.code.as_bytes())) && now <= *exp)
            .ok_or(PairError::CodeInvalid)?;
        let (_, expires_at) = self.codes.remove(idx);
        let sig = b64::decode(&body.sig).ok_or(PairError::BadRequest)?;
        p256::ecdsa::VerifyingKey::from_sec1_bytes(&sig).map_err(|_| PairError::BadRequest)?;
        b64::decode32(&body.kx).ok_or(PairError::BadRequest)?;
        let name: String = body.device_name.trim().chars().take(40).collect();
        let pending = PendingPair {
            id: format!("{}-{now}", short_id(&sig)),
            device_id: short_id(&sig),
            device_name: if name.is_empty() { "手機".into() } else { name },
            kx: body.kx,
            sig: body.sig,
            received_at: now,
            expires_at,
        };
        self.pending.retain(|p| p.device_id != pending.device_id);
        self.pending.push(pending.clone());
        Ok(pending)
    }

    /// 使用者在主視窗按允許：產生裝置。超過配對碼期限就不能允許。
    pub fn approve(&mut self, id: &str, now: i64) -> Result<Device, PairError> {
        let idx = self.pending.iter().position(|p| p.id == id).ok_or(PairError::NoSuchRequest)?;
        let p = self.pending.remove(idx);
        if now > p.expires_at {
            return Err(PairError::NoSuchRequest);
        }
        Ok(Device { id: p.device_id, name: p.device_name, kx: p.kx, sig: p.sig, paired_at: now, last_seq: 0, revoked: false, last_seen: None })
    }

    pub fn reject(&mut self, id: &str) { self.pending.retain(|p| p.id != id); }

    /// 過期的待允許請求自動消失。
    pub fn prune(&mut self, now: i64) {
        self.pending.retain(|p| now <= p.expires_at);
        self.codes.retain(|(_, exp)| now <= *exp);
    }
}

// ---------------------------------------------------------------- QR

pub struct QrPayload {
    pub relay: String,
    pub mac: String,
    pub code: String,
    pub kx: [u8; 32],
    pub sig: [u8; 32],
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn pct_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn qr_payload(relay: &str, mac: &MacKeys, code: &str) -> String {
    format!(
        "kunsu-pair:v1?relay={}&mac={}&code={}&kx={}&sig={}",
        pct_encode(relay),
        mac.mac_id(),
        code,
        b64::encode(&mac.kx_public()),
        b64::encode(&mac.sig_public())
    )
}

pub fn parse_qr_payload(s: &str) -> Option<QrPayload> {
    let query = s.strip_prefix("kunsu-pair:v1?")?;
    let get = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(str::to_string));
    Some(QrPayload {
        relay: pct_decode(&get("relay")?)?,
        mac: get("mac")?,
        code: get("code")?,
        kx: b64::decode32(&get("kx")?)?,
        sig: b64::decode32(&get("sig")?)?,
    })
}

pub fn qr_svg(payload: &str) -> String {
    match qrcode::QrCode::new(payload.as_bytes()) {
        Ok(code) => code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build(),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------- 裝置清單與封包

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// X25519 公鑰（b64url）
    pub kx: String,
    /// P-256 公鑰 SEC1 未壓縮（b64url）
    pub sig: String,
    pub paired_at: i64,
    pub last_seq: u64,
    pub revoked: bool,
    #[serde(default)]
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub devices: Vec<Device>,
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&Device> { self.devices.iter().find(|d| d.id == id) }
    pub fn upsert(&mut self, device: Device) {
        self.devices.retain(|d| d.id != device.id);
        self.devices.push(device);
    }
    pub fn revoke(&mut self, id: &str) -> bool {
        let Some(d) = self.devices.iter_mut().find(|d| d.id == id) else { return false };
        d.revoked = true;
        true
    }
    pub fn to_json(&self) -> String { serde_json::to_string_pretty(self).expect("serializable") }
    pub fn from_json(s: &str) -> Option<Self> { serde_json::from_str(s).ok() }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    pub dev: String,
    pub seq: u64,
    pub ts: i64,
    pub nonce: String,
    pub ct: String,
    pub sig: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Reject {
    #[error("不認識的裝置")]
    UnknownDevice,
    #[error("裝置已撤銷")]
    Revoked,
    #[error("簽章不對")]
    BadSignature,
    #[error("重送的舊封包")]
    Replay,
    #[error("時間差超過 120 秒")]
    Clock,
    #[error("解不開")]
    Decrypt,
}

#[derive(Clone, Copy)]
enum Direction {
    DeviceToMac,
    MacToDevice,
}

fn session_key(shared: &[u8; 32], dev: &str, dir: Direction) -> [u8; 32] {
    let info: &[u8] = match dir {
        Direction::DeviceToMac => b"kunsu-relay-v1|d2m",
        Direction::MacToDevice => b"kunsu-relay-v1|m2d",
    };
    hkdf32(shared, dev.as_bytes(), info)
}

fn envelope_aad(dev: &str, seq: u64, ts: i64) -> String { format!("1|{dev}|{seq}|{ts}") }
pub fn signing_input(e: &Envelope) -> String { format!("1|{}|{}|{}|{}|{}", e.dev, e.seq, e.ts, e.nonce, e.ct) }

/// Mac 解開手機封包（INV-28）：裝置、撤銷、簽章、seq、時間、解密依序檢查；成功才推進 seq。
pub fn open_from_device(mac: &MacKeys, reg: &mut Registry, env: &Envelope, now: i64) -> Result<Vec<u8>, Reject> {
    let device = reg.get(&env.dev).ok_or(Reject::UnknownDevice)?.clone();
    if device.revoked {
        return Err(Reject::Revoked);
    }
    let sig_pub = b64::decode(&device.sig).ok_or(Reject::BadSignature)?;
    if env.v != 1 || !verify_p256(&sig_pub, signing_input(env).as_bytes(), &env.sig) {
        return Err(Reject::BadSignature);
    }
    if env.seq <= device.last_seq {
        return Err(Reject::Replay);
    }
    if (env.ts - now).abs() > CLOCK_SKEW_SECS {
        return Err(Reject::Clock);
    }
    let kx = b64::decode32(&device.kx).ok_or(Reject::Decrypt)?;
    let key = session_key(&mac.shared(&kx), &env.dev, Direction::DeviceToMac);
    let (nonce, ct) = (b64::decode(&env.nonce).ok_or(Reject::Decrypt)?, b64::decode(&env.ct).ok_or(Reject::Decrypt)?);
    let body = open(&key, &nonce, envelope_aad(&env.dev, env.seq, env.ts).as_bytes(), &ct).ok_or(Reject::Decrypt)?;
    if let Some(d) = reg.devices.iter_mut().find(|d| d.id == env.dev) {
        d.last_seq = env.seq;
        d.last_seen = Some(now);
    }
    Ok(body)
}

/// Mac → 手機封包：加密並以 Mac 的 Ed25519 簽章。
pub fn seal_from_mac(mac: &MacKeys, device: &Device, seq: u64, ts: i64, plaintext: &[u8], nonce: [u8; 12]) -> Option<Envelope> {
    let kx = b64::decode32(&device.kx)?;
    let key = session_key(&mac.shared(&kx), &device.id, Direction::MacToDevice);
    let ct = seal(&key, &nonce, envelope_aad(&device.id, seq, ts).as_bytes(), plaintext);
    let mut env = Envelope { v: 1, dev: device.id.clone(), seq, ts, nonce: b64::encode(&nonce), ct: b64::encode(&ct), sig: String::new() };
    env.sig = b64::encode(&mac.sign(signing_input(&env).as_bytes()));
    Some(env)
}
