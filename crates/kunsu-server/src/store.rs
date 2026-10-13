//! DB 行級操作。Question 全量 JSON 進 `data` 欄；熱查詢欄位另抽欄。
//! 一律 runtime query（不用編譯期 DATABASE_URL）。

use chrono::{DateTime, Utc};
use kunsu_core::time::{taipei_date, Ts};
use kunsu_core::types::{Question, Source, State};
use sqlx::{Row, SqlitePool};

use crate::error::ServerError;

/// 終態集合的 SQL 片段（含 routed）。
pub const TERMINAL_STATES_SQL: &str =
    "('answered','expired','cancelled','superseded','routed')";

/// enum → snake_case 字串（serde 白名單同源，不另手寫映射表）。
pub fn state_str(s: State) -> String {
    serde_json::to_value(s)
        .expect("State 可序列化")
        .as_str()
        .expect("State 序列化為字串")
        .to_string()
}

/// per-source 配額鍵：routine_id > session_id > cwd（同 agent/routine 一鍵）。
pub fn source_key(s: &Source) -> String {
    let agent = serde_json::to_value(s.agent)
        .expect("AgentKind 可序列化")
        .as_str()
        .expect("AgentKind 序列化為字串")
        .to_string();
    let scope = s
        .routine_id
        .as_deref()
        .or(s.session_id.as_deref())
        .unwrap_or(s.cwd.as_str());
    format!("{agent}:{scope}")
}

fn tier_str(q: &Question) -> String {
    serde_json::to_value(q.tier)
        .expect("Tier 可序列化")
        .as_str()
        .expect("Tier 序列化為字串")
        .to_string()
}

/// upsert 題目全量快照 + 熱欄位。
pub async fn put_question(pool: &SqlitePool, q: &Question) -> Result<(), ServerError> {
    let data = serde_json::to_string(q)?;
    sqlx::query(
        r#"INSERT INTO questions
             (id, data, state, tier, next_transition_at, kongko_ref, source_key,
              internal, created_date_tpe, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
           ON CONFLICT(id) DO UPDATE SET
             data = excluded.data,
             state = excluded.state,
             tier = excluded.tier,
             next_transition_at = excluded.next_transition_at,
             kongko_ref = excluded.kongko_ref,
             updated_at = excluded.updated_at"#,
    )
    .bind(&q.id)
    .bind(&data)
    .bind(state_str(q.state))
    .bind(tier_str(q))
    .bind(q.next_transition_at.timestamp_millis())
    .bind(&q.kongko_ref)
    .bind(source_key(&q.source))
    .bind(q.internal as i64)
    .bind(taipei_date(q.created_at).to_string())
    .bind(q.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_question(row: &sqlx::sqlite::SqliteRow) -> Result<Question, ServerError> {
    let data: String = row.get("data");
    Ok(serde_json::from_str(&data)?)
}

pub async fn get_question(pool: &SqlitePool, id: &str) -> Result<Option<Question>, ServerError> {
    let row = sqlx::query("SELECT data FROM questions WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(row_to_question).transpose()
}

/// 全部非終態題（OPEN.json 快照與 board_pack 的資料面）。
pub async fn list_open(pool: &SqlitePool) -> Result<Vec<Question>, ServerError> {
    let rows = sqlx::query(&format!(
        "SELECT data FROM questions WHERE state NOT IN {TERMINAL_STATES_SQL} ORDER BY created_at"
    ))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_question).collect()
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<Question>, ServerError> {
    let rows = sqlx::query("SELECT data FROM questions ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_question).collect()
}

/// timer 掃描面：非終態且 next_transition_at 已到。
pub async fn due_questions(pool: &SqlitePool, now: Ts) -> Result<Vec<Question>, ServerError> {
    let rows = sqlx::query(&format!(
        "SELECT data FROM questions
         WHERE state NOT IN {TERMINAL_STATES_SQL} AND next_transition_at <= ?1
         ORDER BY next_transition_at"
    ))
    .bind(now.timestamp_millis())
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_question).collect()
}

/// 今日（台北）某 source 已建題數——非 internal、非 upsert touch（touch 不 insert）。
pub async fn quota_used_today(
    pool: &SqlitePool,
    source_key: &str,
    now: Ts,
) -> Result<u32, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM questions
         WHERE source_key = ?1 AND created_date_tpe = ?2 AND internal = 0",
    )
    .bind(source_key)
    .bind(taipei_date(now).to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") as u32)
}

/// 事件 append。回傳單調遞增 seq（answers_since 游標）。
pub async fn insert_event(
    pool: &SqlitePool,
    question_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    now: Ts,
) -> Result<i64, ServerError> {
    let res = sqlx::query(
        "INSERT INTO events (question_id, kind, payload, date_tpe, at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(question_id)
    .bind(kind)
    .bind(payload.to_string())
    .bind(taipei_date(now).to_string())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// 今日（台北）forced 預算已消耗數（internal 題不寫此事件，天然排除）。
pub async fn forced_budget_used_today(pool: &SqlitePool, now: Ts) -> Result<u32, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM events WHERE kind = 'forced_budget' AND date_tpe = ?1",
    )
    .bind(taipei_date(now).to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") as u32)
}

/// 該題今日是否已消耗過 forced 預算（跨午夜重置 forced_today_used 用）。
pub async fn budget_consumed_today(
    pool: &SqlitePool,
    question_id: &str,
    now: Ts,
) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM events
         WHERE kind = 'forced_budget' AND question_id = ?1 AND date_tpe = ?2",
    )
    .bind(question_id)
    .bind(taipei_date(now).to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") > 0)
}

/// 終態事件行（answers_since 收割面）。
pub struct TerminalEventRow {
    pub seq: i64,
    pub question_id: String,
    pub terminal_type: String,
}

pub async fn terminal_events_after(
    pool: &SqlitePool,
    cursor: i64,
) -> Result<Vec<TerminalEventRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT seq, question_id, payload FROM events
         WHERE kind = 'terminal' AND seq > ?1 ORDER BY seq",
    )
    .bind(cursor)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let payload: serde_json::Value = serde_json::from_str(&r.get::<String, _>("payload"))?;
            Ok(TerminalEventRow {
                seq: r.get("seq"),
                question_id: r.get("question_id"),
                terminal_type: payload
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            })
        })
        .collect()
}

