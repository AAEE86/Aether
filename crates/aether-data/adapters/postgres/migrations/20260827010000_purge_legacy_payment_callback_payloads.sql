-- Callback idempotency uses payload_hash and does not require the raw body.
-- Older versions stored provider-controlled payloads that may contain payment
-- capabilities or customer PII, so remove those legacy copies fail-closed.
UPDATE payment_callbacks
SET payload = NULL
WHERE payload IS NOT NULL;
