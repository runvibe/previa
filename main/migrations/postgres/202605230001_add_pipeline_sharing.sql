ALTER TABLE pipelines ADD COLUMN IF NOT EXISTS owner_user_id TEXT NOT NULL DEFAULT 'anonymous';
ALTER TABLE pipelines ADD COLUMN IF NOT EXISTS owner_username TEXT NOT NULL DEFAULT 'anonymous';
ALTER TABLE pipelines ADD COLUMN IF NOT EXISTS visibility TEXT NOT NULL DEFAULT 'private';

CREATE INDEX IF NOT EXISTS idx_pipelines_owner
    ON pipelines(owner_user_id);

CREATE INDEX IF NOT EXISTS idx_pipelines_visibility
    ON pipelines(project_id, visibility);

CREATE TABLE IF NOT EXISTS pipeline_shares (
    id TEXT PRIMARY KEY NOT NULL,
    pipeline_id TEXT NOT NULL REFERENCES pipelines(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    access_level TEXT NOT NULL DEFAULT 'editor',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    UNIQUE(pipeline_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_pipeline_shares_pipeline
    ON pipeline_shares(pipeline_id);

CREATE INDEX IF NOT EXISTS idx_pipeline_shares_user
    ON pipeline_shares(user_id);