pub async fn get_cursor(pool: &SqlitePool, consumer: &str) -> Result<Option<i64>, ServerError> {
    let row = sqlx::query("SELECT cursor FROM consumers WHERE consumer = ?1")
        .bind(consumer)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("cursor")))
}

pub async fn put_cursor(
    pool: &SqlitePool,
    consumer: &str,
    cursor: i64,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "INSERT INTO consumers (consumer, cursor, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(consumer) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
    )
    .bind(consumer)
    .bind(cursor)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, ServerError> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn put_setting(
    pool: &SqlitePool,
    key: &str,
    value: &str,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_setting(pool: &SqlitePool, key: &str) -> Result<(), ServerError> {
    sqlx::query("DELETE FROM settings WHERE key = ?1").bind(key).execute(pool).await?;
    Ok(())
}

/// reactor 入列（answer_id UNIQUE → 同一答案永不重複入列，冪等）。
/// 回傳是否真的新入列。
pub async fn enqueue_reactor(
    pool: &SqlitePool,
    question_id: &str,
    answer_id: &str,
    dry_run: bool,
    now: Ts,
) -> Result<bool, ServerError> {
    let status = if dry_run { "dry_run" } else { "queued" };
    let res = sqlx::query(
        "INSERT INTO reactor_runs (question_id, answer_id, status, attempts, created_at, updated_at)
         VALUES (?1, ?2, ?3, 0, ?4, ?4)
         ON CONFLICT(answer_id) DO NOTHING",
    )
    .bind(question_id)
    .bind(answer_id)
    .bind(status)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 尚未消耗今日 forced 預算的非終態 T0（非 internal）數——T0 准入的
/// 「今日預算未滿」除了已消耗數，也要算進已排隊等蓋板的存量。
pub async fn open_t0_unforced_count(pool: &SqlitePool) -> Result<u32, ServerError> {
    let row = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM questions
         WHERE tier = 't0' AND internal = 0 AND state NOT IN {TERMINAL_STATES_SQL}
         AND COALESCE(json_extract(data, '$.forced_today_used'), 0) = 0"
    ))
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") as u32)
}

