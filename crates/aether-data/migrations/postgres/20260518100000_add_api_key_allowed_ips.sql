ALTER TABLE api_keys
ADD COLUMN IF NOT EXISTS allowed_ips jsonb NULL;

ALTER TABLE api_keys
ALTER COLUMN allowed_ips TYPE jsonb USING allowed_ips::jsonb;
