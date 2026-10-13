//! AC-MU-01：資料位置不綁任何人（docs/bdd/multi-user.md）。
use std::path::{Path, PathBuf};

use kunsu_core::paths::{meta_ads_config, resolve_vault_root, LEGACY_VAULT};

const HOME: &str = "/Users/someone";

#[test]
fn env_wins_then_config_then_legacy_then_default() {
    let home = Path::new(HOME);
    let cfg = r#"{"data_dir": "/Volumes/work/kunsu"}"#;
    assert_eq!(resolve_vault_root(Some("/tmp/v"), Some(cfg), home, true), PathBuf::from("/tmp/v"));
    assert_eq!(resolve_vault_root(None, Some(cfg), home, true), PathBuf::from("/Volumes/work/kunsu"));
    assert_eq!(resolve_vault_root(None, None, home, true), Path::new(HOME).join(LEGACY_VAULT), "舊資料夾存在就沿用");
    assert_eq!(resolve_vault_root(None, None, home, false), PathBuf::from("/Users/someone/Kunsu"), "新使用者");
}

#[test]
fn blank_or_broken_settings_fall_through() {
    let home = Path::new(HOME);
    assert_eq!(resolve_vault_root(Some("  "), Some("not json"), home, false), PathBuf::from("/Users/someone/Kunsu"));
    assert_eq!(resolve_vault_root(None, Some(r#"{"data_dir": ""}"#), home, false), PathBuf::from("/Users/someone/Kunsu"));
    assert_eq!(resolve_vault_root(None, Some(r#"{"data_dir": "~/Notes/Kunsu"}"#), home, false), PathBuf::from("/Users/someone/Notes/Kunsu"), "~ 展開成家目錄");
}

#[test]
fn meta_ads_config_lives_in_the_vault() {
    assert_eq!(meta_ads_config(Path::new("/v")), PathBuf::from("/v/projects/meta-ads/config.json"));
}

/// 原始碼裡不能再有寫死的個人路徑（測試資料除外）。
#[test]
fn no_hardcoded_personal_paths_in_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // 禁止字串在執行時組出來，這個測試檔本身才不會含有它。
    let forbidden = ["/Users/", "sr", "0725"].concat();
    let mut bad = vec![];
    for dir in ["crates/kunsu-core/src", "crates/kunsu-server/src", "crates/kunsu-cli/src", "crates/kunsu-mobile/src", "src-tauri/src"] {
        walk(&root.join(dir), &mut |p, text| {
            for (i, line) in text.lines().enumerate() {
                if line.contains(&forbidden) { bad.push(format!("{}:{}", p.display(), i + 1)); }
            }
        });
    }
    assert!(bad.is_empty(), "寫死的個人路徑：{bad:#?}");
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path, &str)) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() { walk(&p, f) } else if p.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&p).unwrap();
            // #[cfg(test)] 之後是測試資料，不算
            let src = text.split("#[cfg(test)]").next().unwrap_or("");
            f(&p, src);
        }
    }
}