/// 非終態、非 internal 的 T1 題數（批次窗清點；CONTRACT §6 規則 8）。
pub async fn open_t1_count(pool: &SqlitePool) -> Result<u32, ServerError> {
    let row = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM questions
         WHERE tier = 't1' AND internal = 0 AND state NOT IN {TERMINAL_STATES_SQL}"
    ))
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") as u32)
}

/// 該批次窗今日（台北）是否已觸發過（每窗每日一次的冪等鍵）。
pub async fn batch_window_fired_today(
    pool: &SqlitePool,
    window: &str,
    now: Ts,
) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM events
         WHERE kind = 'batch_window' AND date_tpe = ?1
           AND json_extract(payload, '$.window') = ?2",
    )
    .bind(taipei_date(now).to_string())
    .bind(window)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") > 0)
}

/// answered 且 answer.acked_at 仍空的題（24h 無 ack 告警掃描面；CONTRACT §6 規則 9）。
pub async fn answered_unacked(pool: &SqlitePool) -> Result<Vec<Question>, ServerError> {
    let rows = sqlx::query(
        "SELECT data FROM questions
         WHERE state IN ('answered', 'routed') AND json_extract(data, '$.answer.acked_at') IS NULL",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_question).collect()
}

/// 帳本待補寫佇列（不屬於可重建資料，`wipe_all` 不清）。
pub async fn queue_ledger_row(pool: &SqlitePool, row_json: &str, ts: Ts) -> Result<(), ServerError> {
    sqlx::query("INSERT INTO ledger_outbox (row_json, created_at) VALUES (?1, ?2)")
        .bind(row_json)
        .bind(ts.to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

/// 依寫入順序列出待補寫的帳本行。
pub async fn ledger_outbox(pool: &SqlitePool) -> Result<Vec<(i64, String)>, ServerError> {
    let rows = sqlx::query("SELECT seq, row_json FROM ledger_outbox ORDER BY seq").fetch_all(pool).await?;
    Ok(rows.iter().map(|r| (r.get::<i64, _>("seq"), r.get::<String, _>("row_json"))).collect())
}

pub async fn delete_ledger_outbox(pool: &SqlitePool, seq: i64) -> Result<(), ServerError> {
    sqlx::query("DELETE FROM ledger_outbox WHERE seq = ?1").bind(seq).execute(pool).await?;
    Ok(())
}

pub async fn ledger_outbox_count(pool: &SqlitePool) -> Result<u64, ServerError> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM ledger_outbox").fetch_one(pool).await?;
    Ok(row.get::<i64, _>("n") as u64)
}

/// 這題是否有 approval 已被消耗（executor 驗證過，代表可能已經 commit）。
pub async fn approval_consumed_for(pool: &SqlitePool, question_id: &str) -> Result<bool, ServerError> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM approvals_consumed WHERE question_id = ?1")
        .bind(question_id)
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n") > 0)
}

/// 記下一次授權（nonce）的錢鏈結果（同 nonce 覆寫：例如 EXEC_UNKNOWN 之後由使用者人工對帳）。
pub async fn put_money_outcome(pool: &SqlitePool, nonce: &str, question_id: &str, result: &str, at: Ts) -> Result<(), ServerError> {
    sqlx::query(
        "INSERT INTO money_outcomes (nonce, question_id, result, recorded_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(nonce) DO UPDATE SET result = excluded.result, recorded_at = excluded.recorded_at",
    )
    .bind(nonce)
    .bind(question_id)
    .bind(result)
    .bind(at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// 這題「結果不明」的授權：已被 executor 消耗卻沒有錢鏈結果，或結果是 EXEC_UNKNOWN（CONTRACT §7）。
pub async fn unknown_money_nonces(pool: &SqlitePool, question_id: &str) -> Result<Vec<String>, ServerError> {
    let rows = sqlx::query(
        "SELECT c.nonce FROM approvals_consumed c
         LEFT JOIN money_outcomes o ON o.nonce = c.nonce
         WHERE c.question_id = ?1 AND (o.nonce IS NULL OR o.result LIKE 'EXEC_UNKNOWN%')
         UNION
         SELECT nonce FROM money_outcomes WHERE question_id = ?1 AND result LIKE 'EXEC_UNKNOWN%'",
    )
    .bind(question_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<String, _>("nonce")).collect())
}

/// 使用者人工對帳後結束等人工對帳的列（只動 needs_reconcile）。
pub async fn finish_reconciled_run(pool: &SqlitePool, answer_id: &str, result: &str, now: Ts) -> Result<(), ServerError> {
    sqlx::query(
        "UPDATE reactor_runs SET status = 'done', result = ?1, updated_at = ?2
         WHERE answer_id = ?3 AND status = 'needs_reconcile'",
    )
    .bind(result)
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 這個 nonce 是否已被 executor 消耗（executor 已驗證 approval，可能已經打了 API）。
pub async fn nonce_consumed(pool: &SqlitePool, nonce: &str) -> Result<bool, ServerError> {
    let row = sqlx::query("SELECT 1 FROM approvals_consumed WHERE nonce = ?1").bind(nonce).fetch_optional(pool).await?;
    Ok(row.is_some())
}

/// 這個 nonce 的錢鏈結果（`money_outcomes`；沒有結果回 None）。
pub async fn money_outcome_of(pool: &SqlitePool, nonce: &str) -> Result<Option<String>, ServerError> {
    let row = sqlx::query("SELECT result FROM money_outcomes WHERE nonce = ?1").bind(nonce).fetch_optional(pool).await?;
    Ok(row.map(|r| r.get::<String, _>("result")))
}

/// 這題是否有比這個 nonce 更新的簽發（被重新授權取代，INV-9）。以登記的先後（rowid）判斷，不受時鐘影響。
pub async fn newer_issue_exists(pool: &SqlitePool, question_id: &str, nonce: &str) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT 1 FROM approvals_issued
         WHERE question_id = ?1 AND rowid > (SELECT rowid FROM approvals_issued WHERE nonce = ?2)
         LIMIT 1",
    )
    .bind(question_id)
    .bind(nonce)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// 這題是否有任何一次授權的結果是已執行完成（`COMMIT_OK`／`RECONCILED_EXECUTED`）。
pub async fn executed_outcome_exists(pool: &SqlitePool, question_id: &str) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT 1 FROM money_outcomes
         WHERE question_id = ?1 AND (result LIKE 'COMMIT_OK%' OR result LIKE 'RECONCILED_EXECUTED%')
         LIMIT 1",
    )
    .bind(question_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// 這題簽發了、沒被 executor 消耗、也還沒有結果的 nonce（executor 還沒驗證；依簽發先後）。
pub async fn issued_unsettled_nonces(pool: &SqlitePool, question_id: &str) -> Result<Vec<String>, ServerError> {
    let rows = sqlx::query(
        "SELECT i.nonce FROM approvals_issued i
         WHERE i.question_id = ?1
           AND NOT EXISTS (SELECT 1 FROM money_outcomes o WHERE o.nonce = i.nonce)
           AND NOT EXISTS (SELECT 1 FROM approvals_consumed c WHERE c.nonce = i.nonce)
         ORDER BY i.rowid",
    )
    .bind(question_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<String, _>("nonce")).collect())
}

/// 所有已被 executor 消耗卻沒有任何結果的授權（舊資料補結果用）：(question_id, nonce)。
pub async fn consumed_without_outcome(pool: &SqlitePool) -> Result<Vec<(String, String)>, ServerError> {
    let rows = sqlx::query(
        "SELECT c.question_id, c.nonce FROM approvals_consumed c
         WHERE NOT EXISTS (SELECT 1 FROM money_outcomes o WHERE o.nonce = c.nonce)
         ORDER BY c.consumed_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get::<String, _>("question_id"), r.get::<String, _>("nonce"))).collect())
}

