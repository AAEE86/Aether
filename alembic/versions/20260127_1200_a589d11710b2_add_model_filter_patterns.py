"""add model_include_patterns and model_exclude_patterns to provider_api_keys

Revision ID: a589d11710b2
Revises: 4b4c7b0df1a2
Create Date: 2026-01-27 12:00:00.000000+00:00

为 provider_api_keys 表添加模型过滤规则字段:
1. model_include_patterns: 包含规则列表（支持 * 和 ? 通配符）
2. model_exclude_patterns: 排除规则列表（支持 * 和 ? 通配符）

注意: downgrade 操作会永久删除过滤规则配置
"""
from alembic import op
import sqlalchemy as sa
from sqlalchemy import inspect


# revision identifiers, used by Alembic.
revision = 'a589d11710b2'
down_revision = '4b4c7b0df1a2'
branch_labels = None
depends_on = None


def _column_exists(table_name: str, column_name: str) -> bool:
    """Check if a column exists in the table"""
    bind = op.get_bind()
    inspector = inspect(bind)
    columns = [col["name"] for col in inspector.get_columns(table_name)]
    return column_name in columns


def upgrade() -> None:
    """添加模型过滤规则字段"""
    if not _column_exists("provider_api_keys", "model_include_patterns"):
        op.add_column(
            "provider_api_keys",
            sa.Column("model_include_patterns", sa.JSON(), nullable=True),
        )

    if not _column_exists("provider_api_keys", "model_exclude_patterns"):
        op.add_column(
            "provider_api_keys",
            sa.Column("model_exclude_patterns", sa.JSON(), nullable=True),
        )


def downgrade() -> None:
    """移除模型过滤规则字段"""
    if _column_exists("provider_api_keys", "model_exclude_patterns"):
        op.drop_column("provider_api_keys", "model_exclude_patterns")

    if _column_exists("provider_api_keys", "model_include_patterns"):
        op.drop_column("provider_api_keys", "model_include_patterns")
