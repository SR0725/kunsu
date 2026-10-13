//! 手機配對與中轉協定（BDD docs/bdd/mobile-pairing.md AC-MP-01～04；INVARIANTS 28、29）。

use kunsu_mobile::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const T0: i64 = 1_791_800_000;

fn rng() -> ChaCha20Rng { ChaCha20Rng::seed_from_u64(7) }
fn mac() -> MacKeys { MacKeys::from_seed([1; 32], [2; 32]) }
fn phone() -> DeviceKeys { DeviceKeys::from_seed([3; 32], [4; 32]) }

/// 配對完成：回傳 Mac 端的裝置清單與裝置。
fn paired() -> (Registry, Device) {
    let mac = mac();
    let mut book = PairingBook::default();
    let code = book.issue(T0, &mut rng());
    let wire = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "Alex 的 iPhone", T0, &mut rng());
    let pending = book.receive(&mac, &wire, T0 + 5).unwrap();
    let device = book.approve(&pending.id, T0 + 10).unwrap();
    let mut reg = Registry::default();
    reg.upsert(device.clone());
    (reg, device)
}

fn sealed(seq: u64, ts: i64, body: &str) -> Envelope {
    let mac = mac();
    phone().seal(&mac.kx_public(), seq, ts, body.as_bytes(), [9; 12])
}

// ---------------------------------------------------------------- AC-MP-01 配對碼

#[test]
fn pairing_code_expires_after_five_minutes_and_is_single_use() {
    let mac = mac();
    let mut book = PairingBook::default();
    let code = book.issue(T0, &mut rng());
    assert_eq!(code.len(), 8);
    let late = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "x", T0, &mut rng());
    assert_eq!(book.receive(&mac, &late, T0 + CODE_TTL_SECS + 1).unwrap_err(), PairError::CodeInvalid);

    let code = book.issue(T0, &mut rng());
    let first = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "x", T0, &mut rng());
    assert!(book.receive(&mac, &first, T0 + 1).is_ok());
    let other = DeviceKeys::from_seed([5; 32], [6; 32]);
    let second = other.pair_request(&mac.kx_public(), &mac.mac_id(), &code, "y", T0, &mut rng());
    assert_eq!(book.receive(&mac, &second, T0 + 2).unwrap_err(), PairError::CodeInvalid, "同一個 QR 第二支手機掃 → 拒絕");

    let wrong = phone().pair_request(&mac.kx_public(), &mac.mac_id(), "AAAAAAAA", "x", T0, &mut rng());
    assert_eq!(book.receive(&mac, &wrong, T0 + 1).unwrap_err(), PairError::CodeInvalid);
}

#[test]
fn pairing_request_for_another_mac_cannot_be_opened() {
    let other = MacKeys::from_seed([8; 32], [8; 32]);
    let mut book = PairingBook::default();
    let code = book.issue(T0, &mut rng());
    let wire = phone().pair_request(&other.kx_public(), &other.mac_id(), &code, "x", T0, &mut rng());
    assert_eq!(book.receive(&mac(), &wire, T0).unwrap_err(), PairError::Undecryptable);
}

// ---------------------------------------------------------------- AC-MP-02／INV-29

#[test]
fn inv_pairing_requires_mac_confirm() {
    let mac = mac();
    let mut book = PairingBook::default();
    let code = book.issue(T0, &mut rng());
    let wire = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "Alex 的 iPhone", T0, &mut rng());
    let pending = book.receive(&mac, &wire, T0 + 5).unwrap();
    assert_eq!(pending.device_name, "Alex 的 iPhone");
    assert_eq!(pending.device_id, phone().device_id());
    assert_eq!(book.pending().len(), 1, "正確的請求只進入待允許");

    // 拒絕：清單不變、之後也不能再允許
    book.reject(&pending.id);
    assert!(book.pending().is_empty());
    assert_eq!(book.approve(&pending.id, T0 + 6).unwrap_err(), PairError::NoSuchRequest);

    // 允許才產生裝置
    let code = book.issue(T0, &mut rng());
    let wire = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "Alex 的 iPhone", T0, &mut rng());
    let pending = book.receive(&mac, &wire, T0 + 7).unwrap();
    let device = book.approve(&pending.id, T0 + 8).unwrap();
    assert_eq!(device.id, phone().device_id());
    assert_eq!(device.paired_at, T0 + 8);
    assert!(!device.revoked);
    // 待允許太久（超過配對碼期限）就不能允許
    let code = book.issue(T0, &mut rng());
    let wire = phone().pair_request(&mac.kx_public(), &mac.mac_id(), &code, "x", T0, &mut rng());
    let stale = book.receive(&mac, &wire, T0 + 1).unwrap();
    assert_eq!(book.approve(&stale.id, T0 + CODE_TTL_SECS + 2).unwrap_err(), PairError::NoSuchRequest);
}

