"""add global_priority_by_format and remove deprecated fields

Revision ID: ddd59cdf0349
Revises: 6d579000e511
Create Date: 2026-01-16 12:00:00.000000+00:00

变更:
1. provider_api_keys 表: 添加 global_priority_by_format 字段（按 API 格式的全局优先级）
2. 迁移现有 global_priority 数据到新字段
3. 删除已废弃的 global_priority 字段
4. 删除已废弃的 rate_multiplier 字段（已被 rate_multipliers 替代）
5. 删除已废弃的 providers.timeout 字段（由环境变量控制）
6. 删除已废弃的 provider_endpoints.timeout 字段（由环境变量控制）
"""
from alembic import op
import sqlalchemy as sa
from sqlalchemy.dialects.postgresql import JSON

# revision identifiers, used by Alembic.
revision = 'ddd59cdf0349'
down_revision = '6d579000e511'
branch_labels = None
depends_on = None


def _column_exists(connection, table: str, column: str) -> bool:
    """检查列是否存在"""
    result = connection.execute(
        sa.text("""
            SELECT 1 FROM information_schema.columns
            WHERE table_name = :table AND column_name = :column
        """),
        {"table": table, "column": column}
    )
    return result.fetchone() is not None


def upgrade():
    connection = op.get_bind()

    # 1. 添加 global_priority_by_format 字段
    if not _column_exists(connection, 'provider_api_keys', 'global_priority_by_format'):
        op.add_column(
            'provider_api_keys',
            sa.Column('global_priority_by_format', JSON, nullable=True)
        )

    # 2. 迁移现有 global_priority 数据到新字段
    # 对于有 global_priority 的 Key，将其值应用到所有支持的 api_formats
    if _column_exists(connection, 'provider_api_keys', 'global_priority'):
        # 将 JSON 数组转换为 text[] 后使用 unnest
        connection.execute(sa.text("""
            UPDATE provider_api_keys
            SET global_priority_by_format = (
                SELECT jsonb_object_agg(format, global_priority)
                FROM jsonb_array_elements_text(api_formats::jsonb) AS format
            )
            WHERE global_priority IS NOT NULL
              AND api_formats IS NOT NULL
              AND jsonb_array_length(api_formats::jsonb) > 0
              AND global_priority_by_format IS NULL
        """))

        # 3. 删除 global_priority 字段
        op.drop_column('provider_api_keys', 'global_priority')

    # 4. 删除 rate_multiplier 字段（已被 rate_multipliers 替代）
    if _column_exists(connection, 'provider_api_keys', 'rate_multiplier'):
        op.drop_column('provider_api_keys', 'rate_multiplier')

    # 5. 删除 providers.timeout 字段（由环境变量控制）
    if _column_exists(connection, 'providers', 'timeout'):
        op.drop_column('providers', 'timeout')

    # 6. 删除 provider_endpoints.timeout 字段（由环境变量控制）
    if _column_exists(connection, 'provider_endpoints', 'timeout'):
        op.drop_column('provider_endpoints', 'timeout')


def downgrade():
    connection = op.get_bind()

    # 1. 恢复 rate_multiplier 字段
    if not _column_exists(connection, 'provider_api_keys', 'rate_multiplier'):
        op.add_column(
            'provider_api_keys',
            sa.Column('rate_multiplier', sa.Float, nullable=False, server_default='1.0')
        )

    # 2. 恢复 global_priority 字段并迁移数据
    if not _column_exists(connection, 'provider_api_keys', 'global_priority'):
        op.add_column(
            'provider_api_keys',
            sa.Column('global_priority', sa.Integer, nullable=True)
        )

        # 从 global_priority_by_format 迁移数据（取第一个格式的优先级值）
        if _column_exists(connection, 'provider_api_keys', 'global_priority_by_format'):
            connection.execute(sa.text("""
                UPDATE provider_api_keys
                SET global_priority = (
                    SELECT (value::text)::integer
                    FROM jsonb_each(global_priority_by_format::jsonb)
                    LIMIT 1
                )
                WHERE global_priority_by_format IS NOT NULL
                  AND jsonb_typeof(global_priority_by_format::jsonb) = 'object'
                  AND global_priority IS NULL
            """))

    # 3. 删除 global_priority_by_format 字段
    if _column_exists(connection, 'provider_api_keys', 'global_priority_by_format'):
        op.drop_column('provider_api_keys', 'global_priority_by_format')

    # 4. 恢复 providers.timeout 字段
    if not _column_exists(connection, 'providers', 'timeout'):
        op.add_column(
            'providers',
            sa.Column('timeout', sa.Integer, nullable=True, server_default='300')
        )

    # 5. 恢复 provider_endpoints.timeout 字段
    if not _column_exists(connection, 'provider_endpoints', 'timeout'):
        op.add_column(
            'provider_endpoints',
            sa.Column('timeout', sa.Integer, nullable=True, server_default='300')
        )
