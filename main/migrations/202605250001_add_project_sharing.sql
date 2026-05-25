ALTER TABLE projects ADD COLUMN owner_user_id TEXT NOT NULL DEFAULT 'anonymous';
ALTER TABLE projects ADD COLUMN owner_username TEXT NOT NULL DEFAULT 'anonymous';
ALTER TABLE projects ADD COLUMN visibility TEXT NOT NULL DEFAULT 'private';

CREATE INDEX IF NOT EXISTS idx_projects_owner
    ON projects(owner_user_id);

CREATE INDEX IF NOT EXISTS idx_projects_visibility
    ON projects(visibility);

CREATE TABLE IF NOT EXISTS project_shares (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    access_level TEXT NOT NULL DEFAULT 'editor',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(project_id, user_id),
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_shares_project
    ON project_shares(project_id);

CREATE INDEX IF NOT EXISTS idx_project_shares_user
    ON project_shares(user_id);
