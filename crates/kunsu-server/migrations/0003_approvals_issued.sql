-- 批准簽發登記簿（Codex 覆檢 P1：verify 強化）。
-- 簽章鏈簽出 approval record 當下登記 nonce/answer_id/token/param_hash/時間；
-- /approvals/verify 除驗簽外必須比對本表——只有 server 自己簽發過的 record
-- 才可能被消耗，杜絕「持 key 者離線偽造合法 record」。
-- 刻意不從 ledger rebuild（fail closed：重建後未消耗的舊授權一律作廢，
-- Ray 在 UI 重新授權即可）。
CREATE TABLE approvals_issued (
    nonce          TEXT PRIMARY KEY,
    question_id    TEXT NOT NULL,
    answer_id      TEXT NOT NULL,
    executor_token TEXT NOT NULL,
    param_hash     TEXT NOT NULL,
    answered_at    INTEGER NOT NULL,  -- epoch 秒（record.answered_at 同值）
    issued_at      TEXT NOT NULL
);

CREATE INDEX idx_approvals_issued_question ON approvals_issued (question_id);
