//! kunsu-core — Kunsu（軍師）純函數層。
//!
//! 對應 docs/CONTRACT.md：
//! - §2 資料模型（serde 全 snake_case、enum 白名單解析）→ [`types`]
//! - §2 硬驗證 + 十秒可決 lint → [`validate`]
//! - §3 狀態機純函數 → [`state_machine`]
//! - §10 policy.toml、§15 設定疊層（`policy.local.toml`）→ [`policy`]
//! - Guards → [`guards`]
//! - upsert 合併（不重置倒數、不動 first_proposed）→ [`upsert`]
//! - 回答延續原對話的交付階梯（PRD §20）→ [`continuity`]
//! - 題組的雜湊、輪次與逐項答案驗證（PRD §25）→ [`batch`]
//!
//! 鐵則：零 I/O、零 async、不 import tokio/sqlx。`now` 一律由呼叫端傳入，
//! 本 crate 永不讀系統時鐘。Effect 是純資料，由 server 層解釋執行。

pub mod batch;
pub mod continuity;
pub mod guards;
pub mod paths;
pub mod policy;
pub mod prepare_cmd;
pub mod redline;
pub mod state_machine;
pub mod time;
pub mod types;
pub mod upsert;
pub mod validate;

pub use guards::GuardState;
pub use policy::{Hhmm, Policy};
pub use prepare_cmd::rewrite_prepare_cmd;
pub use state_machine::{next_due, transition, DiscordReason, Effect, Event, NotifyKind, TerminalKind};
pub use types::*;
pub use upsert::apply_upsert;
pub use validate::{validate_create, BatchItemProblem, RejectReason};
