-- Preserve nullable/system-owned history rows while clearing data tied to
-- deleted users or API keys. Raw payment callback payloads are always removed
-- because they are not required for idempotency and may contain PII.
UPDATE public.request_candidates AS history
SET username = NULL, api_key_name = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND (history.username IS NOT NULL OR history.api_key_name IS NOT NULL);

UPDATE public.request_candidates AS history
SET api_key_name = NULL
WHERE history.api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM public.api_keys
      JOIN public.users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
      WHERE api_keys.id = history.api_key_id
  )
  AND history.api_key_name IS NOT NULL;

UPDATE public.video_tasks AS history
SET username = NULL, api_key_name = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND (history.username IS NOT NULL OR history.api_key_name IS NOT NULL);

UPDATE public.video_tasks AS history
SET api_key_name = NULL
WHERE history.api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM public.api_keys
      JOIN public.users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
      WHERE api_keys.id = history.api_key_id
  )
  AND history.api_key_name IS NOT NULL;

UPDATE public.usage AS history
SET username = NULL, api_key_name = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND (history.username IS NOT NULL OR history.api_key_name IS NOT NULL);

UPDATE public.usage AS history
SET api_key_name = NULL
WHERE history.api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM public.api_keys
      JOIN public.users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
      WHERE api_keys.id = history.api_key_id
  )
  AND history.api_key_name IS NOT NULL;

UPDATE public.stats_user_daily AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_summary AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_model AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_provider AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_api_format AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_model_provider AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_provider AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_model AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_model_provider AS history
SET username = NULL
WHERE history.user_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.user_id AND users.is_deleted IS FALSE
  )
  AND history.username IS NOT NULL;

UPDATE public.stats_daily_api_key AS history
SET api_key_name = NULL
WHERE history.api_key_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM public.api_keys
      JOIN public.users AS api_key_owner
        ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
      WHERE api_keys.id = history.api_key_id
  )
  AND history.api_key_name IS NOT NULL;

UPDATE public.user_plan_entitlements AS entitlement
SET status = CASE WHEN status = 'active' THEN 'revoked' ELSE status END,
    expires_at = LEAST(expires_at, NOW()),
    updated_at = NOW()
WHERE NOT EXISTS (
    SELECT 1 FROM public.users
    WHERE users.id = entitlement.user_id AND users.is_deleted IS FALSE
);

UPDATE public.user_referrals AS referral
SET invite_code_snapshot = 'deleted-user',
    source_json = NULL,
    updated_at = NOW()
WHERE NOT EXISTS (
          SELECT 1 FROM public.users
          WHERE users.id = referral.inviter_user_id AND users.is_deleted IS FALSE
      )
   OR NOT EXISTS (
          SELECT 1 FROM public.users
          WHERE users.id = referral.invitee_user_id AND users.is_deleted IS FALSE
      );

UPDATE public.referral_rewards AS reward
SET status = CASE
        WHEN status IN ('pending', 'failed', 'applying') THEN 'voided'
        ELSE status
    END,
    failure_reason = NULL,
    admin_note = NULL,
    updated_at = NOW()
WHERE NOT EXISTS (
          SELECT 1 FROM public.users
          WHERE users.id = reward.inviter_user_id AND users.is_deleted IS FALSE
      )
   OR NOT EXISTS (
          SELECT 1 FROM public.users
          WHERE users.id = reward.invitee_user_id AND users.is_deleted IS FALSE
      );

UPDATE public.referral_rewards AS reward
SET failure_reason = NULL,
    admin_note = NULL,
    updated_at = NOW()
WHERE reward.admin_operator_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = reward.admin_operator_id AND users.is_deleted IS FALSE
  );

UPDATE public.wallets AS wallet
SET status = 'disabled', updated_at = NOW()
WHERE (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
   OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
   OR (wallet.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = wallet.user_id AND users.is_deleted IS FALSE
       ))
   OR (wallet.api_key_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM public.api_keys
           JOIN public.users AS api_key_owner
             ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
           WHERE api_keys.id = wallet.api_key_id
       ));

UPDATE public.audit_logs AS history
SET description = 'deleted user event',
    ip_address = NULL,
    user_agent = NULL,
    event_metadata = NULL,
    error_message = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.user_id AND users.is_deleted IS FALSE
       ))
   OR (history.api_key_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM public.api_keys
           JOIN public.users AS api_key_owner
             ON api_key_owner.id = api_keys.user_id AND api_key_owner.is_deleted IS FALSE
           WHERE api_keys.id = history.api_key_id
       ));

