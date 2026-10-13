//! 第一次打開（BDD multi-user AC-MU-06）：從 DMG 裝的 Mac 沒有 build-install.sh，App 自己把 CLI、policy、playbooks、
//! 外掛與開機啟動準備好。規則：使用者的檔案（policy.toml、改過的 playbook、已有的開機啟動）永不覆蓋；App 自己管的
//! （CLI、外掛）每次跟 App 同步。每次啟動都跑，重複執行沒有副作用。
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// launchd 標籤（與 scripts/build-install.sh 相同）。
pub const LAUNCH_LABEL: &str = "me.ray.kunsu";

/// `bundle`：App 的 Resources 資料夾；`home`：使用者家目錄；`app`：正在執行的 .app 路徑（從 /Applications 執行時才寫開機啟動）。
pub fn setup(bundle: &Path, home: &Path, app: Option<&Path>) -> io::Result<()> {
    let kunsu = home.join(".kunsu");
    fs::create_dir_all(&kunsu)?;

    // CLI：跟 App 同一版
    let cli_src = bundle.join("bin/kunsu");
    if cli_src.is_file() {
        let dst = kunsu.join("bin/kunsu");
        if fs::read(&dst).ok() != Some(fs::read(&cli_src)?) {
            fs::create_dir_all(kunsu.join("bin"))?;
            let tmp = kunsu.join("bin/.kunsu.new");
            fs::copy(&cli_src, &tmp)?;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
            fs::rename(&tmp, &dst)?;
        }
    }

    // policy.toml：沒有才放
    let policy = bundle.join("config/policy.toml");
    if policy.is_file() && !kunsu.join("policy.toml").exists() {
        fs::copy(&policy, kunsu.join("policy.toml"))?;
    }

    // playbooks：缺的才補
    let books = bundle.join("playbooks");
    if books.is_dir() {
        fs::create_dir_all(kunsu.join("playbooks"))?;
        for entry in fs::read_dir(&books)? {
            let entry = entry?;
            let dst = kunsu.join("playbooks").join(entry.file_name());
            if entry.file_type()?.is_file() && !dst.exists() {
                fs::copy(entry.path(), dst)?;
            }
        }
    }

    // 外掛：App 管理，每次同步
    let plugin = bundle.join("plugins/kunsu");
    if plugin.is_dir() {
        sync_dir(&plugin, &kunsu.join("plugins/kunsu"))?;
    }

    // 開機啟動：只在從 /Applications 執行、而且還沒有時寫
    if let Some(app) = app.filter(|a| a.starts_with("/Applications")) {
        let plist = home.join(format!("Library/LaunchAgents/{LAUNCH_LABEL}.plist"));
        if !plist.exists() {
            fs::create_dir_all(plist.parent().expect("has parent"))?;
            fs::write(&plist, launch_plist(app))?;
        }
    }
    Ok(())
}

fn sync_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "__pycache__" {
            continue;
        }
        let (from, to) = (entry.path(), dst.join(&name));
        if entry.file_type()?.is_dir() {
            sync_dir(&from, &to)?;
        } else if fs::read(&to).ok() != Some(fs::read(&from)?) {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 跟 scripts/build-install.sh 寫的一樣：登入時用 `open -a` 打開 App。
fn launch_plist(app: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-a</string>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#,
        app.display()
    )
}
