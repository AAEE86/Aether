"""usage token semantics v2

Revision ID: c3d4e5f6a7b8
Revises: c9d8e7f6a5b4
Create Date: 2026-03-24 14:00:00.000000+00:00
"""

from __future__ import annotations

from collections.abc import Sequence

import sqlalchemy as sa
from sqlalchemy import inspect

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "c3d4e5f6a7b8"
down_revision: str | None = "c9d8e7f6a5b4"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None

BACKFILL_BATCH_SIZE = 2000

# 最大批次数，防止因数据异常导致死循环（2000 * 500000 = 10亿行上限）
_MAX_BATCHES = 500000

# 使用子查询中间层展开 input_output_total_tokens 的计算，
# 确保 total_tokens 引用的是本次 SET 后的新值而非旧值。
_UPGRADE_BACKFILL_SQL = sa.text(
    """
    UPDATE usage
    SET
        input_output_total_tokens = src.new_iot,
        input_context_tokens      = src.new_ict,
        total_tokens              = src.new_total,
        cache_creation_cost_usd_5m        = src.new_cc5m,
        cache_creation_cost_usd_1h        = src.new_cc1h,
        actual_cache_creation_cost_usd_5m = src.new_acc5m,
        actual_cache_creation_cost_usd_1h = src.new_acc1h,
        actual_cache_cost_usd             = src.new_accu,
        cache_creation_price_per_1m_5m    = src.new_cp5m,
        cache_creation_price_per_1m_1h    = src.new_cp1h,
        cache_cost_usd                    = src.new_ccu
    FROM (
        SELECT
            id,
            COALESCE(input_output_total_tokens,
                     COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
                AS new_iot,
            COALESCE(input_tokens, 0) + COALESCE(cache_read_input_tokens, 0)
                AS new_ict,
            /* total_tokens 引用本行计算出的 new_iot，避免依赖 SET 顺序 */
            COALESCE(input_output_total_tokens,
                     COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
                + COALESCE(cache_creation_input_tokens, 0)
                + COALESCE(cache_read_input_tokens, 0)
                AS new_total,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                THEN COALESCE(cache_creation_cost_usd, 0)
                WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                     AND COALESCE(cache_creation_input_tokens, 0) > 0
                THEN COALESCE(cache_creation_cost_usd, 0)
                     * (COALESCE(cache_creation_input_tokens_5m, 0) * 1.0
                        / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                ELSE 0
            END AS new_cc5m,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                THEN COALESCE(cache_creation_cost_usd, 0)
                WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                     AND COALESCE(cache_creation_input_tokens, 0) > 0
                THEN COALESCE(cache_creation_cost_usd, 0)
                     * (COALESCE(cache_creation_input_tokens_1h, 0) * 1.0
                        / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                ELSE 0
            END AS new_cc1h,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                THEN COALESCE(actual_cache_creation_cost_usd, 0)
                WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                     AND COALESCE(cache_creation_input_tokens, 0) > 0
                THEN COALESCE(actual_cache_creation_cost_usd, 0)
                     * (COALESCE(cache_creation_input_tokens_5m, 0) * 1.0
                        / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                ELSE 0
            END AS new_acc5m,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                THEN COALESCE(actual_cache_creation_cost_usd, 0)
                WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                     AND COALESCE(cache_creation_input_tokens, 0) > 0
                THEN COALESCE(actual_cache_creation_cost_usd, 0)
                     * (COALESCE(cache_creation_input_tokens_1h, 0) * 1.0
                        / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                ELSE 0
            END AS new_acc1h,
            COALESCE(actual_cache_creation_cost_usd, 0)
                + COALESCE(actual_cache_read_cost_usd, 0) AS new_accu,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                THEN cache_creation_price_per_1m
                ELSE NULL
            END AS new_cp5m,
            CASE
                WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                     AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                THEN cache_creation_price_per_1m
                ELSE NULL
            END AS new_cp1h,
            COALESCE(cache_creation_cost_usd, 0)
                + COALESCE(cache_read_cost_usd, 0) AS new_ccu
        FROM usage
        WHERE id IN (
            SELECT id FROM usage
            WHERE
                input_output_total_tokens IS DISTINCT FROM
                    COALESCE(input_output_total_tokens,
                             COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
                OR input_context_tokens IS DISTINCT FROM
                    COALESCE(input_tokens, 0) + COALESCE(cache_read_input_tokens, 0)
                OR total_tokens IS DISTINCT FROM (
                    COALESCE(input_output_total_tokens,
                             COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
                    + COALESCE(cache_creation_input_tokens, 0)
                    + COALESCE(cache_read_input_tokens, 0)
                )
                OR cache_creation_cost_usd_5m IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                        THEN COALESCE(cache_creation_cost_usd, 0)
                        WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                             AND COALESCE(cache_creation_input_tokens, 0) > 0
                        THEN COALESCE(cache_creation_cost_usd, 0)
                             * (COALESCE(cache_creation_input_tokens_5m, 0) * 1.0
                                / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                        ELSE 0
                    END
                )
                OR cache_creation_cost_usd_1h IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                        THEN COALESCE(cache_creation_cost_usd, 0)
                        WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                             AND COALESCE(cache_creation_input_tokens, 0) > 0
                        THEN COALESCE(cache_creation_cost_usd, 0)
                             * (COALESCE(cache_creation_input_tokens_1h, 0) * 1.0
                                / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                        ELSE 0
                    END
                )
                OR actual_cache_creation_cost_usd_5m IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                        THEN COALESCE(actual_cache_creation_cost_usd, 0)
                        WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                             AND COALESCE(cache_creation_input_tokens, 0) > 0
                        THEN COALESCE(actual_cache_creation_cost_usd, 0)
                             * (COALESCE(cache_creation_input_tokens_5m, 0) * 1.0
                                / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                        ELSE 0
                    END
                )
                OR actual_cache_creation_cost_usd_1h IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                        THEN COALESCE(actual_cache_creation_cost_usd, 0)
                        WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                             AND COALESCE(cache_creation_input_tokens, 0) > 0
                        THEN COALESCE(actual_cache_creation_cost_usd, 0)
                             * (COALESCE(cache_creation_input_tokens_1h, 0) * 1.0
                                / GREATEST(COALESCE(cache_creation_input_tokens, 0), 1))
                        ELSE 0
                    END
                )
                OR actual_cache_cost_usd IS DISTINCT FROM (
                    COALESCE(actual_cache_creation_cost_usd, 0)
                    + COALESCE(actual_cache_read_cost_usd, 0)
                )
                OR cache_creation_price_per_1m_5m IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_5m, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_1h, 0) = 0
                        THEN cache_creation_price_per_1m
                        ELSE NULL
                    END
                )
                OR cache_creation_price_per_1m_1h IS DISTINCT FROM (
                    CASE
                        WHEN COALESCE(cache_creation_input_tokens_1h, 0) > 0
                             AND COALESCE(cache_creation_input_tokens_5m, 0) = 0
                        THEN cache_creation_price_per_1m
                        ELSE NULL
                    END
                )
                OR cache_cost_usd IS DISTINCT FROM (
                    COALESCE(cache_creation_cost_usd, 0) + COALESCE(cache_read_cost_usd, 0)
                )
            ORDER BY id
            LIMIT :batch_size
        )
    ) AS src
    WHERE usage.id = src.id
    """
)