/// 等人工對帳的 reactor 列（舊版重啟恢復留下的資料補正用）。
pub async fn needs_reconcile_runs(pool: &SqlitePool) -> Result<Vec<ReactorRunRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT question_id, answer_id, status, attempts FROM reactor_runs
         WHERE status = 'needs_reconcile' ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ReactorRunRow {
            question_id: r.get("question_id"),
            answer_id: r.get("answer_id"),
            status: r.get("status"),
            attempts: r.get("attempts"),
        })
        .collect())
}

/// 這個答案的 reactor 列狀態（沒有列回 None）。
pub async fn reactor_run_status(pool: &SqlitePool, answer_id: &str) -> Result<Option<String>, ServerError> {
    let row = sqlx::query("SELECT status FROM reactor_runs WHERE answer_id = ?1").bind(answer_id).fetch_optional(pool).await?;
    Ok(row.map(|r| r.get::<String, _>("status")))
}

/// 重建資料庫時還原一筆交付（不經 CAS）。
pub async fn restore_delivery(
    pool: &SqlitePool,
    question_id: &str,
    answer_id: &str,
    state: &str,
    reason: Option<&str>,
    next_check_at: Option<Ts>,
    updated_at: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "INSERT OR REPLACE INTO deliveries (answer_id, question_id, state, reason, attempts, next_check_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
    )
    .bind(answer_id)
    .bind(question_id)
    .bind(state)
    .bind(reason)
    .bind(next_check_at.map(|t| t.timestamp_millis()))
    .bind(updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// 某題某種事件的 payload（新到舊）。
pub async fn events_for_question(
    pool: &SqlitePool,
    question_id: &str,
    kind: &str,
) -> Result<Vec<serde_json::Value>, ServerError> {
    let rows = sqlx::query("SELECT payload FROM events WHERE question_id = ?1 AND kind = ?2 ORDER BY rowid DESC")
        .bind(question_id)
        .bind(kind)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("payload")).ok())
        .collect())
}

