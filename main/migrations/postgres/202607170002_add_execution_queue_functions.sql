DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'previa_runner_queue') THEN
        CREATE ROLE previa_runner_queue NOLOGIN;
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION queue_assert_runner(
    p_runner_id UUID,
    p_runner_session_token TEXT
) RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM runner_instances
        WHERE id = p_runner_id
          AND session_token_hash = encode(digest(p_runner_session_token, 'sha256'), 'hex')
          AND status NOT IN ('stale', 'stopped')
    ) THEN
        RAISE EXCEPTION 'invalid runner session' USING ERRCODE = '28000';
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION queue_register_runner(
    p_name TEXT,
    p_pool TEXT,
    p_protocol_version INTEGER,
    p_version TEXT,
    p_supported_kinds TEXT[],
    p_capabilities_json JSONB,
    p_labels_json JSONB,
    p_max_e2e_slots INTEGER,
    p_max_load_slots INTEGER,
    p_heartbeat_interval_ms BIGINT
) RETURNS TABLE(runner_id UUID, runner_session_token TEXT)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_expected_protocol INTEGER;
    v_runner_id UUID := gen_random_uuid();
    v_token TEXT := encode(gen_random_bytes(32), 'hex');
BEGIN
    SELECT protocol_version INTO v_expected_protocol FROM queue_protocol WHERE id = 1;
    IF v_expected_protocol IS DISTINCT FROM p_protocol_version THEN
        RAISE EXCEPTION 'queue protocol mismatch: expected %, received %',
            v_expected_protocol, p_protocol_version USING ERRCODE = '22023';
    END IF;

    INSERT INTO runner_instances (
        id, name, session_token_hash, pool, protocol_version, version,
        supported_kinds, capabilities_json, labels_json,
        max_e2e_slots, max_load_slots, heartbeat_interval_ms,
        status, last_heartbeat_at
    ) VALUES (
        v_runner_id, p_name, encode(digest(v_token, 'sha256'), 'hex'),
        p_pool, p_protocol_version, p_version, p_supported_kinds,
        COALESCE(p_capabilities_json, '{}'::jsonb),
        COALESCE(p_labels_json, '{}'::jsonb),
        p_max_e2e_slots, p_max_load_slots, p_heartbeat_interval_ms,
        'ready', CURRENT_TIMESTAMP
    );

    RETURN QUERY SELECT v_runner_id, v_token;
END
$$;

CREATE OR REPLACE FUNCTION queue_heartbeat_runner(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_status TEXT
) RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    IF p_status NOT IN ('ready', 'busy', 'draining', 'stopped') THEN
        RAISE EXCEPTION 'invalid runner status' USING ERRCODE = '22023';
    END IF;

    UPDATE runner_instances
    SET status = p_status,
        last_heartbeat_at = CURRENT_TIMESTAMP,
        updated_at = CURRENT_TIMESTAMP
    WHERE id = p_runner_id;
    RETURN FOUND;
END
$$;

