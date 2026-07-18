CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE queue_protocol (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    protocol_version INTEGER NOT NULL CHECK (protocol_version > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO queue_protocol (id, protocol_version, updated_at)
VALUES (1, 1, CURRENT_TIMESTAMP)
ON CONFLICT (id) DO UPDATE
SET protocol_version = EXCLUDED.protocol_version,
    updated_at = EXCLUDED.updated_at;

CREATE TABLE runner_instances (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    session_token_hash TEXT NOT NULL,
    pool TEXT NOT NULL,
    protocol_version INTEGER NOT NULL,
    version TEXT NOT NULL,
    supported_kinds TEXT[] NOT NULL,
    capabilities_json JSONB NOT NULL,
    labels_json JSONB NOT NULL,
    max_e2e_slots INTEGER NOT NULL CHECK (max_e2e_slots >= 0),
    max_load_slots INTEGER NOT NULL CHECK (max_load_slots >= 0),
    heartbeat_interval_ms BIGINT NOT NULL CHECK (heartbeat_interval_ms > 0),
    status TEXT NOT NULL CHECK (status IN ('ready', 'busy', 'draining', 'stale', 'stopped')),
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (supported_kinds <@ ARRAY['e2e', 'load']::TEXT[]),
    CHECK (cardinality(supported_kinds) > 0),
    CHECK (jsonb_typeof(capabilities_json) = 'object'),
    CHECK (jsonb_typeof(labels_json) = 'object')
);

CREATE INDEX idx_runner_instances_status_pool_heartbeat
    ON runner_instances(status, pool, last_heartbeat_at);
CREATE INDEX idx_runner_instances_protocol_status
    ON runner_instances(protocol_version, status);

CREATE TABLE executions (
    id UUID PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    pipeline_id TEXT REFERENCES pipelines(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('e2e', 'load')),
    status TEXT NOT NULL CHECK (
        status IN ('queued', 'running', 'cancel_requested', 'completed', 'failed', 'cancelled')
    ),
    desired_status TEXT NOT NULL CHECK (desired_status IN ('running', 'cancelled')),
    request_json JSONB NOT NULL,
    shard_count INTEGER NOT NULL CHECK (shard_count > 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    created_by TEXT NOT NULL,
    transaction_id TEXT,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((kind = 'e2e' AND shard_count = 1) OR kind = 'load'),
    CHECK (jsonb_typeof(request_json) = 'object'),
    CHECK ((status IN ('completed', 'failed', 'cancelled')) = (finished_at IS NOT NULL))
);

CREATE INDEX idx_executions_project_created
    ON executions(project_id, created_at DESC);
CREATE INDEX idx_executions_status_queued
    ON executions(status, queued_at);
CREATE INDEX idx_executions_kind_status_created
    ON executions(kind, status, created_at);

CREATE TABLE execution_jobs (
    id UUID PRIMARY KEY,
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('e2e', 'load')),
    shard_index INTEGER,
    pool TEXT NOT NULL,
    requirements_json JSONB NOT NULL,
    payload_json JSONB NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK (
        status IN (
            'queued', 'leased', 'running', 'retry_wait',
            'completed', 'failed', 'cancelled', 'dead_letter'
        )
    ),
    available_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    runner_id UUID REFERENCES runner_instances(id) ON DELETE SET NULL,
    lease_epoch BIGINT NOT NULL DEFAULT 0 CHECK (lease_epoch >= 0),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    result_json JSONB,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (attempt <= max_attempts),
    CHECK (
        (kind = 'e2e' AND shard_index IS NULL)
        OR (kind = 'load' AND shard_index IS NOT NULL AND shard_index >= 0)
    ),
    CHECK (jsonb_typeof(requirements_json) = 'object'),
    CHECK (jsonb_typeof(payload_json) = 'object'),
    CHECK (result_json IS NULL OR jsonb_typeof(result_json) = 'object'),
    CHECK (
        (status IN ('leased', 'running')
            AND runner_id IS NOT NULL
            AND lease_token IS NOT NULL
            AND lease_expires_at IS NOT NULL)
        OR
        (status NOT IN ('leased', 'running')
            AND lease_token IS NULL
            AND lease_expires_at IS NULL)
    ),
    CHECK ((status IN ('completed', 'failed', 'cancelled', 'dead_letter')) = (finished_at IS NOT NULL))
);

CREATE UNIQUE INDEX uq_execution_jobs_load_shard
    ON execution_jobs(execution_id, shard_index)
    WHERE kind = 'load';
CREATE UNIQUE INDEX uq_execution_jobs_e2e
    ON execution_jobs(execution_id)
    WHERE kind = 'e2e';
CREATE INDEX idx_execution_jobs_claim
    ON execution_jobs(status, available_at, priority DESC, created_at);
CREATE INDEX idx_execution_jobs_pool_kind_claim
    ON execution_jobs(pool, kind, status, available_at);
CREATE INDEX idx_execution_jobs_runner_status
    ON execution_jobs(runner_id, status);
CREATE INDEX idx_execution_jobs_execution_status
    ON execution_jobs(execution_id, status);
CREATE INDEX idx_execution_jobs_active_lease
    ON execution_jobs(lease_expires_at)
    WHERE status IN ('leased', 'running');

CREATE TABLE execution_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    job_id UUID NOT NULL REFERENCES execution_jobs(id) ON DELETE CASCADE,
    runner_id UUID NOT NULL REFERENCES runner_instances(id) ON DELETE RESTRICT,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    lease_epoch BIGINT NOT NULL CHECK (lease_epoch > 0),
    seq BIGINT NOT NULL CHECK (seq >= 0),
    event_type TEXT NOT NULL,
    elapsed_ms BIGINT NOT NULL CHECK (elapsed_ms >= 0),
    payload_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (job_id, attempt, seq),
    CHECK (octet_length(payload_json::TEXT) <= 1048576)
);

CREATE INDEX idx_execution_events_execution_id
    ON execution_events(execution_id, id);
CREATE INDEX idx_execution_events_job_attempt_seq
    ON execution_events(job_id, attempt, seq);
CREATE INDEX idx_execution_events_execution_type
    ON execution_events(execution_id, event_type, id);

CREATE TABLE execution_snapshots (
    execution_id UUID PRIMARY KEY REFERENCES executions(id) ON DELETE CASCADE,
    version BIGINT NOT NULL DEFAULT 0 CHECK (version >= 0),
    last_event_id BIGINT NOT NULL DEFAULT 0 CHECK (last_event_id >= 0),
    status TEXT NOT NULL CHECK (
        status IN ('queued', 'running', 'cancel_requested', 'completed', 'failed', 'cancelled')
    ),
    snapshot_json JSONB NOT NULL,
    projection_owner UUID,
    projection_lease_epoch BIGINT NOT NULL DEFAULT 0 CHECK (projection_lease_epoch >= 0),
    projection_lease_expires_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (jsonb_typeof(snapshot_json) = 'object'),
    CHECK (
        (projection_owner IS NULL AND projection_lease_expires_at IS NULL)
        OR (projection_owner IS NOT NULL AND projection_lease_expires_at IS NOT NULL)
    )
);

CREATE INDEX idx_execution_snapshots_projection_lease
    ON execution_snapshots(projection_lease_expires_at);
