-- Keep nullable/system-owned history rows intact while clearing values tied to
-- deleted users or API keys. Raw payment callback payloads are always removed
-- because they are not required for idempotency and may contain PII.
UPDATE request_candidates
SET username = NULL, api_key_name = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = request_candidates.user_id AND users.is_deleted = 0
  )
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE request_candidates
SET api_key_name = NULL
WHERE api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM api_keys
      JOIN users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
      WHERE api_keys.id = request_candidates.api_key_id
  )
  AND api_key_name IS NOT NULL;

UPDATE video_tasks
SET username = NULL, api_key_name = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = video_tasks.user_id AND users.is_deleted = 0
  )
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE video_tasks
SET api_key_name = NULL
WHERE api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM api_keys
      JOIN users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
      WHERE api_keys.id = video_tasks.api_key_id
  )
  AND api_key_name IS NOT NULL;

UPDATE `usage`
SET username = NULL, api_key_name = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = `usage`.user_id AND users.is_deleted = 0
  )
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE `usage`
SET api_key_name = NULL
WHERE api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM api_keys
      JOIN users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
      WHERE api_keys.id = `usage`.api_key_id
  )
  AND api_key_name IS NOT NULL;

UPDATE stats_user_daily
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_summary
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_summary.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_model
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_model.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_provider
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_provider.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_api_format
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_api_format.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_model_provider
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_model_provider.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_cost_savings.user_id AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_provider
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_cost_savings_provider.user_id
        AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_model
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = stats_user_daily_cost_savings_model.user_id
        AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_model_provider
SET username = NULL
WHERE user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM users
      WHERE users.id = stats_user_daily_cost_savings_model_provider.user_id
        AND users.is_deleted = 0
  )
  AND username IS NOT NULL;

UPDATE stats_daily_api_key
SET api_key_name = NULL
WHERE api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM api_keys
      JOIN users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
      WHERE api_keys.id = stats_daily_api_key.api_key_id
  )
  AND api_key_name IS NOT NULL;

UPDATE user_plan_entitlements AS entitlement
SET status = CASE WHEN status = 'active' THEN 'revoked' ELSE status END,
    expires_at = MIN(expires_at, CAST(strftime('%s', 'now') AS INTEGER)),
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE NOT EXISTS (
    SELECT 1 FROM users
    WHERE users.id = entitlement.user_id AND users.is_deleted = 0
);

UPDATE user_referrals AS referral
SET invite_code_snapshot = 'deleted-user',
    source_json = NULL,
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE NOT EXISTS (
          SELECT 1 FROM users
          WHERE users.id = referral.inviter_user_id AND users.is_deleted = 0
      )
   OR NOT EXISTS (
          SELECT 1 FROM users
          WHERE users.id = referral.invitee_user_id AND users.is_deleted = 0
      );

UPDATE referral_rewards AS reward
SET status = CASE
        WHEN status IN ('pending', 'failed', 'applying') THEN 'voided'
        ELSE status
    END,
    failure_reason = NULL,
    admin_note = NULL,
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE NOT EXISTS (
          SELECT 1 FROM users
          WHERE users.id = reward.inviter_user_id AND users.is_deleted = 0
      )
   OR NOT EXISTS (
          SELECT 1 FROM users
          WHERE users.id = reward.invitee_user_id AND users.is_deleted = 0
      );

UPDATE referral_rewards AS reward
SET failure_reason = NULL,
    admin_note = NULL,
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE reward.admin_operator_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = reward.admin_operator_id AND users.is_deleted = 0
  );

UPDATE wallets AS wallet
SET status = 'disabled',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
   OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
   OR (wallet.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = wallet.user_id AND users.is_deleted = 0
       ))
   OR (wallet.api_key_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM api_keys
           JOIN users AS api_key_owner
             ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
           WHERE api_keys.id = wallet.api_key_id
       ));

UPDATE audit_logs AS history
SET description = 'deleted user event',
    ip_address = NULL,
    user_agent = NULL,
    event_metadata = NULL,
    error_message = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.user_id AND users.is_deleted = 0
       ))
   OR (history.api_key_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM api_keys
           JOIN users AS api_key_owner
             ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
           WHERE api_keys.id = history.api_key_id
       ));

UPDATE wallet_transactions AS history
SET description = NULL
WHERE EXISTS (
    SELECT 1
    FROM wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM users
                WHERE users.id = wallet.user_id AND users.is_deleted = 0
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM api_keys
                JOIN users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM wallets AS wallet WHERE wallet.id = history.wallet_id
   )
   OR (history.operator_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.operator_id AND users.is_deleted = 0
       ));

UPDATE payment_orders AS history
SET gateway_response = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.user_id AND users.is_deleted = 0
       ))
   OR EXISTS (
    SELECT 1
    FROM wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM users
                WHERE users.id = wallet.user_id AND users.is_deleted = 0
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM api_keys
                JOIN users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM wallets AS wallet WHERE wallet.id = history.wallet_id
   );

-- Raw provider payloads are never needed for idempotency and must be purged
-- even when an old callback cannot be linked back to an order.
UPDATE payment_callbacks
SET payload = NULL
WHERE payload IS NOT NULL;

UPDATE payment_callbacks AS history
SET error_message = NULL
WHERE history.error_message IS NOT NULL
  AND (
    NOT EXISTS (
        SELECT 1
        FROM payment_orders AS payment_order
        WHERE payment_order.id = history.payment_order_id
           OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no)
    )
    OR EXISTS (
        SELECT 1
        FROM payment_orders AS payment_order
        LEFT JOIN wallets AS wallet ON wallet.id = payment_order.wallet_id
        WHERE (payment_order.id = history.payment_order_id
               OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no))
          AND ((payment_order.user_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM users
                    WHERE users.id = payment_order.user_id AND users.is_deleted = 0
                ))
            OR wallet.id IS NULL
            OR (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
            OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
            OR (wallet.user_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM users
                    WHERE users.id = wallet.user_id AND users.is_deleted = 0
                ))
            OR (wallet.api_key_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1
                    FROM api_keys
                    JOIN users AS api_key_owner
                      ON api_key_owner.id = api_keys.user_id
                     AND api_key_owner.is_deleted = 0
                    WHERE api_keys.id = wallet.api_key_id
                )))
    )
  );

UPDATE refund_requests AS history
SET reason = NULL,
    payout_reference = NULL,
    payout_proof = NULL,
    failure_reason = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.user_id AND users.is_deleted = 0
       ))
   OR EXISTS (
    SELECT 1
    FROM wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM users
                WHERE users.id = wallet.user_id AND users.is_deleted = 0
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM api_keys
                JOIN users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted = 0
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM wallets AS wallet WHERE wallet.id = history.wallet_id
   )
   OR (history.requested_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.requested_by AND users.is_deleted = 0
       ))
   OR (history.approved_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.approved_by AND users.is_deleted = 0
       ))
   OR (history.processed_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM users
           WHERE users.id = history.processed_by AND users.is_deleted = 0
       ));

UPDATE redeem_code_batches AS history
SET description = NULL
WHERE history.created_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = history.created_by AND users.is_deleted = 0
  );

UPDATE refund_requests
SET requested_by = NULL
WHERE requested_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = refund_requests.requested_by AND users.is_deleted = 0
  );

UPDATE refund_requests
SET approved_by = NULL
WHERE approved_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = refund_requests.approved_by AND users.is_deleted = 0
  );

UPDATE refund_requests
SET processed_by = NULL
WHERE processed_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users
      WHERE users.id = refund_requests.processed_by AND users.is_deleted = 0
  );
