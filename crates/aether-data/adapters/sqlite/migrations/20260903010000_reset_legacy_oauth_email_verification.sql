-- Older gateway releases marked every OAuth email as verified without
-- retaining verification provenance. Reset those claims conservatively; a
-- later trusted OAuth assertion for the same normalized email can upgrade it.
UPDATE users
SET email_verified = 0,
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE LOWER(TRIM(auth_source)) = 'oauth'
  AND email_verified = 1;
