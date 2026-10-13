//! Bearer token 檔（CONTRACT §0：`~/.kunsu/token`）。
//! 首次啟動自動生成 32 hex、權限 0600；agent 讀此檔帶 `Authorization: Bearer`。

use std::fs;
use std::io;
use std::path::Path;

/// 讀取或生成 Bearer token。已存在且非空 → 原樣回傳（trim 後）；
/// 否則生成 32 hex（16 bytes 亂數）寫入並 chmod 0600。
pub fn ensure_token(path: &Path) -> io::Result<String> {
    if let Ok(raw) = fs::read_to_string(path) {
        let t = raw.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let token = hex::encode(uuid::Uuid::new_v4().into_bytes());
    fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_32_hex_with_0600_and_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let t1 = ensure_token(&path).unwrap();
        assert_eq!(t1.len(), 32);
        assert!(t1.chars().all(|c| c.is_ascii_hexdigit()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let t2 = ensure_token(&path).unwrap();
        assert_eq!(t1, t2, "第二次啟動沿用同一 token");
    }
}
