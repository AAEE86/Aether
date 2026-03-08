"""cost fields: Float -> Numeric(20,8) + provider_api_keys composite index

Revision ID: 2053ab8ed764
Revises: 13a4c8f6d9e0
Create Date: 2026-03-08 15:30:00.000000+00:00

"""

import sqlalchemy as sa

from alembic import op

# revision identifiers, used by Alembic.
revision = "2053ab8ed764"
down_revision = "13a4c8f6d9e0"
branch_labels = None
depends_on = None


# (table_name, column_name, nullable, server_default)
_COST_COLUMNS: list[tuple[str, str, bool, str | None]] = [
    # api_keys
    ("api_keys", "total_cost_usd", True, "0.0"),
    # usage
    ("usage", "input_cost_usd", True, "0.0"),
    ("usage", "output_cost_usd", True, "0.0"),
    ("usage", "cache_cost_usd", True, "0.0"),
    ("usage", "cache_creation_cost_usd", True, "0.0"),
    ("usage", "cache_read_cost_usd", True, "0.0"),
    ("usage", "request_cost_usd", True, "0.0"),
    ("usage", "total_cost_usd", True, "0.0"),
    ("usage", "actual_input_cost_usd", True, "0.0"),
    ("usage", "actual_output_cost_usd", True, "0.0"),
    ("usage", "actual_cache_creation_cost_usd", True, "0.0"),
    ("usage", "actual_cache_read_cost_usd", True, "0.0"),
    ("usage", "actual_request_cost_usd", True, "0.0"),
    ("usage", "actual_total_cost_usd", True, "0.0"),
    ("usage", "rate_multiplier", True, "1.0"),
    ("usage", "input_price_per_1m", True, None),
    ("usage", "output_price_per_1m", True, None),
    ("usage", "cache_creation_price_per_1m", True, None),
    ("usage", "cache_read_price_per_1m", True, None),
    ("usage", "price_per_request", True, None),
    # providers
    ("providers", "monthly_quota_usd", True, None),
    ("providers", "monthly_used_usd", True, "0.0"),
    # global_models
    ("global_models", "default_price_per_request", True, None),
    # models
    ("models", "price_per_request", True, None),
    # stats_hourly
    ("stats_hourly", "total_cost", False, "0.0"),
    ("stats_hourly", "actual_total_cost", False, "0.0"),
    # stats_hourly_user
    ("stats_hourly_user", "total_cost", False, "0.0"),
    # stats_hourly_model
    ("stats_hourly_model", "total_cost", False, "0.0"),
    # stats_hourly_provider
    ("stats_hourly_provider", "total_cost", False, "0.0"),
    # stats_daily
    ("stats_daily", "total_cost", False, "0.0"),
    ("stats_daily", "actual_total_cost", False, "0.0"),
    ("stats_daily", "input_cost", False, "0.0"),
    ("stats_daily", "output_cost", False, "0.0"),
    ("stats_daily", "cache_creation_cost", False, "0.0"),
    ("stats_daily", "cache_read_cost", False, "0.0"),
    # stats_daily_model
    ("stats_daily_model", "total_cost", False, "0.0"),
    # stats_daily_provider
    ("stats_daily_provider", "total_cost", False, "0.0"),
    # stats_daily_api_key
    ("stats_daily_api_key", "total_cost", False, "0.0"),
    # stats_summary
    ("stats_summary", "all_time_cost", False, "0.0"),
    ("stats_summary", "all_time_actual_cost", False, "0.0"),
    # stats_user_daily
    ("stats_user_daily", "total_cost", False, "0.0"),
]

# rate_multiplier uses a smaller precision
_RATE_MULTIPLIER_TYPE = sa.Numeric(10, 6)
_COST_TYPE = sa.Numeric(20, 8)


def _column_exists(table_name: str, column_name: str) -> bool:
    bind = op.get_bind()
    result = bind.execute(
        sa.text(
            "SELECT 1 FROM information_schema.columns "
            "WHERE table_name = :table AND column_name = :col"
        ),
        {"table": table_name, "col": column_name},
    )
    return result.scalar() is not None


def _index_exists(index_name: str) -> bool:
    bind = op.get_bind()
    result = bind.execute(
        sa.text("SELECT 1 FROM pg_indexes WHERE indexname = :name"),
        {"name": index_name},
    )
    return result.scalar() is not None


def _is_numeric_type(table_name: str, column_name: str) -> bool:
    """Check if a column is already numeric type (not float/double precision)."""
    bind = op.get_bind()
    result = bind.execute(
        sa.text(
            "SELECT data_type FROM information_schema.columns "
            "WHERE table_name = :table AND column_name = :col"
        ),
        {"table": table_name, "col": column_name},
    )
    data_type = result.scalar()
    return data_type == "numeric"


def _type_spec(col: str) -> str:
    """Return the SQL type literal for a given column name."""
    return "NUMERIC(10,6)" if col == "rate_multiplier" else "NUMERIC(20,8)"


def _batch_alter_type(
    columns: list[tuple[str, str, bool, str | None]],
    cast_suffix: str,
    type_fn=None,
) -> None:
    """Group columns by table and issue ONE ALTER TABLE per table.

    This avoids rewriting the same table N times (once per column).
    """
    from collections import defaultdict

    by_table: dict[str, list[tuple[str, str, bool, str | None]]] = defaultdict(list)
    for table, col, nullable, default in columns:
        if not _column_exists(table, col):
            continue
        by_table[table].append((table, col, nullable, default))

    bind = op.get_bind()
    for table, cols in by_table.items():
        # Build a single ALTER TABLE with multiple ALTER COLUMN clauses
        parts: list[str] = []
        for _t, col, _nullable, _default in cols:
            target_type = type_fn(col) if type_fn else _type_spec(col)
            parts.append(
                f"ALTER COLUMN {col} TYPE {target_type} USING {col}::{cast_suffix}"
            )
        if not parts:
            continue
        sql = f"ALTER TABLE {table} " + ", ".join(parts)
        bind.execute(sa.text(sql))


def upgrade() -> None:
    # -- 1. cost fields: Float -> Numeric  (batched per table)
    # Filter out columns that are already numeric
    cols_to_convert = [
        (t, c, n, d)
        for t, c, n, d in _COST_COLUMNS
        if _column_exists(t, c) and not _is_numeric_type(t, c)
    ]
    _batch_alter_type(cols_to_convert, cast_suffix="numeric", type_fn=_type_spec)

    # -- 2. provider_api_keys composite index
    if not _index_exists("idx_provider_api_keys_provider_active"):
        op.create_index(
            "idx_provider_api_keys_provider_active",
            "provider_api_keys",
            ["provider_id", "is_active"],
        )


def downgrade() -> None:
    # -- 2. drop composite index
    if _index_exists("idx_provider_api_keys_provider_active"):
        op.drop_index(
            "idx_provider_api_keys_provider_active",
            table_name="provider_api_keys",
        )

    # -- 1. Numeric -> Float  (batched per table)
    cols_to_revert = [
        (t, c, n, d)
        for t, c, n, d in _COST_COLUMNS
        if _column_exists(t, c) and _is_numeric_type(t, c)
    ]
    _batch_alter_type(
        cols_to_revert,
        cast_suffix="double precision",
        type_fn=lambda _col: "DOUBLE PRECISION",
    )
