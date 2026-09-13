CREATE TABLE chore_catalog_provenance (
    chore_id TEXT PRIMARY KEY NOT NULL REFERENCES chores(id) ON DELETE CASCADE,
    template_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    catalog_version INTEGER NOT NULL CHECK (catalog_version > 0),
    locale TEXT NOT NULL CHECK (length(trim(locale)) > 0),
    created_at TEXT NOT NULL
);

CREATE INDEX idx_chore_catalog_provenance_template
    ON chore_catalog_provenance(template_id);
