CREATE INDEX idx_request_candidates_key_created
    ON request_candidates (key_id, created_at DESC, id ASC);

ALTER TABLE request_candidates
    ADD INDEX idx_request_candidates_provider_runtime_activity
        (provider_id, started_at DESC, created_at DESC, id ASC),
    ADD INDEX idx_request_candidates_key_runtime_activity
        (key_id, started_at DESC, created_at DESC, id ASC),
    ADD INDEX idx_request_candidates_api_key_runtime_activity
        (api_key_id, started_at DESC, created_at DESC, id ASC);
