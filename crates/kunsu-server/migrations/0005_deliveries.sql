-- 回答延續原對話（PRD §20、BDD conversation-continuity）：每個答案一列交付紀錄。
-- state：pending | held（可再變）；live_waiter | resumed | picked_up | fresh | no_continuation | dead（最終，永不改變）。
CREATE TABLE deliveries (
    answer_id     TEXT PRIMARY KEY,
    question_id   TEXT NOT NULL,
    state         TEXT NOT NULL,
    reason        TEXT,
    attempts      INTEGER NOT NULL DEFAULT 0,
    next_check_at INTEGER,          -- epoch 毫秒（與 questions.next_transition_at 同慣例）
    updated_at    TEXT NOT NULL
);
CREATE INDEX deliveries_due ON deliveries (state, next_check_at);
