-- Stripe PaymentIntent client secrets are payment capabilities. New writes
-- store only encrypted values; legacy plaintext cannot be migrated safely
-- without the application encryption key, so remove it fail-closed.
UPDATE payment_orders
SET status = CASE
        WHEN LOWER(BTRIM(payment_method)) = 'stripe' AND status = 'pending'
        THEN 'expired'
        ELSE status
    END,
    gateway_response = gateway_response - 'client_secret'
WHERE gateway_response ? 'client_secret';