/// 該題是否已有指定 kind 的事件（no_ack_alert 防重複等）。
pub async fn has_event_for_question(
    pool: &SqlitePool,
    question_id: &str,
    kind: &str,
) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM events WHERE question_id = ?1 AND kind = ?2",
    )
    .bind(question_id)
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") > 0)
}

/// 指定 kind 事件自某台北日期（含）以來的筆數（fatigue Discord 週一次節流）。
pub async fn count_events_since_date(
    pool: &SqlitePool,
    kind: &str,
    date_tpe_start: &str,
) -> Result<u32, ServerError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM events WHERE kind = ?1 AND date_tpe >= ?2",
    )
    .bind(kind)
    .bind(date_tpe_start)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n") as u32)
}

/// 疲勞降載統計面（CONTRACT §6 規則 10）。
/// - engaged：滑動窗內同 (product, kind) 且「進入 forced/批次」的題數——
///   曾消耗 forced 預算、或（現值）tier=t1（批次窗涵蓋全部非終態 T1）、
///   或曾 snooze 封頂（表示進過蓋板）。
/// - skipped：engaged 中被略過的——snooze 封頂降級、或 expired 未答。
pub struct FatigueStats {
    pub engaged: u32,
    pub skipped: u32,
}

pub async fn fatigue_stats(
    pool: &SqlitePool,
    product: &str,
    kind: &str,
    window_start_date_tpe: &str,
) -> Result<FatigueStats, ServerError> {
    let row = sqlx::query(
        r#"SELECT
             COALESCE(SUM(CASE WHEN engaged THEN 1 ELSE 0 END), 0) AS engaged,
             COALESCE(SUM(CASE WHEN engaged AND skipped THEN 1 ELSE 0 END), 0) AS skipped
           FROM (
             SELECT
               (q.tier = 't1'
                 OR EXISTS(SELECT 1 FROM events e
                           WHERE e.question_id = q.id AND e.kind = 'forced_budget')
                 OR EXISTS(SELECT 1 FROM events e
                           WHERE e.question_id = q.id AND e.kind = 'fatigue'
                             AND json_extract(e.payload, '$.type') = 'snooze_cap_downgrade')
               ) AS engaged,
               (q.state = 'expired'
                 OR EXISTS(SELECT 1 FROM events e
                           WHERE e.question_id = q.id AND e.kind = 'fatigue'
                             AND json_extract(e.payload, '$.type') = 'snooze_cap_downgrade')
               ) AS skipped
             FROM questions q
             WHERE q.internal = 0
               AND q.created_date_tpe >= ?1
               AND json_extract(q.data, '$.product') = ?2
               AND json_extract(q.data, '$.kind') = ?3
           )"#,
    )
    .bind(window_start_date_tpe)
    .bind(product)
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(FatigueStats {
        engaged: row.get::<i64, _>("engaged") as u32,
        skipped: row.get::<i64, _>("skipped") as u32,
    })
}

