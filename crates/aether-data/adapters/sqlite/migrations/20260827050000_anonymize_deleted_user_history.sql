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

UPDATE usage
SET username = NULL, api_key_name = NULL
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = usage.user_id
)
  AND (username IS NOT NULL OR api_key_name IS NOT NULL);

UPDATE usage
SET api_key_name = NULL
WHERE api_key_name IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM api_keys WHERE api_keys.id = usage.api_key_id
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
    expires_at = MIN(expires_at, CAST(strftime('%s', 'now') AS INTEGER)),
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE NOT EXISTS (
    SELECT 1 FROM users WHERE users.id = entitlement.user_id
);

UPDATE wallets AS wallet
SET status = 'disabled',
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE (wallet.user_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM users WHERE users.id = wallet.user_id
      ))
   OR (wallet.api_key_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM api_keys WHERE api_keys.id = wallet.api_key_id
      ));

UPDATE user_referrals AS referral
SET invite_code_snapshot = 'deleted-user',
    source_json = NULL,
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
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
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
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
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
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

ALTER TABLE user_plan_entitlements RENAME TO _aether_user_plan_entitlements_with_user_fk;
DROP INDEX IF EXISTS idx_user_plan_entitlements_user_active;
DROP INDEX IF EXISTS idx_user_plan_entitlements_order;

CREATE TABLE user_plan_entitlements (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    plan_id TEXT NOT NULL,
    payment_order_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    starts_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    entitlements_snapshot TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(plan_id) REFERENCES billing_plans(id) ON DELETE RESTRICT,
    FOREIGN KEY(payment_order_id) REFERENCES payment_orders(id) ON DELETE RESTRICT
);

INSERT INTO user_plan_entitlements (
    id, user_id, plan_id, payment_order_id, status, starts_at, expires_at,
    entitlements_snapshot, created_at, updated_at
)
SELECT
    id, user_id, plan_id, payment_order_id, status, starts_at, expires_at,
    entitlements_snapshot, created_at, updated_at
FROM _aether_user_plan_entitlements_with_user_fk;

CREATE INDEX idx_user_plan_entitlements_user_active
    ON user_plan_entitlements (user_id, status, expires_at);
CREATE INDEX idx_user_plan_entitlements_order
    ON user_plan_entitlements (payment_order_id);

ALTER TABLE entitlement_usage_ledgers RENAME TO _aether_entitlement_usage_ledgers_with_user_fk;
DROP INDEX IF EXISTS idx_entitlement_usage_user_date;
DROP INDEX IF EXISTS idx_entitlement_usage_entitlement_date;

CREATE TABLE entitlement_usage_ledgers (
    id TEXT PRIMARY KEY,
    user_entitlement_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    amount_usd REAL NOT NULL,
    balance_before REAL NOT NULL,
    balance_after REAL NOT NULL,
    usage_date TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (user_entitlement_id, request_id),
    FOREIGN KEY(user_entitlement_id) REFERENCES user_plan_entitlements(id) ON DELETE CASCADE
);

INSERT INTO entitlement_usage_ledgers (
    id, user_entitlement_id, user_id, request_id, amount_usd,
    balance_before, balance_after, usage_date, created_at
)
SELECT
    id, user_entitlement_id, user_id, request_id, amount_usd,
    balance_before, balance_after, usage_date, created_at
FROM _aether_entitlement_usage_ledgers_with_user_fk;

CREATE INDEX idx_entitlement_usage_user_date
    ON entitlement_usage_ledgers (user_id, usage_date);
CREATE INDEX idx_entitlement_usage_entitlement_date
    ON entitlement_usage_ledgers (user_entitlement_id, usage_date);

DROP TABLE _aether_entitlement_usage_ledgers_with_user_fk;
DROP TABLE _aether_user_plan_entitlements_with_user_fk;

