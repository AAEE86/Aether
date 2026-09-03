-- Historical HTTP captures predate the deny-by-default persistence policy and
-- may contain credentials or request content. Keep the usage and projected
-- billing facts, but remove every legacy copy of the raw exchange.
UPDATE public.usage AS usage_rows
SET error_message = NULL,
    error_category = CASE
        WHEN usage_rows.error_category IS NULL
          OR BTRIM(usage_rows.error_category) = '' THEN NULL
        WHEN LOWER(BTRIM(usage_rows.error_category)) IN (
            'auth', 'cancelled', 'client_error', 'http_error',
            'non_success_status', 'provider_error', 'rate_limit', 'redirect',
            'server_error', 'stream_missing_terminal_event',
            'stream_terminal_error', 'upstream_error'
        ) THEN LOWER(BTRIM(usage_rows.error_category))
        ELSE 'other_error'
    END,
    request_headers = NULL,
    request_body = NULL,
    provider_request_headers = NULL,
    provider_request_body = NULL,
    response_headers = NULL,
    response_body = NULL,
    client_response_headers = NULL,
    client_response_body = NULL,
    request_body_compressed = NULL,
    provider_request_body_compressed = NULL,
    response_body_compressed = NULL,
    client_response_body_compressed = NULL,
    request_metadata = (
        SELECT JSONB_STRIP_NULLS(JSONB_BUILD_OBJECT(
            'plan_usage_reservation_token', reservation.reservation_token,
            'plan_usage_reservation_deferred', CASE
                WHEN usage_rows.request_metadata::jsonb
                     -> 'plan_usage_reservation_deferred' = 'true'::jsonb
                THEN TRUE
                ELSE NULL
            END
        ))::json
        FROM public.usage_cost_reservations AS reservation
        WHERE reservation.state = 'reserved'
          AND reservation.reservation_token ~
              '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
          AND reservation.request_id = usage_rows.request_id
          AND reservation.subject_id = usage_rows.user_id
          AND reservation.reservation_token = usage_rows.request_metadata
              ->> 'plan_usage_reservation_token'
        LIMIT 1
    )
WHERE error_message IS NOT NULL
   OR error_category IS NOT NULL
   OR request_headers IS NOT NULL
   OR request_body IS NOT NULL
   OR provider_request_headers IS NOT NULL
   OR provider_request_body IS NOT NULL
   OR response_headers IS NOT NULL
   OR response_body IS NOT NULL
   OR client_response_headers IS NOT NULL
   OR client_response_body IS NOT NULL
   OR request_body_compressed IS NOT NULL
   OR provider_request_body_compressed IS NOT NULL
   OR response_body_compressed IS NOT NULL
   OR client_response_body_compressed IS NOT NULL
   OR request_metadata IS NOT NULL;

DELETE FROM public.usage_http_audits;

DELETE FROM public.usage_body_blobs;

-- All fields needed by billing reads are projected into typed columns on this
-- table. The historical JSON documents include rule expressions, catalog
-- snapshots, and arbitrary dimensions, so they are not retained.
UPDATE public.usage_settlement_snapshots
SET settlement_snapshot = NULL,
    billing_dimensions = NULL
WHERE settlement_snapshot IS NOT NULL
   OR billing_dimensions IS NOT NULL;

UPDATE public.video_tasks
SET original_request_body = NULL,
    converted_request_body = NULL,
    progress_message = NULL,
    error_message = NULL,
    request_metadata = NULL,
    video_url = NULL,
    video_urls = NULL,
    thumbnail_url = NULL,
    stored_video_path = NULL,
    webhook_url = NULL
WHERE original_request_body IS NOT NULL
   OR converted_request_body IS NOT NULL
   OR progress_message IS NOT NULL
   OR error_message IS NOT NULL
   OR request_metadata IS NOT NULL
   OR video_url IS NOT NULL
   OR video_urls IS NOT NULL
   OR thumbnail_url IS NOT NULL
   OR stored_video_path IS NOT NULL
   OR webhook_url IS NOT NULL;
