//! AC-MU-06：從 DMG 裝的 Mac 第一次打開就自己準備好。
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::first_run::{setup, LAUNCH_LABEL};

fn bundle(dir: &Path, cli: &str) {
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin/kunsu"), cli).unwrap();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/policy.toml"), "[force]\ncycle_min = 15\n").unwrap();
    std::fs::create_dir_all(dir.join("playbooks")).unwrap();
    std::fs::write(dir.join("playbooks/a.md"), "A").unwrap();
    std::fs::write(dir.join("playbooks/b.md"), "B").unwrap();
    std::fs::create_dir_all(dir.join("plugins/kunsu/scripts")).unwrap();
    std::fs::write(dir.join("plugins/kunsu/plugin.json"), "{}").unwrap();
    std::fs::write(dir.join("plugins/kunsu/scripts/manage.py"), "print(1)").unwrap();
}

#[test]
fn ac_mu_06_first_launch_prepares_everything() {
    let res = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    bundle(res.path(), "cli v1");
    let app = Path::new("/Applications/Kunsu.app");
    setup(res.path(), home.path(), Some(app)).unwrap();
    let k = home.path().join(".kunsu");
    assert_eq!(std::fs::read_to_string(k.join("bin/kunsu")).unwrap(), "cli v1");
    assert_eq!(std::fs::metadata(k.join("bin/kunsu")).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(k.join("policy.toml").exists());
    assert_eq!(std::fs::read_to_string(k.join("playbooks/b.md")).unwrap(), "B");
    assert!(k.join("plugins/kunsu/scripts/manage.py").exists());
    let plist = home.path().join(format!("Library/LaunchAgents/{LAUNCH_LABEL}.plist"));
    let text = std::fs::read_to_string(&plist).unwrap();
    assert!(text.contains("/Applications/Kunsu.app") && text.contains("RunAtLoad"));
}

#[test]
fn ac_mu_06_never_overwrites_the_users_files_but_keeps_cli_and_plugin_in_sync() {
    let res = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    bundle(res.path(), "cli v1");
    setup(res.path(), home.path(), None).unwrap();
    let k = home.path().join(".kunsu");
    std::fs::write(k.join("policy.toml"), "# 使用者改過").unwrap();
    std::fs::write(k.join("playbooks/a.md"), "使用者改過").unwrap();
    std::fs::remove_file(k.join("playbooks/b.md")).unwrap();
    // App 更新：CLI 與外掛換新
    bundle(res.path(), "cli v2");
    std::fs::write(res.path().join("plugins/kunsu/plugin.json"), "{\"v\":2}").unwrap();
    setup(res.path(), home.path(), None).unwrap();
    assert_eq!(std::fs::read_to_string(k.join("policy.toml")).unwrap(), "# 使用者改過");
    assert_eq!(std::fs::read_to_string(k.join("playbooks/a.md")).unwrap(), "使用者改過");
    assert_eq!(std::fs::read_to_string(k.join("playbooks/b.md")).unwrap(), "B", "缺的補回來");
    assert_eq!(std::fs::read_to_string(k.join("bin/kunsu")).unwrap(), "cli v2");
    assert_eq!(std::fs::read_to_string(k.join("plugins/kunsu/plugin.json")).unwrap(), "{\"v\":2}");
    // 不是從 /Applications 執行（開發中）就不寫開機啟動
    assert!(!home.path().join(format!("Library/LaunchAgents/{LAUNCH_LABEL}.plist")).exists());
}

#[test]
fn ac_mu_06_existing_login_item_is_left_alone() {
    let res = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    bundle(res.path(), "cli");
    let plist = home.path().join(format!("Library/LaunchAgents/{LAUNCH_LABEL}.plist"));
    std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
    std::fs::write(&plist, "custom").unwrap();
    setup(res.path(), home.path(), Some(Path::new("/Applications/Kunsu.app"))).unwrap();
    assert_eq!(std::fs::read_to_string(&plist).unwrap(), "custom");
}

#[test]
fn ac_mu_06_fix_commands_point_at_kunsu_dir_not_the_repo() {
    assert!(!kunsu_server::settings::CODEX_FIX.contains("Desktop/kunsu"), "{}", kunsu_server::settings::CODEX_FIX);
    assert!(kunsu_server::settings::CODEX_FIX.contains("~/.kunsu/plugins/kunsu/scripts/manage.py"));
    assert!(!kunsu_server::settings::CLI_FIX.contains("Desktop/kunsu"), "{}", kunsu_server::settings::CLI_FIX);
}
