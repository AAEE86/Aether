UPDATE background_task_runs
SET owner_instance = NULL,
    created_by = CASE
        WHEN LOWER(BTRIM(created_by)) IN ('admin', 'scheduler', 'system')
        THEN LOWER(BTRIM(created_by))
        ELSE NULL
    END,
    progress_message = NULL,
    payload_json = NULL,
    result_json = NULL,
    error_message = CASE
        WHEN status = 'failed' THEN 'background_task_failed'
        ELSE NULL
    END
WHERE owner_instance IS NOT NULL
   OR created_by IS NOT NULL
   OR progress_message IS NOT NULL
   OR payload_json IS NOT NULL
   OR result_json IS NOT NULL
   OR error_message IS NOT NULL;

UPDATE background_task_events
SET event_type = CASE
        WHEN event_type IN (
            'cancel_requested', 'failed', 'queued', 'running',
            'skipped', 'succeeded', 'worker_boot'
        ) THEN event_type
        ELSE 'unclassified_event'
    END,
    message = CASE
        WHEN event_type IN (
            'cancel_requested', 'failed', 'queued', 'running',
            'skipped', 'succeeded', 'worker_boot'
        ) THEN event_type
        ELSE 'unclassified_event'
    END,
    payload_json = NULL
WHERE message <> event_type
   OR payload_json IS NOT NULL
   OR event_type NOT IN (
       'cancel_requested', 'failed', 'queued', 'running',
       'skipped', 'succeeded', 'worker_boot'
   );