/// reactor 回執：answer_id 對應的入列標 done + 結果回寫。
pub async fn mark_reactor_done(
    pool: &SqlitePool,
    answer_id: &str,
    result: &str,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "UPDATE reactor_runs SET status = 'done', result = ?1, updated_at = ?2
         WHERE answer_id = ?3 AND status != 'needs_reconcile'",
    )
    .bind(result)
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// kongko 冪等記錄。已存在（重複投遞）回 false。
pub async fn record_kongko_event(
    pool: &SqlitePool,
    publication_id: &str,
    batch_id: &str,
    payload: &serde_json::Value,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "INSERT INTO kongko_events (publication_id, batch_id, payload, received_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(publication_id) DO NOTHING",
    )
    .bind(publication_id)
    .bind(batch_id)
    .bind(payload.to_string())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// kongko_ref 命中的非終態題（發佈事件結案鍵）。
pub async fn open_question_by_kongko_ref(
    pool: &SqlitePool,
    batch_id: &str,
) -> Result<Option<Question>, ServerError> {
    let row = sqlx::query(&format!(
        "SELECT data FROM questions
         WHERE kongko_ref = ?1 AND state NOT IN {TERMINAL_STATES_SQL}
         ORDER BY created_at LIMIT 1"
    ))
    .bind(batch_id)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_question).transpose()
}

/// 最後一次作答時間（/health 的 last_answer_at）。
pub async fn last_answer_at(pool: &SqlitePool) -> Result<Option<Ts>, ServerError> {
    let row = sqlx::query(
        "SELECT at FROM events WHERE kind = 'terminal' AND payload LIKE '%\"answered\"%'
         ORDER BY seq DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| {
        DateTime::parse_from_rfc3339(&r.get::<String, _>("at"))
            .ok()
            .map(|d| d.with_timezone(&Utc))
    }))
}

/// rebuild 前整庫清空（consumers 一起清：游標重置的後果只是 at-least-once 重放）。
pub async fn wipe_all(pool: &SqlitePool) -> Result<(), ServerError> {
    for table in [
        "questions",
        "events",
        "consumers",
        "reactor_runs",
        "kongko_events",
        "policies",
        "approvals_consumed",
        "approvals_issued",
        "deliveries",
        "money_outcomes",
    ] {
        sqlx::query(&format!("DELETE FROM {table}")).execute(pool).await?;
    }
    Ok(())
}

/// approval 單次消耗（INV-9）：nonce 唯一鍵；已存在（重放）回 false。
pub async fn insert_approval_consumed(
    pool: &SqlitePool,
    nonce: &str,
    question_id: &str,
    record: &serde_json::Value,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "INSERT INTO approvals_consumed (nonce, question_id, record, consumed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(nonce) DO NOTHING",
    )
    .bind(nonce)
    .bind(question_id)
    .bind(record.to_string())
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 簽發登記（verify 強化）：簽章鏈簽出 record 當下寫入。nonce 唯一鍵。
#[allow(clippy::too_many_arguments)]
pub async fn insert_approval_issued(
    pool: &SqlitePool,
    nonce: &str,
    question_id: &str,
    answer_id: &str,
    executor_token: &str,
    param_hash: &str,
    answered_at_epoch: i64,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "INSERT INTO approvals_issued
           (nonce, question_id, answer_id, executor_token, param_hash, answered_at, issued_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(nonce) DO NOTHING",
    )
    .bind(nonce)
    .bind(question_id)
    .bind(answer_id)
    .bind(executor_token)
    .bind(param_hash)
    .bind(answered_at_epoch)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// 簽發登記查詢面（verify 用）。
pub struct IssuedApproval {
    pub question_id: String,
    pub answer_id: String,
    pub executor_token: String,
    pub param_hash: String,
    pub answered_at_epoch: i64,
}

pub async fn get_approval_issued(
    pool: &SqlitePool,
    nonce: &str,
) -> Result<Option<IssuedApproval>, ServerError> {
    let row = sqlx::query(
        "SELECT question_id, answer_id, executor_token, param_hash, answered_at
         FROM approvals_issued WHERE nonce = ?1",
    )
    .bind(nonce)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| IssuedApproval {
        question_id: r.get("question_id"),
        answer_id: r.get("answer_id"),
        executor_token: r.get("executor_token"),
        param_hash: r.get("param_hash"),
        answered_at_epoch: r.get("answered_at"),
    }))
}

