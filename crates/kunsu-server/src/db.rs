//! SQLite pool（WAL）+ migration。CONTRACT §0：prod 路徑
//! `~/Library/Application Support/me.ray.kunsu/kunsu.db`；測試用 tempfile。
//! 單一寫者假設 → pool 上限 1 連線，寫入天然序列化（服務層另有 write gate）。

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::ServerError;

/// app 沒開時給 `kunsu doctor` 用：唯讀打開資料庫數帳本待補寫行數（不建資料庫、不跑 migration、不寫入資料）。
/// 舊資料庫還沒有 `ledger_outbox` 表時回 0。
pub async fn offline_ledger_backlog(db_path: &Path) -> Result<u64, ServerError> {
    let opts = SqliteConnectOptions::from_str("sqlite://")
        .expect("空 options 恆合法")
        .filename(db_path)
        .read_only(true)
        // 不用 immutable：app 當掉沒 checkpoint 時，待補寫的行可能只在 -wal 裡，immutable 會讀不到而誤報 0。
        // 代價是 SQLite 可能在旁邊留下 -shm／空的 -wal（無害，下次 app 開啟照常使用）。
        .create_if_missing(false);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
    let has_table: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'ledger_outbox'").fetch_optional(&pool).await?;
    let n = if has_table.is_some() {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ledger_outbox").fetch_one(&pool).await?;
        n.max(0) as u64
    } else {
        0
    };
    pool.close().await;
    Ok(n)
}

pub async fn open_pool(db_path: &Path) -> Result<SqlitePool, ServerError> {
    let opts = SqliteConnectOptions::from_str("sqlite://")
        .expect("空 options 恆合法")
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1) // 單一寫者
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