_DOWNGRADE_BACKFILL_SQL = sa.text(
    """
    UPDATE usage
    SET total_tokens = COALESCE(input_output_total_tokens, COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
    WHERE id IN (
        SELECT id
        FROM usage
        WHERE total_tokens IS DISTINCT FROM
            COALESCE(input_output_total_tokens, COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0))
        ORDER BY id
        LIMIT :batch_size
    )
    """
)


def column_exists(table_name: str, column_name: str) -> bool:
    bind = op.get_bind()
    inspector = inspect(bind)
    columns = [c["name"] for c in inspector.get_columns(table_name)]
    return column_name in columns


def run_backfill_in_batches(sql: sa.TextClause, batch_size: int = BACKFILL_BATCH_SIZE) -> None:
    context = op.get_context()
    for _ in range(_MAX_BATCHES):
        # Commit the preceding schema transaction before each batch so PostgreSQL
        # does not keep ALTER TABLE locks for the entire data backfill.
        with context.autocommit_block():
            rowcount = op.get_bind().execute(sql, {"batch_size": batch_size}).rowcount
        if rowcount == 0:
            break
    else:
        raise RuntimeError(
            f"Backfill did not converge after {_MAX_BATCHES} batches "
            f"(batch_size={batch_size}). Possible infinite loop due to data anomaly."
        )