/// reactor 待處理列（queued=真跑、dry_run=internal 題只記錄不 spawn）。
pub struct ReactorRunRow {
    pub question_id: String,
    pub answer_id: String,
    pub status: String,
    pub attempts: i64,
}

pub async fn pending_reactor_runs(pool: &SqlitePool) -> Result<Vec<ReactorRunRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT question_id, answer_id, status, attempts FROM reactor_runs
         WHERE status IN ('queued', 'dry_run') ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ReactorRunRow {
            question_id: r.get("question_id"),
            answer_id: r.get("answer_id"),
            status: r.get("status"),
            attempts: r.get("attempts"),
        })
        .collect())
}

/// 冪等 claim（Codex 覆檢 P1）：外部副作用（prepare/commit/wake spawn）之前
/// 持久化 executing 狀態。CAS：只有 queued 能被 claim。
pub async fn mark_reactor_executing(
    pool: &SqlitePool,
    answer_id: &str,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "UPDATE reactor_runs SET status = 'executing', updated_at = ?1
         WHERE answer_id = ?2 AND status = 'queued'",
    )
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 崩潰恢復掃描面：status=executing 的殘留列（上一個進程在外部副作用中間死掉）。
pub async fn executing_reactor_runs(
    pool: &SqlitePool,
) -> Result<Vec<ReactorRunRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT question_id, answer_id, status, attempts FROM reactor_runs
         WHERE status = 'executing' ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ReactorRunRow {
            question_id: r.get("question_id"),
            answer_id: r.get("answer_id"),
            status: r.get("status"),
            attempts: r.get("attempts"),
        })
        .collect())
}

