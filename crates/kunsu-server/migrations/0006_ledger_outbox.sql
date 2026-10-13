-- 帳本待補寫佇列（CONTRACT §7、INV-13）：vault 帳本寫不進去（權限、逾時）時，該行先存這裡，之後每次寫帳本前依序補寫。
-- 不屬於可由帳本重建的資料：rebuild-db 不清，而且佇列沒清空前拒絕重建。
CREATE TABLE ledger_outbox (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    row_json   TEXT NOT NULL,
    created_at TEXT NOT NULL
);