CREATE OR REPLACE FUNCTION queue_claim_job(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_lease_ms BIGINT
) RETURNS TABLE(
    job_id UUID,
    execution_id UUID,
    kind TEXT,
    shard_index INTEGER,
    payload_json JSONB,
    attempt INTEGER,
    lease_epoch BIGINT,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    IF p_lease_ms <= 0 THEN
        RAISE EXCEPTION 'lease duration must be positive' USING ERRCODE = '22023';
    END IF;

    RETURN QUERY
    WITH candidate AS (
        SELECT j.id
        FROM execution_jobs j
        JOIN executions e ON e.id = j.execution_id
        JOIN runner_instances r ON r.id = p_runner_id
        WHERE j.status = 'queued'
          AND j.available_at <= CURRENT_TIMESTAMP
          AND e.desired_status = 'running'
          AND j.pool = r.pool
          AND j.kind = ANY (r.supported_kinds)
          AND (
              j.requirements_json = '{}'::jsonb
              OR (r.labels_json || r.capabilities_json) @> j.requirements_json
          )
          AND (
              (j.kind = 'e2e' AND (
                  SELECT count(*)
                  FROM execution_jobs active
                  WHERE active.runner_id = r.id
                    AND active.kind = 'e2e'
                    AND active.status IN ('leased', 'running')
              ) < r.max_e2e_slots)
              OR
              (j.kind = 'load' AND (
                  SELECT count(*)
                  FROM execution_jobs active
                  WHERE active.runner_id = r.id
                    AND active.kind = 'load'
                    AND active.status IN ('leased', 'running')
              ) < r.max_load_slots)
          )
        ORDER BY j.priority DESC, j.created_at ASC
        FOR UPDATE OF j SKIP LOCKED
        LIMIT 1
    ),
    claimed AS (
        UPDATE execution_jobs j
        SET status = 'leased',
            attempt = j.attempt + 1,
            runner_id = p_runner_id,
            lease_epoch = j.lease_epoch + 1,
            lease_token = gen_random_uuid(),
            lease_expires_at = CURRENT_TIMESTAMP + (p_lease_ms * INTERVAL '1 millisecond'),
            started_at = COALESCE(j.started_at, CURRENT_TIMESTAMP),
            updated_at = CURRENT_TIMESTAMP
        FROM candidate
        WHERE j.id = candidate.id
        RETURNING j.*
    )
    SELECT c.id, c.execution_id, c.kind, c.shard_index, c.payload_json,
           c.attempt, c.lease_epoch, c.lease_token, c.lease_expires_at
    FROM claimed c;
END
$$;

CREATE OR REPLACE FUNCTION queue_renew_job_lease(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID,
    p_lease_ms BIGINT
) RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    UPDATE execution_jobs
    SET status = 'running',
        lease_expires_at = CURRENT_TIMESTAMP + (p_lease_ms * INTERVAL '1 millisecond'),
        updated_at = CURRENT_TIMESTAMP
    WHERE id = p_job_id
      AND runner_id = p_runner_id
      AND attempt = p_attempt
      AND lease_epoch = p_lease_epoch
      AND lease_token = p_lease_token
      AND status IN ('leased', 'running')
      AND lease_expires_at > CURRENT_TIMESTAMP;
    RETURN FOUND;
END
$$;

CREATE OR REPLACE FUNCTION queue_publish_events(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID,
    p_events JSONB
) RETURNS BIGINT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_execution_id UUID;
    v_inserted BIGINT;
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    IF jsonb_typeof(p_events) <> 'array' THEN
        RAISE EXCEPTION 'events must be a JSON array' USING ERRCODE = '22023';
    END IF;

    SELECT execution_id INTO v_execution_id
    FROM execution_jobs
    WHERE id = p_job_id
      AND runner_id = p_runner_id
      AND attempt = p_attempt
      AND lease_epoch = p_lease_epoch
      AND lease_token = p_lease_token
      AND status IN ('leased', 'running')
      AND lease_expires_at > CURRENT_TIMESTAMP
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'stale job fencing token' USING ERRCODE = '40001';
    END IF;

    INSERT INTO execution_events (
        execution_id, job_id, runner_id, attempt, lease_epoch,
        seq, event_type, elapsed_ms, payload_json
    )
    SELECT v_execution_id, p_job_id, p_runner_id, p_attempt, p_lease_epoch,
           event.seq, event.event_type, event.elapsed_ms, event.payload_json
    FROM jsonb_to_recordset(p_events) AS event(
        seq BIGINT,
        event_type TEXT,
        elapsed_ms BIGINT,
        payload_json JSONB
    )
    ON CONFLICT (job_id, attempt, seq) DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;

    IF v_inserted > 0 THEN
        PERFORM pg_notify('previa_events', v_execution_id::TEXT);
    END IF;
    RETURN v_inserted;
END
$$;

CREATE OR REPLACE FUNCTION queue_complete_job(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID,
    p_result_json JSONB
) RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    UPDATE execution_jobs
    SET status = 'completed',
        result_json = COALESCE(p_result_json, '{}'::jsonb),
        finished_at = CURRENT_TIMESTAMP,
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = CURRENT_TIMESTAMP
    WHERE id = p_job_id
      AND runner_id = p_runner_id
      AND attempt = p_attempt
      AND lease_epoch = p_lease_epoch
      AND lease_token = p_lease_token
      AND status IN ('leased', 'running')
      AND lease_expires_at > CURRENT_TIMESTAMP;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'stale job fencing token' USING ERRCODE = '40001';
    END IF;
    PERFORM pg_notify('previa_events', p_job_id::TEXT);
    RETURN TRUE;
END
$$;

CREATE OR REPLACE FUNCTION queue_fail_job(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID,
    p_error TEXT,
    p_result_json JSONB,
    p_retryable BOOLEAN,
    p_backoff_base_ms BIGINT,
    p_backoff_max_ms BIGINT
) RETURNS TEXT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_attempt INTEGER;
    v_max_attempts INTEGER;
    v_status TEXT;
    v_delay_ms BIGINT;
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    SELECT attempt, max_attempts INTO v_attempt, v_max_attempts
    FROM execution_jobs
    WHERE id = p_job_id
      AND runner_id = p_runner_id
      AND attempt = p_attempt
      AND lease_epoch = p_lease_epoch
      AND lease_token = p_lease_token
      AND status IN ('leased', 'running')
      AND lease_expires_at > CURRENT_TIMESTAMP
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'stale job fencing token' USING ERRCODE = '40001';
    END IF;

    IF p_retryable AND v_attempt < v_max_attempts THEN
        v_status := 'retry_wait';
        v_delay_ms := LEAST(
            p_backoff_base_ms * (2::BIGINT ^ GREATEST(v_attempt - 1, 0)),
            p_backoff_max_ms
        );
    ELSIF p_retryable THEN
        v_status := 'dead_letter';
    ELSE
        v_status := 'failed';
    END IF;

    UPDATE execution_jobs
    SET status = v_status,
        result_json = p_result_json,
        last_error = p_error,
        available_at = CASE
            WHEN v_status = 'retry_wait'
                THEN CURRENT_TIMESTAMP + (v_delay_ms * INTERVAL '1 millisecond')
            ELSE available_at
        END,
        finished_at = CASE
            WHEN v_status IN ('failed', 'dead_letter') THEN CURRENT_TIMESTAMP
            ELSE NULL
        END,
        runner_id = CASE WHEN v_status = 'retry_wait' THEN NULL ELSE runner_id END,
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = CURRENT_TIMESTAMP
    WHERE id = p_job_id;
    PERFORM pg_notify('previa_events', p_job_id::TEXT);
    RETURN v_status;
END
$$;

CREATE OR REPLACE FUNCTION queue_acknowledge_cancellation(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID,
    p_result_json JSONB
) RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    UPDATE execution_jobs j
    SET status = 'cancelled',
        result_json = p_result_json,
        finished_at = CURRENT_TIMESTAMP,
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = CURRENT_TIMESTAMP
    FROM executions e
    WHERE j.id = p_job_id
      AND e.id = j.execution_id
      AND e.desired_status = 'cancelled'
      AND j.runner_id = p_runner_id
      AND j.attempt = p_attempt
      AND j.lease_epoch = p_lease_epoch
      AND j.lease_token = p_lease_token
      AND j.status IN ('leased', 'running');
    IF NOT FOUND THEN
        RAISE EXCEPTION 'stale fencing token or cancellation not requested'
            USING ERRCODE = '40001';
    END IF;
    RETURN TRUE;
END
$$;

CREATE OR REPLACE FUNCTION queue_read_control(
    p_runner_id UUID,
    p_runner_session_token TEXT,
    p_job_id UUID,
    p_attempt INTEGER,
    p_lease_epoch BIGINT,
    p_lease_token UUID
) RETURNS TEXT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_desired_status TEXT;
BEGIN
    PERFORM queue_assert_runner(p_runner_id, p_runner_session_token);
    SELECT e.desired_status INTO v_desired_status
    FROM execution_jobs j
    JOIN executions e ON e.id = j.execution_id
    WHERE j.id = p_job_id
      AND j.runner_id = p_runner_id
      AND j.attempt = p_attempt
      AND j.lease_epoch = p_lease_epoch
      AND j.lease_token = p_lease_token
      AND j.status IN ('leased', 'running');
    IF NOT FOUND THEN
        RAISE EXCEPTION 'stale job fencing token' USING ERRCODE = '40001';
    END IF;
    RETURN v_desired_status;
END
$$;

REVOKE ALL ON queue_protocol, runner_instances, executions, execution_jobs,
    execution_events, execution_snapshots FROM previa_runner_queue;
REVOKE ALL ON FUNCTION queue_assert_runner(UUID, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_register_runner(
    TEXT, TEXT, INTEGER, TEXT, TEXT[], JSONB, JSONB, INTEGER, INTEGER, BIGINT
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_heartbeat_runner(UUID, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_claim_job(UUID, TEXT, BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_renew_job_lease(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, BIGINT
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_publish_events(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_complete_job(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_fail_job(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, TEXT, JSONB, BOOLEAN, BIGINT, BIGINT
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_acknowledge_cancellation(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) FROM PUBLIC;
REVOKE ALL ON FUNCTION queue_read_control(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID
) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION queue_register_runner(
    TEXT, TEXT, INTEGER, TEXT, TEXT[], JSONB, JSONB, INTEGER, INTEGER, BIGINT
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_heartbeat_runner(UUID, TEXT, TEXT)
    TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_claim_job(UUID, TEXT, BIGINT)
    TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_renew_job_lease(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, BIGINT
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_publish_events(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_complete_job(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_fail_job(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, TEXT, JSONB, BOOLEAN, BIGINT, BIGINT
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_acknowledge_cancellation(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID, JSONB
) TO previa_runner_queue;
GRANT EXECUTE ON FUNCTION queue_read_control(
    UUID, TEXT, UUID, INTEGER, BIGINT, UUID
) TO previa_runner_queue;