UPDATE public.wallet_transactions AS history
SET description = NULL
WHERE EXISTS (
    SELECT 1
    FROM public.wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM public.users
                WHERE users.id = wallet.user_id AND users.is_deleted IS FALSE
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM public.api_keys
                JOIN public.users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id
                 AND api_key_owner.is_deleted IS FALSE
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM public.wallets AS wallet WHERE wallet.id = history.wallet_id
   )
   OR (history.operator_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.operator_id AND users.is_deleted IS FALSE
       ));

UPDATE public.payment_orders AS history
SET gateway_response = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.user_id AND users.is_deleted IS FALSE
       ))
   OR EXISTS (
    SELECT 1
    FROM public.wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM public.users
                WHERE users.id = wallet.user_id AND users.is_deleted IS FALSE
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM public.api_keys
                JOIN public.users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id
                 AND api_key_owner.is_deleted IS FALSE
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM public.wallets AS wallet WHERE wallet.id = history.wallet_id
   );

-- Raw provider payloads are never needed for idempotency and must be purged
-- even when an old callback cannot be linked back to an order.
UPDATE public.payment_callbacks
SET payload = NULL
WHERE payload IS NOT NULL;

UPDATE public.payment_callbacks AS history
SET error_message = NULL
WHERE history.error_message IS NOT NULL
  AND (
    NOT EXISTS (
        SELECT 1
        FROM public.payment_orders AS payment_order
        WHERE payment_order.id = history.payment_order_id
           OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no)
    )
    OR EXISTS (
        SELECT 1
        FROM public.payment_orders AS payment_order
        LEFT JOIN public.wallets AS wallet ON wallet.id = payment_order.wallet_id
        WHERE (payment_order.id = history.payment_order_id
               OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no))
          AND ((payment_order.user_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM public.users
                    WHERE users.id = payment_order.user_id AND users.is_deleted IS FALSE
                ))
            OR wallet.id IS NULL
            OR (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
            OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
            OR (wallet.user_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM public.users
                    WHERE users.id = wallet.user_id AND users.is_deleted IS FALSE
                ))
            OR (wallet.api_key_id IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1
                    FROM public.api_keys
                    JOIN public.users AS api_key_owner
                      ON api_key_owner.id = api_keys.user_id
                     AND api_key_owner.is_deleted IS FALSE
                    WHERE api_keys.id = wallet.api_key_id
                )))
    )
  );

UPDATE public.refund_requests AS history
SET reason = NULL, payout_reference = NULL, payout_proof = NULL, failure_reason = NULL
WHERE (history.user_id IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.user_id AND users.is_deleted IS FALSE
       ))
   OR EXISTS (
    SELECT 1
    FROM public.wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND ((wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
        OR (wallet.user_id IS NOT NULL AND wallet.api_key_id IS NOT NULL)
        OR (wallet.user_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM public.users
                WHERE users.id = wallet.user_id AND users.is_deleted IS FALSE
            ))
        OR (wallet.api_key_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                FROM public.api_keys
                JOIN public.users AS api_key_owner
                  ON api_key_owner.id = api_keys.user_id
                 AND api_key_owner.is_deleted IS FALSE
                WHERE api_keys.id = wallet.api_key_id
            )))
)
   OR NOT EXISTS (
       SELECT 1 FROM public.wallets AS wallet WHERE wallet.id = history.wallet_id
   )
   OR (history.requested_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.requested_by AND users.is_deleted IS FALSE
       ))
   OR (history.approved_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.approved_by AND users.is_deleted IS FALSE
       ))
   OR (history.processed_by IS NOT NULL
       AND NOT EXISTS (
           SELECT 1 FROM public.users
           WHERE users.id = history.processed_by AND users.is_deleted IS FALSE
       ));

UPDATE public.refund_requests AS history
SET requested_by = NULL
WHERE history.requested_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.requested_by AND users.is_deleted IS FALSE
  );

UPDATE public.refund_requests AS history
SET approved_by = NULL
WHERE history.approved_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.approved_by AND users.is_deleted IS FALSE
  );

UPDATE public.refund_requests AS history
SET processed_by = NULL
WHERE history.processed_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.processed_by AND users.is_deleted IS FALSE
  );

UPDATE public.redeem_code_batches AS history
SET description = NULL
WHERE history.created_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users
      WHERE users.id = history.created_by AND users.is_deleted IS FALSE
  );
