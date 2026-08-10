CREATE INDEX IF NOT EXISTS idx_request_candidates_key_created
    ON public.request_candidates USING btree (key_id, created_at DESC, id ASC);

-- Runtime admission filters on the attempt start time, falling back to the
-- materialization time for rows that have not started yet. Match that exact
-- expression so the five-minute window does not scan a key's full history.
CREATE INDEX IF NOT EXISTS idx_request_candidates_provider_runtime_activity
    ON public.request_candidates USING btree (
        provider_id, (COALESCE(started_at, created_at)) DESC, id ASC
    );

CREATE INDEX IF NOT EXISTS idx_request_candidates_key_runtime_activity
    ON public.request_candidates USING btree (
        key_id, (COALESCE(started_at, created_at)) DESC, id ASC
    );

CREATE INDEX IF NOT EXISTS idx_request_candidates_api_key_runtime_activity
    ON public.request_candidates USING btree (
        api_key_id, (COALESCE(started_at, created_at)) DESC, id ASC
    );
