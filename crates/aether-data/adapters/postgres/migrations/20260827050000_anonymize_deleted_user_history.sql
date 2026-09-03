ALTER TABLE public.request_candidates
    DROP CONSTRAINT IF EXISTS request_candidates_user_id_fkey;
ALTER TABLE public.video_tasks
    DROP CONSTRAINT IF EXISTS video_tasks_user_id_fkey;
ALTER TABLE public.usage
    DROP CONSTRAINT IF EXISTS usage_user_id_fkey;
ALTER TABLE public.stats_user_daily
    DROP CONSTRAINT IF EXISTS stats_user_daily_user_id_fkey;
ALTER TABLE public.stats_user_summary
    DROP CONSTRAINT IF EXISTS stats_user_summary_user_id_fkey;
ALTER TABLE public.stats_user_daily_model
    DROP CONSTRAINT IF EXISTS stats_user_daily_model_user_id_fkey;
ALTER TABLE public.stats_user_daily_provider
    DROP CONSTRAINT IF EXISTS stats_user_daily_provider_user_id_fkey;
ALTER TABLE public.stats_user_daily_api_format
    DROP CONSTRAINT IF EXISTS stats_user_daily_api_format_user_id_fkey;
ALTER TABLE public.stats_user_daily_model_provider
    DROP CONSTRAINT IF EXISTS stats_user_daily_model_provider_user_id_fkey;
ALTER TABLE public.stats_user_daily_cost_savings
    DROP CONSTRAINT IF EXISTS stats_user_daily_cost_savings_user_id_fkey;
ALTER TABLE public.stats_user_daily_cost_savings_provider
    DROP CONSTRAINT IF EXISTS stats_user_daily_cost_savings_provider_user_id_fkey;
ALTER TABLE public.stats_user_daily_cost_savings_model
    DROP CONSTRAINT IF EXISTS stats_user_daily_cost_savings_model_user_id_fkey;
ALTER TABLE public.stats_user_daily_cost_savings_model_provider
    DROP CONSTRAINT IF EXISTS stats_user_daily_cost_savings_model_provider_user_id_fkey;
ALTER TABLE public.stats_hourly_user_model
    DROP CONSTRAINT IF EXISTS stats_hourly_user_model_user_id_fkey;
ALTER TABLE public.user_model_usage_counts
    DROP CONSTRAINT IF EXISTS user_model_usage_counts_user_id_fkey;

ALTER TABLE public.audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_user_id_fkey;
ALTER TABLE public.announcements
    DROP CONSTRAINT IF EXISTS announcements_author_id_fkey;
ALTER TABLE public.payment_orders
    DROP CONSTRAINT IF EXISTS payment_orders_user_id_fkey;
ALTER TABLE public.proxy_nodes
    DROP CONSTRAINT IF EXISTS proxy_nodes_registered_by_fkey;
ALTER TABLE public.refund_requests
    DROP CONSTRAINT IF EXISTS refund_requests_user_id_fkey,
    DROP CONSTRAINT IF EXISTS refund_requests_requested_by_fkey,
    DROP CONSTRAINT IF EXISTS refund_requests_approved_by_fkey,
    DROP CONSTRAINT IF EXISTS refund_requests_processed_by_fkey;
ALTER TABLE public.wallet_transactions
    DROP CONSTRAINT IF EXISTS wallet_transactions_operator_id_fkey;
ALTER TABLE public.wallets
    DROP CONSTRAINT IF EXISTS wallets_user_id_fkey,
    DROP CONSTRAINT IF EXISTS wallets_api_key_id_fkey;
ALTER TABLE public.redeem_code_batches
    DROP CONSTRAINT IF EXISTS redeem_code_batches_created_by_fkey;
ALTER TABLE public.redeem_codes
    DROP CONSTRAINT IF EXISTS redeem_codes_redeemed_by_user_id_fkey,
    DROP CONSTRAINT IF EXISTS redeem_codes_disabled_by_fkey;

ALTER TABLE public.user_plan_entitlements
    DROP CONSTRAINT IF EXISTS user_plan_entitlements_user_id_fkey;
ALTER TABLE public.entitlement_usage_ledgers
    DROP CONSTRAINT IF EXISTS entitlement_usage_ledgers_user_id_fkey;
ALTER TABLE public.user_referrals
    DROP CONSTRAINT IF EXISTS user_referrals_inviter_user_id_fkey,
    DROP CONSTRAINT IF EXISTS user_referrals_invitee_user_id_fkey;
ALTER TABLE public.referral_rewards
    DROP CONSTRAINT IF EXISTS referral_rewards_inviter_user_id_fkey,
    DROP CONSTRAINT IF EXISTS referral_rewards_invitee_user_id_fkey;

ALTER TABLE public.request_candidates
    DROP CONSTRAINT IF EXISTS request_candidates_api_key_id_fkey;
ALTER TABLE public.video_tasks
    DROP CONSTRAINT IF EXISTS video_tasks_api_key_id_fkey;
ALTER TABLE public.usage
    DROP CONSTRAINT IF EXISTS usage_api_key_id_fkey;
ALTER TABLE public.stats_daily_api_key
    DROP CONSTRAINT IF EXISTS stats_daily_api_key_api_key_id_fkey;

