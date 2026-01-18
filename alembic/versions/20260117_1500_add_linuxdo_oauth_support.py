"""add oauth support

Revision ID: d4e5f6g7h8i9
Revises: ddd59cdf0349
Create Date: 2026-01-17 15:00:00.000000+00:00

支持通用 OAuth 认证：
- 添加 oauth_provider_configs 表存储多个 OAuth 提供商配置
- 添加用户表 OAuth 相关字段（oauth_provider_id, oauth_user_id, oauth_username）
- 更新 authsource 枚举添加 'oauth' 值
"""
from alembic import op
import sqlalchemy as sa
from sqlalchemy import text

# revision identifiers, used by Alembic.
revision = 'd4e5f6g7h8i9'
down_revision = 'ddd59cdf0349'
branch_labels = None
depends_on = None


def _type_exists(conn, type_name: str) -> bool:
    """检查 PostgreSQL 类型是否存在"""
    result = conn.execute(
        text("SELECT 1 FROM pg_type WHERE typname = :name"),
        {"name": type_name}
    )
    return result.scalar() is not None


def _column_exists(conn, table_name: str, column_name: str) -> bool:
    """检查列是否存在"""
    result = conn.execute(
        text("""
            SELECT 1 FROM information_schema.columns
            WHERE table_name = :table AND column_name = :column
        """),
        {"table": table_name, "column": column_name}
    )
    return result.scalar() is not None


def _index_exists(conn, index_name: str) -> bool:
    """检查索引是否存在"""
    result = conn.execute(
        text("SELECT 1 FROM pg_indexes WHERE indexname = :name"),
        {"name": index_name}
    )
    return result.scalar() is not None


def _table_exists(conn, table_name: str) -> bool:
    """检查表是否存在"""
    result = conn.execute(
        text("""
            SELECT 1 FROM information_schema.tables
            WHERE table_name = :name AND table_schema = 'public'
        """),
        {"name": table_name}
    )
    return result.scalar() is not None


def _enum_value_exists(conn, enum_name: str, value: str) -> bool:
    """检查枚举值是否存在"""
    result = conn.execute(
        text("""
            SELECT 1 FROM pg_enum e
            JOIN pg_type t ON e.enumtypid = t.oid
            WHERE t.typname = :enum_name AND e.enumlabel = :value
        """),
        {"enum_name": enum_name, "value": value}
    )
    return result.scalar() is not None


