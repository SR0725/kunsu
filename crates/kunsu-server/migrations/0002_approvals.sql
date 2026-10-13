-- 批准單次消耗表（CONTRACT §5 /approvals/verify；INV-9 inv_approval_single_use）。
-- nonce 是 record 唯一鍵：同一 record 第二次 verify 必 valid=false。
-- vault ledger 的 approval_consumed 行是真相層；本表可由 rebuild-db 重建
-- （單次性判定不因重建失效——survival.md 場景 5）。
CREATE TABLE approvals_consumed (
    nonce       TEXT PRIMARY KEY,
    question_id TEXT NOT NULL,
    record      TEXT NOT NULL,      -- approval record 全量 JSON（含 hmac）
    consumed_at TEXT NOT NULL
);

CREATE INDEX idx_approvals_question ON approvals_consumed (question_id);