def upgrade() -> None:
    if column_exists("usage", "total_tokens") and not column_exists("usage", "input_output_total_tokens"):
        with op.batch_alter_table("usage") as batch_op:
            batch_op.alter_column(
                "total_tokens",
                new_column_name="input_output_total_tokens",
                existing_type=sa.Integer(),
                existing_nullable=True,
            )

    with op.batch_alter_table("usage") as batch_op:
        if not column_exists("usage", "input_context_tokens"):
            batch_op.add_column(sa.Column("input_context_tokens", sa.Integer(), nullable=False, server_default="0"))
        if not column_exists("usage", "total_tokens"):
            batch_op.add_column(sa.Column("total_tokens", sa.Integer(), nullable=False, server_default="0"))
        if not column_exists("usage", "cache_creation_cost_usd_5m"):
            batch_op.add_column(sa.Column("cache_creation_cost_usd_5m", sa.Numeric(20, 8), nullable=False, server_default="0"))
        if not column_exists("usage", "cache_creation_cost_usd_1h"):
            batch_op.add_column(sa.Column("cache_creation_cost_usd_1h", sa.Numeric(20, 8), nullable=False, server_default="0"))
        if not column_exists("usage", "actual_cache_creation_cost_usd_5m"):
            batch_op.add_column(sa.Column("actual_cache_creation_cost_usd_5m", sa.Numeric(20, 8), nullable=False, server_default="0"))
        if not column_exists("usage", "actual_cache_creation_cost_usd_1h"):
            batch_op.add_column(sa.Column("actual_cache_creation_cost_usd_1h", sa.Numeric(20, 8), nullable=False, server_default="0"))
        if not column_exists("usage", "actual_cache_cost_usd"):
            batch_op.add_column(sa.Column("actual_cache_cost_usd", sa.Numeric(20, 8), nullable=False, server_default="0"))
        if not column_exists("usage", "cache_creation_price_per_1m_5m"):
            batch_op.add_column(sa.Column("cache_creation_price_per_1m_5m", sa.Numeric(20, 8), nullable=True))
        if not column_exists("usage", "cache_creation_price_per_1m_1h"):
            batch_op.add_column(sa.Column("cache_creation_price_per_1m_1h", sa.Numeric(20, 8), nullable=True))

    run_backfill_in_batches(_UPGRADE_BACKFILL_SQL)


def downgrade() -> None:
    run_backfill_in_batches(_DOWNGRADE_BACKFILL_SQL)

    with op.batch_alter_table("usage") as batch_op:
        if column_exists("usage", "cache_creation_price_per_1m_1h"):
            batch_op.drop_column("cache_creation_price_per_1m_1h")
        if column_exists("usage", "cache_creation_price_per_1m_5m"):
            batch_op.drop_column("cache_creation_price_per_1m_5m")
        if column_exists("usage", "actual_cache_cost_usd"):
            batch_op.drop_column("actual_cache_cost_usd")
        if column_exists("usage", "actual_cache_creation_cost_usd_1h"):
            batch_op.drop_column("actual_cache_creation_cost_usd_1h")
        if column_exists("usage", "actual_cache_creation_cost_usd_5m"):
            batch_op.drop_column("actual_cache_creation_cost_usd_5m")
        if column_exists("usage", "cache_creation_cost_usd_1h"):
            batch_op.drop_column("cache_creation_cost_usd_1h")
        if column_exists("usage", "cache_creation_cost_usd_5m"):
            batch_op.drop_column("cache_creation_cost_usd_5m")
        if column_exists("usage", "input_context_tokens"):
            batch_op.drop_column("input_context_tokens")
        if column_exists("usage", "total_tokens"):
            batch_op.drop_column("total_tokens")
        if column_exists("usage", "input_output_total_tokens"):
            batch_op.alter_column(
                "input_output_total_tokens",
                new_column_name="total_tokens",
                existing_type=sa.Integer(),
                existing_nullable=True,
            )