def upgrade() -> None:
    """添加通用 OAuth 认证支持

    1. 更新 authsource 枚举添加 'oauth' 值
    2. 在 users 表添加通用 OAuth 字段（oauth_provider_id, oauth_user_id, oauth_username）
    3. 创建 oauth_provider_configs 表（支持多提供商）
    """
    conn = op.get_bind()

    # 1. 更新 authsource 枚举添加 'oauth' 值（幂等）
    if _type_exists(conn, 'authsource'):
        if not _enum_value_exists(conn, 'authsource', 'oauth'):
            conn.execute(text("ALTER TYPE authsource ADD VALUE 'oauth'"))

    # 2. 在 users 表添加通用 OAuth 字段（幂等）
    if not _column_exists(conn, 'users', 'oauth_provider_id'):
        op.add_column('users', sa.Column('oauth_provider_id', sa.String(length=64), nullable=True))

    if not _column_exists(conn, 'users', 'oauth_user_id'):
        op.add_column('users', sa.Column('oauth_user_id', sa.String(length=128), nullable=True))

    if not _column_exists(conn, 'users', 'oauth_username'):
        op.add_column('users', sa.Column('oauth_username', sa.String(length=255), nullable=True))

    # 创建索引（幂等）
    if not _index_exists(conn, 'ix_users_oauth_provider_id'):
        op.create_index('ix_users_oauth_provider_id', 'users', ['oauth_provider_id'])

    if not _index_exists(conn, 'ix_users_oauth_user_id'):
        op.create_index('ix_users_oauth_user_id', 'users', ['oauth_user_id'])

    if not _index_exists(conn, 'ix_users_oauth_username'):
        op.create_index('ix_users_oauth_username', 'users', ['oauth_username'])

    # 3. 创建 oauth_provider_configs 表（幂等）
    if not _table_exists(conn, 'oauth_provider_configs'):
        op.create_table(
            'oauth_provider_configs',
            sa.Column('id', sa.Integer(), autoincrement=True, nullable=False),
            sa.Column('provider_id', sa.String(length=64), nullable=False),
            sa.Column('display_name', sa.String(length=100), nullable=False),
            # OAuth 端点配置
            sa.Column('authorization_url', sa.String(length=500), nullable=False),
            sa.Column('token_url', sa.String(length=500), nullable=False),
            sa.Column('userinfo_url', sa.String(length=500), nullable=False),
            # 用户信息字段映射
            sa.Column('userinfo_mapping', sa.JSON(), nullable=True),
            # OAuth 凭证
            sa.Column('client_id', sa.String(length=255), nullable=False),
            sa.Column('client_secret_encrypted', sa.Text(), nullable=True),
            sa.Column('redirect_uri', sa.String(length=500), nullable=False),
            sa.Column('frontend_callback_url', sa.String(length=500), nullable=True),
            # OAuth 配置
            sa.Column('scope', sa.String(length=500), nullable=True, server_default='user'),
            sa.Column('is_enabled', sa.Boolean(), nullable=False, server_default='false'),
            # 时间戳
            sa.Column('created_at', sa.DateTime(timezone=True), nullable=False, server_default=sa.text('now()')),
            sa.Column('updated_at', sa.DateTime(timezone=True), nullable=False, server_default=sa.text('now()')),
            sa.PrimaryKeyConstraint('id'),
        )
        # 创建唯一索引
        op.create_index('ix_oauth_provider_configs_provider_id', 'oauth_provider_configs', ['provider_id'], unique=True)


def downgrade() -> None:
    """回滚通用 OAuth 认证支持

    警告：回滚前请确保：
    1. 已备份数据库
    2. 没有 OAuth 用户需要保留
    """
    conn = op.get_bind()

    # 检查是否存在 OAuth 用户，防止数据丢失
    if _column_exists(conn, 'users', 'oauth_user_id'):
        result = conn.execute(text("SELECT COUNT(*) FROM users WHERE oauth_user_id IS NOT NULL"))
        oauth_user_count = result.scalar()
        if oauth_user_count and oauth_user_count > 0:
            raise RuntimeError(
                f"无法回滚：存在 {oauth_user_count} 个 OAuth 用户。"
                f"请先删除或转换这些用户。"
            )

    # 1. 删除 oauth_provider_configs 表（幂等）
    if _index_exists(conn, 'ix_oauth_provider_configs_provider_id'):
        op.drop_index('ix_oauth_provider_configs_provider_id', table_name='oauth_provider_configs')

    if _table_exists(conn, 'oauth_provider_configs'):
        op.drop_table('oauth_provider_configs')

    # 2. 删除 users 表的 OAuth 相关字段（幂等）
    if _index_exists(conn, 'ix_users_oauth_username'):
        op.drop_index('ix_users_oauth_username', table_name='users')

    if _index_exists(conn, 'ix_users_oauth_user_id'):
        op.drop_index('ix_users_oauth_user_id', table_name='users')

    if _index_exists(conn, 'ix_users_oauth_provider_id'):
        op.drop_index('ix_users_oauth_provider_id', table_name='users')

    if _column_exists(conn, 'users', 'oauth_username'):
        op.drop_column('users', 'oauth_username')

    if _column_exists(conn, 'users', 'oauth_user_id'):
        op.drop_column('users', 'oauth_user_id')

    if _column_exists(conn, 'users', 'oauth_provider_id'):
        op.drop_column('users', 'oauth_provider_id')

    # 注意：PostgreSQL 不支持从枚举中删除值
    # 'oauth' 值将保留在 authsource 枚举中
