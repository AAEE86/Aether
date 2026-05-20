ALTER TABLE api_keys
ADD COLUMN allowed_ips TEXT NULL AFTER allowed_models;
