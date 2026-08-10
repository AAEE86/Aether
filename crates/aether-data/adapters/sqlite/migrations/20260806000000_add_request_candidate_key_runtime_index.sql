CREATE INDEX IF NOT EXISTS idx_request_candidates_key_created
    ON request_candidates (key_id, created_at DESC, id ASC);

CREATE INDEX IF NOT EXISTS idx_request_candidates_provider_runtime_activity
    ON request_candidates (provider_id, COALESCE(started_at, created_at) DESC, id ASC);

CREATE INDEX IF NOT EXISTS idx_request_candidates_key_runtime_activity
    ON request_candidates (key_id, COALESCE(started_at, created_at) DESC, id ASC);

CREATE INDEX IF NOT EXISTS idx_request_candidates_api_key_runtime_activity
    ON request_candidates (api_key_id, COALESCE(started_at, created_at) DESC, id ASC);
