-- Kunsu server schema（CONTRACT §10 儲存路徑；SQLite 是可重建的運行索引，
-- vault ledger 才是真相層——本 DB 可整顆刪除後由 rebuild_db 重建）。
-- 單一寫者假設：不做行鎖設計，寫入序列化在服務層。

-- 題目主表：全量 Question 存 JSON 快照（ledger 快照同構），
-- 熱查詢欄位抽出成欄（state / next_transition_at / kongko_ref / source_key）。
CREATE TABLE questions (
    id                 TEXT PRIMARY KEY,
    data               TEXT NOT NULL,             -- Question 全量 JSON（serde snake_case）
    state              TEXT NOT NULL,
    tier               TEXT NOT NULL,
    next_transition_at INTEGER NOT NULL,          -- unix millis UTC（timer due 掃描比較用）
    kongko_ref         TEXT,
    source_key         TEXT NOT NULL,             -- per-source 配額鍵（agent:routine|session|cwd）
    internal           INTEGER NOT NULL DEFAULT 0,
    created_date_tpe   TEXT NOT NULL,             -- 台北本地日期（配額按日計數用）
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE INDEX idx_questions_due   ON questions (state, next_transition_at);
CREATE INDEX idx_questions_kongko ON questions (kongko_ref);
CREATE INDEX idx_questions_quota ON questions (source_key, created_date_tpe);

-- 事件流：狀態轉移史 + 終態事件（answers_since 游標收割面）+ fatigue /
-- forced 預算消耗等遙測。seq 單調遞增即游標。
CREATE TABLE events (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    question_id TEXT,
    kind        TEXT NOT NULL,                    -- state_changed | terminal | fatigue | forced_budget | kongko_event | ...
    payload     TEXT NOT NULL,                    -- JSON
    date_tpe    TEXT NOT NULL,                    -- 台北本地日期（每日預算/配額統計用）
    at          TEXT NOT NULL
);

CREATE INDEX idx_events_kind_date ON events (kind, date_tpe);
CREATE INDEX idx_events_question  ON events (question_id);

-- per-consumer 游標（CONTRACT §4 ask_answers_since；at-least-once）。
CREATE TABLE consumers (
    consumer TEXT PRIMARY KEY,
    cursor   INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- reactor 串行 queue（本階段只入列與回寫回執；真正 spawn 是下一階段）。
CREATE TABLE reactor_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    question_id TEXT NOT NULL,
    answer_id   TEXT NOT NULL,                    -- 冪等鍵（同 answer 永不重複執行）
    status      TEXT NOT NULL,                    -- queued | dry_run | running | done | dead
    attempts    INTEGER NOT NULL DEFAULT 0,
    result      TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (answer_id)
);

-- T3 standing policies 鏡像（正本在 vault logs/asks/policies/<slug>.md；
-- 比對引擎在 agent 端，server 只存已知政策供 veto 窗與留痕）。
CREATE TABLE policies (
    policy_id           TEXT PRIMARY KEY,
    data                TEXT NOT NULL,            -- frontmatter JSON 鏡像
    signed_at           TEXT,
    signing_question_id TEXT,
    revoked_at          TEXT
);

-- kongko 發佈事件冪等記錄（P1 定案 5：同 batchId/publicationId 重複事件 no-op）。
CREATE TABLE kongko_events (
    publication_id TEXT PRIMARY KEY,
    batch_id       TEXT NOT NULL,
    payload        TEXT NOT NULL,
    received_at    TEXT NOT NULL
);
