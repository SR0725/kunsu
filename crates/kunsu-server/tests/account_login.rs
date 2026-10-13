//! Mac 登入帳號（BDD multi-user AC-MU-03）：跑真的帳號服務（relay/main.js），沒有共用的 RELAY_MAC_TOKEN。
//! 需要 node；沒有 node 就跳過。

mod common;

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use kunsu_server::mobile::MobileBridge;
use kunsu_server::StaticKeyProvider;
use tempfile::TempDir;

struct Cloud {
    child: Child,
    port: u16,
    mail: std::path::PathBuf,
    _dir: TempDir,
}

impl Drop for Cloud {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn start_cloud() -> Option<Cloud> {
    let dir = TempDir::new().unwrap();
    let mail = dir.path().join("mail.jsonl");
    let main = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../relay/main.js");
    let mut child = Command::new("node")
        .arg(main)
        .env("PORT", "0")
        .env("KUNSU_DEV_MAIL_LOG", &mail)
        .env("PUBLIC_BASE_URL", "http://cloud.test")
        .env_remove("RELAY_MAC_TOKEN")
        .env_remove("DATABASE_URL")
        .env_remove("RESEND_API_KEY")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).ok()?;
    let port = line.trim().rsplit(' ').next()?.parse().ok()?;
    Some(Cloud { child, port, mail, _dir: dir })
}

/// 模擬使用者在信裡點連結（任何裝置）。
async fn click_link(cloud: &Cloud) {
    let text = std::fs::read_to_string(&cloud.mail).unwrap();
    let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let body = last["text"].as_str().unwrap();
    let token = body.split("token=").nth(1).unwrap().split_whitespace().next().unwrap();
    let res = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/auth/email/verify", cloud.port))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

async fn eventually(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn ac_mu_03_mac_logs_in_registers_and_uses_its_own_relay_credential() {
    let Some(cloud) = start_cloud() else {
        eprintln!("沒有 node，跳過");
        return;
    };
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    let relay = format!("ws://127.0.0.1:{}/v1", cloud.port);
    let bridge = MobileBridge::load(ctx.handle.clone(), Arc::new(StaticKeyProvider(test_key())), dir.path(), &relay).unwrap();
    assert!(bridge.account().is_none(), "還沒登入");
    bridge.spawn();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!bridge.status().connected, "沒有憑證上不了線");

    let poll = bridge.account_start_login("alex@example.com").await.unwrap();
    assert!(!bridge.account_poll(&poll).await.unwrap(), "還沒點連結");
    click_link(&cloud).await;
    assert!(bridge.account_poll(&poll).await.unwrap(), "點了之後登入並註冊這台 Mac");

    let acc = bridge.account().unwrap();
    assert_eq!(acc.email, "alex@example.com");
    let token_file = dir.path().join("relay-token");
    let mode = std::fs::metadata(&token_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let session_file = dir.path().join("account.json");
    assert_eq!(std::fs::metadata(&session_file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(eventually(|| bridge.status().connected).await, "用自己的憑證上線");

    bridge.account_logout().await.unwrap();
    assert!(bridge.account().is_none());
    assert!(!token_file.exists() && !session_file.exists());
    assert!(eventually(|| !bridge.status().connected).await, "登出後斷線");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!bridge.status().connected, "也不會再連上");
}
