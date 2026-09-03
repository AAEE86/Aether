-- Historical HTTP captures predate the deny-by-default persistence policy and
-- may contain credentials or request content. Keep the usage and projected
-- billing facts, but remove every legacy copy of the raw exchange.
UPDATE `usage` AS usage_rows
LEFT JOIN usage_cost_reservations AS reservation
  ON reservation.state = 'reserved'
 AND REGEXP_LIKE(
     reservation.reservation_token,
     '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
     'c'
 )
 AND reservation.request_id = usage_rows.request_id
 AND reservation.subject_id = usage_rows.user_id
 AND reservation.reservation_token = JSON_UNQUOTE(JSON_EXTRACT(
     CASE
       WHEN JSON_VALID(usage_rows.request_metadata) = 1
       THEN usage_rows.request_metadata
       ELSE JSON_OBJECT()
     END,
     '$.plan_usage_reservation_token'
 ))
SET usage_rows.error_message = NULL,
    usage_rows.error_category = CASE
        WHEN usage_rows.error_category IS NULL
          OR TRIM(usage_rows.error_category) = '' THEN NULL
        WHEN LOWER(TRIM(usage_rows.error_category)) IN (
            'auth', 'cancelled', 'client_error', 'http_error',
            'non_success_status', 'provider_error', 'rate_limit', 'redirect',
            'server_error', 'stream_missing_terminal_event',
            'stream_terminal_error', 'upstream_error'
        ) THEN LOWER(TRIM(usage_rows.error_category))
        ELSE 'other_error'
    END,
    usage_rows.request_headers = NULL,
    usage_rows.request_body = NULL,
    usage_rows.provider_request_headers = NULL,
    usage_rows.provider_request_body = NULL,
    usage_rows.response_headers = NULL,
    usage_rows.response_body = NULL,
    usage_rows.client_response_headers = NULL,
    usage_rows.client_response_body = NULL,
    usage_rows.request_body_compressed = NULL,
    usage_rows.provider_request_body_compressed = NULL,
    usage_rows.response_body_compressed = NULL,
    usage_rows.client_response_body_compressed = NULL,
    usage_rows.request_metadata = CASE
        WHEN reservation.reservation_token IS NULL THEN NULL
        WHEN JSON_TYPE(JSON_EXTRACT(
            CASE
              WHEN JSON_VALID(usage_rows.request_metadata) = 1
              THEN usage_rows.request_metadata
              ELSE JSON_OBJECT()
            END,
            '$.plan_usage_reservation_deferred'
        )) = 'BOOLEAN'
          AND JSON_UNQUOTE(JSON_EXTRACT(
            CASE
              WHEN JSON_VALID(usage_rows.request_metadata) = 1
              THEN usage_rows.request_metadata
              ELSE JSON_OBJECT()
            END,
            '$.plan_usage_reservation_deferred'
          )) = 'true'
        THEN JSON_SET(
            JSON_OBJECT(
                'plan_usage_reservation_token', reservation.reservation_token
            ),
            '$.plan_usage_reservation_deferred',
            JSON_EXTRACT('true', '$')
        )
        ELSE JSON_OBJECT(
            'plan_usage_reservation_token', reservation.reservation_token
        )
    END
WHERE usage_rows.error_message IS NOT NULL
   OR usage_rows.error_category IS NOT NULL
   OR usage_rows.request_headers IS NOT NULL
   OR usage_rows.request_body IS NOT NULL
   OR usage_rows.provider_request_headers IS NOT NULL
   OR usage_rows.provider_request_body IS NOT NULL
   OR usage_rows.response_headers IS NOT NULL
   OR usage_rows.response_body IS NOT NULL
   OR usage_rows.client_response_headers IS NOT NULL
   OR usage_rows.client_response_body IS NOT NULL
   OR usage_rows.request_body_compressed IS NOT NULL
   OR usage_rows.provider_request_body_compressed IS NOT NULL
   OR usage_rows.response_body_compressed IS NOT NULL
   OR usage_rows.client_response_body_compressed IS NOT NULL
   OR usage_rows.request_metadata IS NOT NULL;

DELETE FROM usage_http_audits;

DELETE FROM usage_body_blobs;

-- All fields needed by billing reads are projected into typed columns on this
-- table. The historical JSON documents include rule expressions, catalog
-- snapshots, and arbitrary dimensions, so they are not retained.
UPDATE usage_settlement_snapshots
SET settlement_snapshot = NULL,
    billing_dimensions = NULL
WHERE settlement_snapshot IS NOT NULL
   OR billing_dimensions IS NOT NULL;

UPDATE video_tasks
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
