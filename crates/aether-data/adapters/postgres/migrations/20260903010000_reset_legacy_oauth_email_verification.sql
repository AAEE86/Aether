-- Older gateway releases marked every OAuth email as verified, even when the
-- provider did not make a trustworthy verification assertion. There is no
-- persisted provenance that can distinguish those rows, so fail closed. A
-- later OAuth login may re-verify the same normalized email from a trusted
-- provider assertion.
UPDATE public.users
SET email_verified = FALSE,
    updated_at = CURRENT_TIMESTAMP
WHERE LOWER(TRIM(auth_source::text)) = 'oauth'
  AND email_verified = TRUE;
