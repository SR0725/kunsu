-- Persistent singleton settings. Values are versionable JSON so presentation policy can evolve
-- without coupling question truth to an app-specific table shape.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
