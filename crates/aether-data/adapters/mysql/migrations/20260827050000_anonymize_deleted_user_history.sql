SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'user_plan_entitlements'
          AND CONSTRAINT_NAME = 'user_plan_entitlements_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE user_plan_entitlements DROP FOREIGN KEY user_plan_entitlements_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'entitlement_usage_ledgers'
          AND CONSTRAINT_NAME = 'entitlement_usage_ledgers_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE entitlement_usage_ledgers DROP FOREIGN KEY entitlement_usage_ledgers_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'user_referrals'
          AND CONSTRAINT_NAME = 'user_referrals_inviter_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE user_referrals DROP FOREIGN KEY user_referrals_inviter_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'user_referrals'
          AND CONSTRAINT_NAME = 'user_referrals_invitee_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE user_referrals DROP FOREIGN KEY user_referrals_invitee_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'referral_rewards'
          AND CONSTRAINT_NAME = 'referral_rewards_inviter_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE referral_rewards DROP FOREIGN KEY referral_rewards_inviter_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

SET @aether_drop_fact_user_fk_sql := IF(
    EXISTS (
        SELECT 1 FROM information_schema.TABLE_CONSTRAINTS
        WHERE CONSTRAINT_SCHEMA = DATABASE()
          AND TABLE_NAME = 'referral_rewards'
          AND CONSTRAINT_NAME = 'referral_rewards_invitee_user_id_fkey'
          AND CONSTRAINT_TYPE = 'FOREIGN KEY'
    ),
    'ALTER TABLE referral_rewards DROP FOREIGN KEY referral_rewards_invitee_user_id_fkey',
    'DO 0'
);
PREPARE aether_drop_fact_user_fk_stmt FROM @aether_drop_fact_user_fk_sql;
EXECUTE aether_drop_fact_user_fk_stmt;
DEALLOCATE PREPARE aether_drop_fact_user_fk_stmt;

UPDATE request_candidates
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = request_candidates.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE request_candidates
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM api_keys WHERE api_keys.id = request_candidates.api_key_id
  );

UPDATE video_tasks
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = video_tasks.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE video_tasks
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM api_keys WHERE api_keys.id = video_tasks.api_key_id
  );

UPDATE `usage`
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = `usage`.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE `usage`
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM api_keys WHERE api_keys.id = `usage`.api_key_id
  );

UPDATE stats_user_daily
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_summary
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_summary.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_model
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_model.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_provider
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_provider.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_api_format
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_api_format.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_model_provider
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_model_provider.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_cost_savings.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_provider
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_cost_savings_provider.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_model
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_cost_savings_model.user_id
)
  AND username IS NOT NULL;

UPDATE stats_user_daily_cost_savings_model_provider
SET username = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = stats_user_daily_cost_savings_model_provider.user_id
)
  AND username IS NOT NULL;

UPDATE stats_daily_api_key
SET api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM api_keys WHERE api_keys.id = stats_daily_api_key.api_key_id
)
  AND api_key_name IS NOT NULL;

UPDATE user_plan_entitlements AS entitlement
SET status = CASE WHEN status = 'active' THEN 'revoked' ELSE status END,
    expires_at = LEAST(expires_at, UNIX_TIMESTAMP()),
    updated_at = UNIX_TIMESTAMP()
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = entitlement.user_id
);

UPDATE wallets AS wallet
SET status = 'disabled', updated_at = UNIX_TIMESTAMP()
WHERE (wallet.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = wallet.user_id
      ))
   OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
      ));

UPDATE user_referrals AS referral
SET invite_code_snapshot = 'deleted-user',
    source_json = NULL,
    updated_at = UNIX_TIMESTAMP()
WHERE NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = referral.inviter_user_id
      )
   OR NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = referral.invitee_user_id
      );

UPDATE referral_rewards AS reward
SET status = CASE
        WHEN status IN ('pending', 'failed', 'applying') THEN 'voided'
        ELSE status
    END,
    failure_reason = NULL,
    admin_note = NULL,
    updated_at = UNIX_TIMESTAMP()
WHERE NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = reward.inviter_user_id
      )
   OR NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = reward.invitee_user_id
      );

UPDATE audit_logs AS history
SET description = 'deleted user event',
    ip_address = NULL,
    user_agent = NULL,
    event_metadata = NULL,
    error_message = NULL
WHERE (history.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.user_id
      ))
   OR (history.api_key_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM api_keys WHERE api_keys.id = history.api_key_id
      ));

UPDATE wallet_transactions AS history
SET description = NULL
WHERE EXISTS (
    SELECT 1
    FROM wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND (
          (wallet.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM users WHERE users.id = wallet.user_id
          ))
          OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
          ))
      )
);

UPDATE wallet_transactions AS history
SET description = NULL
WHERE history.operator_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users WHERE users.id = history.operator_id
  );

UPDATE payment_orders AS history
SET gateway_response = NULL
WHERE (history.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.user_id
      ))
   OR EXISTS (
    SELECT 1
    FROM wallets AS wallet
    WHERE wallet.id = history.wallet_id
      AND (
          (wallet.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM users WHERE users.id = wallet.user_id
          ))
          OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
          ))
      )
);

UPDATE payment_callbacks AS history
SET payload = NULL,
    error_message = NULL
WHERE EXISTS (
    SELECT 1
    FROM payment_orders AS payment_order
    LEFT JOIN wallets AS wallet ON wallet.id = payment_order.wallet_id
    WHERE (
            payment_order.id = history.payment_order_id
            OR (history.order_no IS NOT NULL AND payment_order.order_no = history.order_no)
          )
      AND (
          (payment_order.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM users WHERE users.id = payment_order.user_id
          ))
          OR (wallet.user_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM users WHERE users.id = wallet.user_id
          ))
          OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
          ))
      )
);

UPDATE refund_requests AS history
SET reason = NULL,
    payout_reference = NULL,
    payout_proof = NULL,
    failure_reason = NULL
WHERE (history.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.user_id
      ))
   OR EXISTS (
          SELECT 1
          FROM wallets AS wallet
          WHERE wallet.id = history.wallet_id
            AND (
                (wallet.user_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM users WHERE users.id = wallet.user_id
                ))
                OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
                ))
            )
      )
   OR (history.requested_by IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.requested_by
      ))
   OR (history.approved_by IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.approved_by
      ))
   OR (history.processed_by IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = history.processed_by
      ));

UPDATE referral_rewards AS reward
SET failure_reason = NULL,
    admin_note = NULL,
    updated_at = UNIX_TIMESTAMP()
WHERE reward.admin_operator_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users WHERE users.id = reward.admin_operator_id
  );

UPDATE redeem_code_batches AS history
SET description = NULL
WHERE history.created_by IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM users WHERE users.id = history.created_by
  );