#[test]
fn pair_accepted_is_signed_by_the_mac() {
    let (_, device) = paired();
    let mac = mac();
    let ok = pair_accepted(&mac, &device, T0);
    assert!(phone().verify_pair_accepted(&mac.sig_public(), &ok));
    let forged = pair_accepted(&MacKeys::from_seed([1; 32], [9; 32]), &device, T0);
    assert!(!phone().verify_pair_accepted(&mac.sig_public(), &forged));
}

// ---------------------------------------------------------------- AC-MP-03／INV-28

#[test]
fn inv_mobile_envelope_verified() {
    let mac = mac();
    // 正常封包：解開、記下 seq
    let (mut reg, device) = paired();
    let body = open_from_device(&mac, &mut reg, &sealed(1, T0, r#"{"type":"sync"}"#), T0).unwrap();
    assert_eq!(body, br#"{"type":"sync"}"#);
    assert_eq!(reg.get(&device.id).unwrap().last_seq, 1);

    type Case = (&'static str, Box<dyn Fn(&mut Registry) -> Envelope>, Reject);
    let cases: Vec<Case> = vec![
        ("未知裝置", Box::new(|_| {
            DeviceKeys::from_seed([7; 32], [7; 32]).seal(&crate::mac().kx_public(), 5, T0, b"{}", [1; 12])
        }), Reject::UnknownDevice),
        ("已撤銷", Box::new(|r: &mut Registry| { r.revoke(&phone().device_id()); sealed(5, T0, "{}") }), Reject::Revoked),
        ("seq 重放", Box::new(|_| sealed(1, T0, "{}")), Reject::Replay),
        ("seq 倒退", Box::new(|_| sealed(0, T0, "{}")), Reject::Replay),
        ("時間太早", Box::new(|_| sealed(5, T0 - 121, "{}")), Reject::Clock),
        ("時間太晚", Box::new(|_| sealed(5, T0 + 121, "{}")), Reject::Clock),
        ("密文被改", Box::new(|_| { let mut e = sealed(5, T0, "{}"); e.ct = tamper(&e.ct); e }), Reject::BadSignature),
        ("簽章被改", Box::new(|_| { let mut e = sealed(5, T0, "{}"); e.sig = tamper(&e.sig); e }), Reject::BadSignature),
        ("seq 被改（簽章涵蓋）", Box::new(|_| { let mut e = sealed(5, T0, "{}"); e.seq = 6; e }), Reject::BadSignature),
    ];
    for (name, make, want) in cases {
        let (mut reg, device) = paired();
        open_from_device(&mac, &mut reg, &sealed(1, T0, "{}"), T0).unwrap();
        let env = make(&mut reg);
        let got = open_from_device(&mac, &mut reg, &env, T0).unwrap_err();
        assert_eq!(got, want, "{name}");
        if want != Reject::UnknownDevice {
            assert_eq!(reg.get(&device.id).unwrap().last_seq, 1, "{name}：被拒的封包不得推進 seq");
        }
    }
}

fn tamper(b64: &str) -> String {
    let mut bytes = b64::decode(b64).unwrap();
    bytes[0] ^= 0x01;
    b64::encode(&bytes)
}

// ---------------------------------------------------------------- 雙向加解密

#[test]
fn mac_to_device_round_trip_and_forgery_rejected() {
    let mac = mac();
    let (_, device) = paired();
    let env = seal_from_mac(&mac, &device, 3, T0, br#"{"type":"snapshot"}"#, [4; 12]).unwrap();
    let body = phone().open_from_mac(&mac.kx_public(), &mac.sig_public(), &env).unwrap();
    assert_eq!(body, br#"{"type":"snapshot"}"#);
    let impostor = MacKeys::from_seed([1; 32], [5; 32]);
    let forged = seal_from_mac(&impostor, &device, 4, T0, b"{}", [4; 12]).unwrap();
    assert_eq!(phone().open_from_mac(&mac.kx_public(), &mac.sig_public(), &forged).unwrap_err(), Reject::BadSignature);
}

#[test]
fn qr_payload_carries_relay_mac_code_and_public_keys() {
    let mac = mac();
    let p = qr_payload("wss://kunsu-relay.zeabur.app/v1", &mac, "ABCD2345");
    assert!(p.starts_with("kunsu-pair:v1?"));
    let parsed = parse_qr_payload(&p).unwrap();
    assert_eq!(parsed.relay, "wss://kunsu-relay.zeabur.app/v1");
    assert_eq!(parsed.mac, mac.mac_id());
    assert_eq!(parsed.code, "ABCD2345");
    assert_eq!(parsed.kx, mac.kx_public());
    assert_eq!(parsed.sig, mac.sig_public());
    assert!(qr_svg(&p).starts_with("<?xml") || qr_svg(&p).contains("<svg"));
}

#[test]
fn keys_survive_a_json_round_trip() {
    let mac = mac();
    let back = MacKeys::from_json(&mac.to_json()).unwrap();
    assert_eq!(back.mac_id(), mac.mac_id());
    assert_eq!(back.kx_public(), mac.kx_public());
    let mut reg = Registry::default();
    reg.upsert(paired().1);
    let back = Registry::from_json(&reg.to_json()).unwrap();
    assert_eq!(back.devices.len(), 1);
}

// ---------------------------------------------------------------- AC-MP-04 互通測試向量（Swift 端讀同一個檔）

#[test]
fn interop_vector_matches_the_committed_file() {
    use serde_json::json;
    let mac = MacKeys::from_seed([1; 32], [2; 32]);
    let dev = DeviceKeys::from_seed([3; 32], [4; 32]);
    let body = json!({"type": "answer", "question_id": "q1", "choice_id": "a"}).to_string();
    let d2m = dev.seal(&mac.kx_public(), 7, T0, body.as_bytes(), [5; 12]);
    let reg_dev = Device {
        id: dev.device_id(),
        name: "x".into(),
        kx: b64::encode(&dev.kx_public()),
        sig: b64::encode(&dev.sig_public()),
        paired_at: T0,
        last_seq: 0,
        revoked: false,
        last_seen: None,
    };
    let m2d = seal_from_mac(&mac, &reg_dev, 9, T0, br#"{"type":"snapshot"}"#, [6; 12]).unwrap();
    let pair = dev.pair_request_with_eph([7; 32], [8; 12], &mac.kx_public(), &mac.mac_id(), "ABCD2345", "iPhone", T0);
    let vector = json!({
        "seeds": {"mac_kx": b64::encode(&[1; 32]), "mac_sig": b64::encode(&[2; 32]), "dev_kx": b64::encode(&[3; 32]), "dev_sig": b64::encode(&[4; 32]), "eph": b64::encode(&[7; 32])},
        "mac_id": mac.mac_id(),
        "mac_kx_pub": b64::encode(&mac.kx_public()),
        "mac_sig_pub": b64::encode(&mac.sig_public()),
        "device_id": dev.device_id(),
        "dev_sig_pub": b64::encode(&dev.sig_public()),
        "pair": {"code": "ABCD2345", "device_name": "iPhone", "ts": T0, "nonce": b64::encode(&[8; 12]), "wire": pair},
        "d2m": {"plaintext": body, "envelope": d2m},
        "m2d": {"plaintext": "{\"type\":\"snapshot\"}", "envelope": m2d},
        "pair_accepted": pair_accepted(&mac, &reg_dev, T0),
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/design/mobile-relay-vector.json");
    let text = serde_json::to_string_pretty(&vector).unwrap() + "\n";
    if std::env::var("UPDATE_VECTOR").is_ok() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "互通測試向量變了：確認後用 UPDATE_VECTOR=1 重寫");
}