/// 崩潰後不盲目重發：標 needs_reconcile 等人工對帳。
pub async fn mark_reactor_needs_reconcile(
    pool: &SqlitePool,
    answer_id: &str,
    reason: &str,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "UPDATE reactor_runs SET status = 'needs_reconcile', result = ?1, updated_at = ?2
         WHERE answer_id = ?3",
    )
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 重新授權（決策/授權分離）：已終結（done/dead）的執行列重新入列。
/// executing / needs_reconcile 不允許——前者仍在跑、後者等人工對帳。
pub async fn requeue_reactor(
    pool: &SqlitePool,
    answer_id: &str,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "UPDATE reactor_runs SET status = 'queued', attempts = 0, updated_at = ?1
         WHERE answer_id = ?2 AND status IN ('done', 'dead')",
    )
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 死信標記（重試 3 次仍失敗；BDD harness-roundtrip 場景 9）。
pub async fn mark_reactor_dead(
    pool: &SqlitePool,
    answer_id: &str,
    reason: &str,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query(
        "UPDATE reactor_runs SET status = 'dead', result = ?1, updated_at = ?2
         WHERE answer_id = ?3",
    )
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 重試計數回寫。
pub async fn bump_reactor_attempts(
    pool: &SqlitePool,
    answer_id: &str,
    attempts: i64,
    now: Ts,
) -> Result<(), ServerError> {
    sqlx::query("UPDATE reactor_runs SET attempts = ?1, updated_at = ?2 WHERE answer_id = ?3")
        .bind(attempts)
        .bind(now.to_rfc3339())
        .bind(answer_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 交付紀錄（回答延續原對話，PRD §20）
// ---------------------------------------------------------------------------

pub struct DeliveryRow {
    pub answer_id: String,
    pub question_id: String,
    pub state: String,
    pub reason: Option<String>,
    pub attempts: i64,
}

fn delivery_row(r: &sqlx::sqlite::SqliteRow) -> DeliveryRow {
    DeliveryRow {
        answer_id: r.get("answer_id"),
        question_id: r.get("question_id"),
        state: r.get("state"),
        reason: r.get("reason"),
        attempts: r.get("attempts"),
    }
}

pub async fn get_delivery(pool: &SqlitePool, answer_id: &str) -> Result<Option<DeliveryRow>, ServerError> {
    let row = sqlx::query(
        "SELECT answer_id, question_id, state, reason, attempts FROM deliveries WHERE answer_id = ?1",
    )
    .bind(answer_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(delivery_row))
}

/// 寫入交付狀態。只有不存在或仍是 pending/held 的列能被改寫——最終狀態永不改變
/// （`inv_single_continuation`）。回傳是否真的寫入。
pub async fn put_delivery(
    pool: &SqlitePool,
    question_id: &str,
    answer_id: &str,
    state: &str,
    reason: Option<&str>,
    next_check_at: Option<Ts>,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "INSERT INTO deliveries (answer_id, question_id, state, reason, attempts, next_check_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)
         ON CONFLICT(answer_id) DO UPDATE SET
            state = excluded.state, reason = excluded.reason,
            next_check_at = excluded.next_check_at, updated_at = excluded.updated_at
         WHERE deliveries.state IN ('pending', 'held')",
    )
    .bind(answer_id)
    .bind(question_id)
    .bind(state)
    .bind(reason)
    .bind(next_check_at.map(|t| t.timestamp_millis()))
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 到期待重檢的交付（pending／held 且 next_check_at ≤ now）。
pub async fn due_deliveries(pool: &SqlitePool, now: Ts) -> Result<Vec<DeliveryRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT answer_id, question_id, state, reason, attempts FROM deliveries
         WHERE state IN ('pending', 'held') AND next_check_at IS NOT NULL AND next_check_at <= ?1
         ORDER BY next_check_at",
    )
    .bind(now.timestamp_millis())
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(delivery_row).collect())
}

pub async fn bump_delivery_attempts(pool: &SqlitePool, answer_id: &str) -> Result<i64, ServerError> {
    sqlx::query("UPDATE deliveries SET attempts = attempts + 1 WHERE answer_id = ?1")
        .bind(answer_id)
        .execute(pool)
        .await?;
    Ok(get_delivery(pool, answer_id).await?.map(|d| d.attempts).unwrap_or(0))
}

/// 這個答案的 reactor 列是否還沒跑完（queued／dry_run／executing）。
pub async fn reactor_run_open(pool: &SqlitePool, answer_id: &str) -> Result<bool, ServerError> {
    let row = sqlx::query(
        "SELECT 1 FROM reactor_runs WHERE answer_id = ?1 AND status IN ('queued', 'dry_run', 'executing')",
    )
    .bind(answer_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// 結束「交付中」：只改寫仍是 resuming 的列。
pub async fn finish_delivery(
    pool: &SqlitePool,
    answer_id: &str,
    state: &str,
    reason: Option<&str>,
    next_check_at: Option<Ts>,
    now: Ts,
) -> Result<bool, ServerError> {
    let res = sqlx::query(
        "UPDATE deliveries SET state = ?1, reason = ?2, next_check_at = ?3, updated_at = ?4
         WHERE answer_id = ?5 AND state = 'resuming'",
    )
    .bind(state)
    .bind(reason)
    .bind(next_check_at.map(|t| t.timestamp_millis()))
    .bind(now.to_rfc3339())
    .bind(answer_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 卡在「交付中」太久的列（app 在 spawn 途中崩潰）。
pub async fn stuck_deliveries(pool: &SqlitePool, older_than: Ts) -> Result<Vec<DeliveryRow>, ServerError> {
    let rows = sqlx::query(
        "SELECT answer_id, question_id, state, reason, attempts, updated_at FROM deliveries WHERE state = 'resuming'",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .filter(|r| {
            let at: String = r.get("updated_at");
            DateTime::parse_from_rfc3339(&at).is_ok_and(|t| t.with_timezone(&Utc) < older_than)
        })
        .map(delivery_row)
        .collect())
}
