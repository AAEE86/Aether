-- Identity claims required by the application are stored in dedicated columns.
-- Remove legacy provider-controlled userinfo JSON, which may contain unrelated
-- PII or credentials and is not needed for authentication or account binding.
UPDATE public.user_oauth_links
SET extra_data = NULL
WHERE extra_data IS NOT NULL;
