UPDATE request_candidates
SET
    username = NULL,
    api_key_name = NULL,
    extra_data = NULL,
    required_capabilities = NULL,
    error_message = NULL,
    error_type = NULL,
    skip_reason = NULL
WHERE username IS NOT NULL
   OR api_key_name IS NOT NULL
   OR extra_data IS NOT NULL
   OR required_capabilities IS NOT NULL
   OR error_message IS NOT NULL
   OR error_type IS NOT NULL
   OR skip_reason IS NOT NULL;
