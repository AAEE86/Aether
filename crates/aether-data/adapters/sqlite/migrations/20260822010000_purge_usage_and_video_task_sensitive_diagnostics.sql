-- Historical HTTP captures predate the deny-by-default persistence policy and
-- may contain credentials or request content. Keep the usage and projected
-- billing facts, but remove every legacy copy of the raw exchange.
UPDATE "usage"
SET error_message = NULL,
    error_category = CASE
        WHEN error_category IS NULL OR TRIM(error_category) = '' THEN NULL
        WHEN LOWER(TRIM(error_category)) IN (
            'auth', 'cancelled', 'client_error', 'http_error',
            'non_success_status', 'provider_error', 'rate_limit', 'redirect',
            'server_error', 'stream_missing_terminal_event',
            'stream_terminal_error', 'upstream_error'
        ) THEN LOWER(TRIM(error_category))
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
        SELECT CASE
            WHEN JSON_TYPE(
                CASE
                  WHEN JSON_VALID("usage".request_metadata)
                  THEN "usage".request_metadata
                  ELSE '{}'
                END,
                '$.plan_usage_reservation_deferred'
            ) = 'true'
            THEN JSON_SET(
                JSON_OBJECT(
                    'plan_usage_reservation_token', reservation.reservation_token
                ),
                '$.plan_usage_reservation_deferred',
                JSON('true')
            )
            ELSE JSON_OBJECT(
                'plan_usage_reservation_token', reservation.reservation_token
            )
        END
        FROM usage_cost_reservations AS reservation
        WHERE reservation.state = 'reserved'
          AND LENGTH(reservation.reservation_token) = 36
          AND reservation.reservation_token = LOWER(reservation.reservation_token)
          AND SUBSTR(reservation.reservation_token, 9, 1) = '-'
          AND SUBSTR(reservation.reservation_token, 14, 1) = '-'
          AND SUBSTR(reservation.reservation_token, 19, 1) = '-'
          AND SUBSTR(reservation.reservation_token, 24, 1) = '-'
          AND LENGTH(REPLACE(reservation.reservation_token, '-', '')) = 32
          AND REPLACE(reservation.reservation_token, '-', '')
              NOT GLOB '*[^0-9a-f]*'
          AND reservation.request_id = "usage".request_id
          AND reservation.subject_id = "usage".user_id
          AND reservation.reservation_token = JSON_EXTRACT(
              CASE
                WHEN JSON_VALID("usage".request_metadata)
                THEN "usage".request_metadata
                ELSE '{}'
              END,
              '$.plan_usage_reservation_token'
          )
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
