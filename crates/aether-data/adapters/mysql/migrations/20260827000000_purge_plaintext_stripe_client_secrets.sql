-- Stripe PaymentIntent client secrets are payment capabilities. New writes
-- store only encrypted values; legacy plaintext cannot be migrated safely
-- without the application encryption key, so remove it fail-closed.
UPDATE payment_orders
SET status = CASE
        WHEN LOWER(TRIM(payment_method)) = 'stripe' AND status = 'pending'
        THEN 'expired'
        ELSE status
    END,
    gateway_response = JSON_REMOVE(gateway_response, '$.client_secret')
WHERE gateway_response IS NOT NULL
  AND JSON_VALID(gateway_response) = 1
  AND JSON_CONTAINS_PATH(gateway_response, 'one', '$.client_secret') = 1;
