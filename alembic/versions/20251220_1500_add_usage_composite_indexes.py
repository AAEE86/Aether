"""add usage table composite indexes for query optimization

Revision ID: b2c3d4e5f6g7
Revises: a1b2c3d4e5f6
Create Date: 2025-12-20 15:00:00.000000+00:00

"""
from alembic import op
from sqlalchemy import text

# revision identifiers, used by Alembic.
revision = 'b2c3d4e5f6g7'
down_revision = 'a1b2c3d4e5f6'
branch_labels = None
depends_on = None


def upgrade() -> None:
    """为 usage 表添加复合索引以优化常见查询

    使用 CONCURRENTLY 创建索引以避免锁表，
    但需要在 AUTOCOMMIT 模式下执行（不能在事务内）
    """
    conn = op.get_bind()
    engine = conn.engine

    # 使用新连接并设置 AUTOCOMMIT 模式以支持 CREATE INDEX CONCURRENTLY
    with engine.connect().execution_options(isolation_level="AUTOCOMMIT") as autocommit_conn:
        # 使用 IF NOT EXISTS 避免重复创建，无需单独检查索引是否存在

        # 1. user_id + created_at 复合索引 (用户用量查询)
        autocommit_conn.execute(text(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_usage_user_created "
            "ON usage (user_id, created_at)"
        ))

        # 2. api_key_id + created_at 复合索引 (API Key 用量查询)
        autocommit_conn.execute(text(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_usage_apikey_created "
            "ON usage (api_key_id, created_at)"
        ))

        # 3. provider + model + created_at 复合索引 (模型统计查询)
        autocommit_conn.execute(text(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_usage_provider_model_created "
            "ON usage (provider, model, created_at)"
        ))


def downgrade() -> None:
    """删除复合索引"""
    conn = op.get_bind()

    # 使用 IF EXISTS 避免索引不存在时报错
    conn.execute(text(
        "DROP INDEX IF EXISTS idx_usage_provider_model_created"
    ))
    conn.execute(text(
        "DROP INDEX IF EXISTS idx_usage_apikey_created"
    ))
    conn.execute(text(
        "DROP INDEX IF EXISTS idx_usage_user_created"
    ))
