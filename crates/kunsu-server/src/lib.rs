//! kunsu-server — Kunsu 服務層＋協定層（CONTRACT §5/§6/§7/§8/§10）。
//!
//! 服務層（前一波）：SQLite（WAL）儲存、`ServerHandle`、timer loop、
//! vault ledger（append-only jsonl + OPEN.json + rebuild）、tokio broadcast
//! 事件通道。
//!
//! 協定層（本波）：
//! - [`http`]：axum HTTP（bind 127.0.0.1:4777）——Bearer auth（/health 免）、
//!   /health、/describe、/events/kongko、/events（WS）、/approvals/verify、/mcp。
//! - [`mcp`]：手寫 JSON-RPC 2.0 streamable HTTP MCP（14 tools；MCP 面物理上
//!   沒有作答方法——INV-7）。
//! - [`approval`]：批准簽章鏈（KeyProvider/Keychain、executor.py
//!   normalized_param_hash 逐字元一致移植、HMAC record、verify 單次消耗+TTL）。
//! - [`reactor`]：串行喚醒（claude/codex spawn、dry_run、重試退避、死信）。
//! - [`inbox`]：檔案 inbox ingester（60s 掃描、processed/quarantine 分流）。
//! - [`discord`]：真 webhook 發送器（policy 空 → meta-ads config fallback）。
//! - [`settings`]：主視窗設定（`policy.local.toml` 疊層讀寫、熱載）與 `app_status`（CONTRACT §15）。
//!
//! 鐵則：
//! - 狀態機／驗證／policy 全部委派 kunsu-core 純函數；本層只做 I/O 與編排。
//! - 單一寫者：所有變更序列化在 `ServerHandle` 的 write gate 後面。
//! - vault ledger 是真相層；SQLite 只是可重建的運行索引（`rebuild_db`）。
//! - `now` 由呼叫端傳入（timer loop 傳 `Utc::now()`），測試完全掌控時間。

pub mod approval;
pub mod auth;
pub mod db;
pub mod discord;
pub mod draft;
pub mod error;
pub mod events;
pub mod guards_probe;
pub mod http;
pub mod inbox;
pub mod ledger;
pub mod mcp;
pub mod mobile;
pub mod reactor;
pub mod relay_ai;
pub mod service;
pub mod settings;
pub mod store;
pub mod timer;

pub use approval::{
    default_key_provider, derive_subkey, normalized_param_hash, run_approval_chain, sign_record,
    ui_answer_signature, verify_and_consume, verify_spec_matches_card, ChainOutcome, KeyProvider,
    StaticKeyProvider, VerifyOutcome, APPROVAL_CLOCK_SKEW_SECONDS, APPROVAL_TTL_SECONDS,
    DOMAIN_APPROVAL_RECORD, DOMAIN_UI_ANSWER, EXECUTOR_SCRIPT_REL, KEYCHAIN_SERVICE, PYTHON_BIN,
};
#[cfg(target_os = "macos")]
pub use approval::KeychainKeyProvider;
pub use auth::ensure_token;
pub use discord::{
    resolve_webhook, DiscordMessage, DiscordSender, LoggingDiscord, RecordingDiscord,
    WebhookDiscord,
};
pub use draft::{
    CanvasRenderer, CanvasSource, DraftIssue, DraftPaths, EvidenceBundle, RegisterOptions, RegisterOutcome,
    RenderReport, RenderRequest,
};
pub use error::ServerError;
pub use events::{ExecStatus, ExecStep, ServerEvent};
pub use guards_probe::{GuardProbe, GuardSnapshot, StubGuardProbe};
pub use http::{router, serve, AppState};
pub use inbox::{InboxIngester, IngestReport};
pub use ledger::{Ledger, LedgerRow, LedgerType};
pub use mcp::{normalize_create_args, tool_specs, ASK_WAIT_MAX_TIMEOUT_S};
pub use reactor::{PlannedCommand, Reactor, ReactorConfig};
pub use service::{
    AnswerBatch, AnswerInput, ApprovalOutcome, BoardPack, CreateOutcome, Dedup, DndStatus, Health,
    KongkoOutcome, KongkoPublication, KunsuConfig, MoneyState, RebuildReport, ServerHandle, TerminalEvent,
    UpdatePatch,
};
pub use timer::spawn_timer;
