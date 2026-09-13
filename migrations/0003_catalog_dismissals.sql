CREATE TABLE catalog_dismissals (
    template_id TEXT PRIMARY KEY NOT NULL CHECK (length(trim(template_id)) > 0),
    dismissed_at TEXT NOT NULL
);