ALTER TABLE user_referrals RENAME TO _aether_user_referrals_with_user_fk;
DROP INDEX IF EXISTS idx_user_referrals_inviter;
DROP INDEX IF EXISTS idx_user_referrals_created;
DROP INDEX IF EXISTS idx_user_referrals_invite_code;

CREATE TABLE user_referrals (
    id TEXT PRIMARY KEY,
    inviter_user_id TEXT NOT NULL,
    invitee_user_id TEXT NOT NULL UNIQUE,
    invite_code_snapshot TEXT NOT NULL,
    source_json TEXT,
    first_paid_order_id TEXT,
    first_paid_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(first_paid_order_id) REFERENCES payment_orders(id) ON DELETE SET NULL
);

INSERT INTO user_referrals (
    id, inviter_user_id, invitee_user_id, invite_code_snapshot, source_json,
    first_paid_order_id, first_paid_at, created_at, updated_at
)
SELECT
    id, inviter_user_id, invitee_user_id, invite_code_snapshot, source_json,
    first_paid_order_id, first_paid_at, created_at, updated_at
FROM _aether_user_referrals_with_user_fk;

CREATE INDEX idx_user_referrals_inviter
    ON user_referrals (inviter_user_id, created_at);
CREATE INDEX idx_user_referrals_created
    ON user_referrals (created_at);
CREATE INDEX idx_user_referrals_invite_code
    ON user_referrals (invite_code_snapshot);

ALTER TABLE referral_rewards RENAME TO _aether_referral_rewards_with_user_fk;
DROP INDEX IF EXISTS idx_referral_rewards_inviter_status;
DROP INDEX IF EXISTS idx_referral_rewards_inviter_created;
DROP INDEX IF EXISTS idx_referral_rewards_created;
DROP INDEX IF EXISTS idx_referral_rewards_source_order;

CREATE TABLE referral_rewards (
    id TEXT PRIMARY KEY,
    referral_id TEXT NOT NULL,
    inviter_user_id TEXT NOT NULL,
    invitee_user_id TEXT NOT NULL,
    reward_type TEXT NOT NULL,
    trigger_point TEXT NOT NULL,
    source_order_id TEXT,
    idempotency_key TEXT NOT NULL UNIQUE,
    amount_usd REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    wallet_transaction_id TEXT,
    reversed_amount_usd REAL NOT NULL DEFAULT 0,
    pending_reversal_amount_usd REAL NOT NULL DEFAULT 0,
    failure_reason TEXT,
    admin_operator_id TEXT,
    admin_note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(referral_id) REFERENCES user_referrals(id) ON DELETE CASCADE,
    FOREIGN KEY(source_order_id) REFERENCES payment_orders(id) ON DELETE SET NULL
);

INSERT INTO referral_rewards (
    id, referral_id, inviter_user_id, invitee_user_id, reward_type,
    trigger_point, source_order_id, idempotency_key, amount_usd, status,
    wallet_transaction_id, reversed_amount_usd, pending_reversal_amount_usd,
    failure_reason, admin_operator_id, admin_note, created_at, updated_at
)
SELECT
    id, referral_id, inviter_user_id, invitee_user_id, reward_type,
    trigger_point, source_order_id, idempotency_key, amount_usd, status,
    wallet_transaction_id, reversed_amount_usd, pending_reversal_amount_usd,
    failure_reason, admin_operator_id, admin_note, created_at, updated_at
FROM _aether_referral_rewards_with_user_fk;

CREATE INDEX idx_referral_rewards_inviter_status
    ON referral_rewards (inviter_user_id, status, created_at);
CREATE INDEX idx_referral_rewards_inviter_created
    ON referral_rewards (inviter_user_id, created_at);
CREATE INDEX idx_referral_rewards_created
    ON referral_rewards (created_at);
CREATE INDEX idx_referral_rewards_source_order
    ON referral_rewards (source_order_id);

DROP TABLE _aether_referral_rewards_with_user_fk;
DROP TABLE _aether_user_referrals_with_user_fk;