UPDATE public.request_candidates AS history
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE public.request_candidates AS history
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.api_keys WHERE api_keys.id = history.api_key_id
  );

UPDATE public.video_tasks AS history
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE public.video_tasks AS history
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.api_keys WHERE api_keys.id = history.api_key_id
  );

UPDATE public.usage AS history
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE public.usage AS history
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.api_keys WHERE api_keys.id = history.api_key_id
  );

UPDATE public.stats_user_daily AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_summary AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_model AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_provider AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_api_format AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_model_provider AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_provider AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_model AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_user_daily_cost_savings_model_provider AS history
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = history.user_id
)
  AND username IS NOT NULL;

UPDATE public.stats_daily_api_key AS history
SET api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM public.api_keys WHERE api_keys.id = history.api_key_id
)
  AND api_key_name IS NOT NULL;

UPDATE public.user_plan_entitlements AS entitlement
SET status = CASE WHEN status = 'active' THEN 'revoked' ELSE status END,
    expires_at = LEAST(expires_at, NOW()),
    updated_at = NOW()
WHERE NOT EXISTS (
    SELECT 1 FROM public.users WHERE users.id = entitlement.user_id
);

UPDATE public.wallets AS wallet
SET status = 'disabled', updated_at = NOW()
WHERE (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
   OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = wallet.user_id
      ))
   OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM public.api_keys WHERE api_keys.id = wallet.api_key_id
      ));

UPDATE public.user_referrals AS referral
SET invite_code_snapshot = 'deleted-user',
    source_json = NULL,
    updated_at = NOW()
WHERE NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = referral.inviter_user_id
      )
   OR NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = referral.invitee_user_id
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
          SELECT 1 FROM public.users WHERE users.id = reward.inviter_user_id
      )
   OR NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = reward.invitee_user_id
      );

UPDATE public.audit_logs AS history
SET description = 'deleted user event',
    ip_address = NULL,
    user_agent = NULL,
    event_metadata = NULL,
    error_message = NULL
WHERE history.user_id IS NULL
   OR NOT EXISTS (
      SELECT 1 FROM public.users WHERE users.id = history.user_id
  )
   OR (history.api_key_id IS NOT NULL AND NOT EXISTS (
      SELECT 1 FROM public.api_keys WHERE api_keys.id = history.api_key_id
  ));

UPDATE public.wallet_transactions AS history
SET description = NULL
WHERE EXISTS (
    SELECT 1
    FROM public.wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND (
          (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
          OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM public.users WHERE users.id = wallet.user_id
          ))
          OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM public.api_keys WHERE api_keys.id = wallet.api_key_id
          ))
      )
);

UPDATE public.wallet_transactions AS history
SET description = NULL
WHERE history.operator_id IS NULL
   OR NOT EXISTS (
      SELECT 1 FROM public.users WHERE users.id = history.operator_id
  );

UPDATE public.payment_orders AS history
SET gateway_response = NULL
WHERE history.user_id IS NULL
   OR NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = history.user_id
      )
   OR EXISTS (
          SELECT 1
          FROM public.wallets AS wallet
          WHERE wallet.id = history.wallet_id
            AND (
                (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
                OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM public.users WHERE users.id = wallet.user_id
                ))
                OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM public.api_keys WHERE api_keys.id = wallet.api_key_id
                ))
            )
      );

UPDATE public.payment_callbacks AS history
SET payload = NULL,
    error_message = NULL
WHERE EXISTS (
    SELECT 1
    FROM public.payment_orders AS payment_order
    LEFT JOIN public.wallets AS wallet ON wallet.id = payment_order.wallet_id
    WHERE (
            payment_order.id = history.payment_order_id
            OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no)
          )
      AND (
          payment_order.user_id IS NULL
          OR NOT EXISTS (
              SELECT 1 FROM public.users WHERE users.id = payment_order.user_id
          )
          OR (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
          OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM public.users WHERE users.id = wallet.user_id
          ))
          OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM public.api_keys WHERE api_keys.id = wallet.api_key_id
          ))
      )
);

UPDATE public.refund_requests AS history
SET reason = NULL,
    payout_reference = NULL,
    payout_proof = NULL,
    failure_reason = NULL
WHERE history.user_id IS NULL
   OR NOT EXISTS (
          SELECT 1 FROM public.users WHERE users.id = history.user_id
      )
   OR EXISTS (
          SELECT 1
          FROM public.wallets AS wallet
          WHERE wallet.id = history.wallet_id
            AND (
                (wallet.user_id IS NULL AND wallet.api_key_id IS NULL)
                OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM public.users WHERE users.id = wallet.user_id
                ))
                OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM public.api_keys WHERE api_keys.id = wallet.api_key_id
                ))
            )
      );

UPDATE public.referral_rewards AS reward
SET failure_reason = NULL,
    admin_note = NULL,
    updated_at = NOW()
WHERE reward.admin_operator_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM public.users WHERE users.id = reward.admin_operator_id
  );

UPDATE public.redeem_code_batches AS history
SET description = NULL
WHERE history.created_by IS NULL
   OR NOT EXISTS (
      SELECT 1 FROM public.users WHERE users.id = history.created_by
  );
