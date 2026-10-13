-- 錢的結果以每一次授權（nonce）追蹤（CONTRACT §7）：每個簽出的 nonce 最後的錢鏈結果
-- （COMMIT_OK／EXEC_BLOCKED／EXEC_UNKNOWN／RECONCILED_*）。已被 executor 消耗卻沒有結果、或結果是 EXEC_UNKNOWN＝結果不明。
-- 可由帳本 execution 行的 money_outcome 重建。
CREATE TABLE money_outcomes (
    nonce       TEXT PRIMARY KEY,
    question_id TEXT NOT NULL,
    result      TEXT NOT NULL,
    recorded_at TEXT NOT NULL
);

CREATE INDEX idx_money_outcomes_question ON money_outcomes (question_id);
