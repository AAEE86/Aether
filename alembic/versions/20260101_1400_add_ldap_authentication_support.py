"""add ldap authentication support

Revision ID: c3d4e5f6g7h8
Revises: b2c3d4e5f6g7
Create Date: 2026-01-01 14:00:00.000000+00:00

"""
from alembic import op
import sqlalchemy as sa
from sqlalchemy import text

# revision identifiers, used by Alembic.
revision = 'c3d4e5f6g7h8'
down_revision = 'b2c3d4e5f6g7'
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


def upgrade() -> None:
    """添加 LDAP 认证支持

    1. 创建 authsource 枚举类型
    2. 在 users 表添加 auth_source 字段和 LDAP 标识字段
    3. 创建 ldap_configs 表
    """
    conn = op.get_bind()

    # 1. 创建 authsource 枚举类型（幂等）
    if not _type_exists(conn, 'authsource'):
        conn.execute(text("CREATE TYPE authsource AS ENUM ('local', 'ldap')"))

    # 2. 在 users 表添加字段（幂等）
    if not _column_exists(conn, 'users', 'auth_source'):
        op.add_column('users', sa.Column(
            'auth_source',
            sa.Enum('local', 'ldap', name='authsource', create_type=False),
            nullable=False,
            server_default='local'
        ))

    if not _column_exists(conn, 'users', 'ldap_dn'):
        op.add_column('users', sa.Column('ldap_dn', sa.String(length=512), nullable=True))

    if not _column_exists(conn, 'users', 'ldap_username'):
        op.add_column('users', sa.Column('ldap_username', sa.String(length=255), nullable=True))

    # 创建索引（幂等）
    if not _index_exists(conn, 'ix_users_ldap_dn'):
        op.create_index('ix_users_ldap_dn', 'users', ['ldap_dn'])

    if not _index_exists(conn, 'ix_users_ldap_username'):
        op.create_index('ix_users_ldap_username', 'users', ['ldap_username'])

    # 3. 创建 ldap_configs 表（幂等）
    if not _table_exists(conn, 'ldap_configs'):
        op.create_table(
            'ldap_configs',
            sa.Column('id', sa.Integer(), autoincrement=True, nullable=False),
            sa.Column('server_url', sa.String(length=255), nullable=False),
            sa.Column('bind_dn', sa.String(length=255), nullable=False),
            sa.Column('bind_password_encrypted', sa.Text(), nullable=True),
            sa.Column('base_dn', sa.String(length=255), nullable=False),
            sa.Column('user_search_filter', sa.String(length=500), nullable=False, server_default='(uid={username})'),
            sa.Column('username_attr', sa.String(length=50), nullable=False, server_default='uid'),
            sa.Column('email_attr', sa.String(length=50), nullable=False, server_default='mail'),
            sa.Column('display_name_attr', sa.String(length=50), nullable=False, server_default='cn'),
            sa.Column('is_enabled', sa.Boolean(), nullable=False, server_default='false'),
            sa.Column('is_exclusive', sa.Boolean(), nullable=False, server_default='false'),
            sa.Column('use_starttls', sa.Boolean(), nullable=False, server_default='false'),
            sa.Column('connect_timeout', sa.Integer(), nullable=False, server_default='10'),
            sa.Column('created_at', sa.DateTime(timezone=True), nullable=False, server_default=sa.text('now()')),
            sa.Column('updated_at', sa.DateTime(timezone=True), nullable=False, server_default=sa.text('now()')),
            sa.PrimaryKeyConstraint('id')
        )


def downgrade() -> None:
    """回滚 LDAP 认证支持

    警告：回滚前请确保：
    1. 已备份数据库
    2. 没有 LDAP 用户需要保留
    """
    conn = op.get_bind()

    # 检查是否存在 LDAP 用户，防止数据丢失
    if _column_exists(conn, 'users', 'auth_source'):
        result = conn.execute(text("SELECT COUNT(*) FROM users WHERE auth_source = 'ldap'"))
        ldap_user_count = result.scalar()
        if ldap_user_count and ldap_user_count > 0:
            raise RuntimeError(
                f"无法回滚：存在 {ldap_user_count} 个 LDAP 用户。"
                f"请先删除或转换这些用户，或使用 --force 参数强制回滚（将丢失数据）。"
            )

    # 1. 删除 ldap_configs 表（幂等）
    if _table_exists(conn, 'ldap_configs'):
        op.drop_table('ldap_configs')

    # 2. 删除 users 表的 LDAP 相关字段（幂等）
    if _index_exists(conn, 'ix_users_ldap_username'):
        op.drop_index('ix_users_ldap_username', table_name='users')

    if _index_exists(conn, 'ix_users_ldap_dn'):
        op.drop_index('ix_users_ldap_dn', table_name='users')

    if _column_exists(conn, 'users', 'ldap_username'):
        op.drop_column('users', 'ldap_username')

    if _column_exists(conn, 'users', 'ldap_dn'):
        op.drop_column('users', 'ldap_dn')

    if _column_exists(conn, 'users', 'auth_source'):
        op.drop_column('users', 'auth_source')

    # 3. 删除 authsource 枚举类型（幂等）
    # 注意：不使用 CASCADE，因为此时所有依赖应该已被删除
    if _type_exists(conn, 'authsource'):
        conn.execute(text("DROP TYPE authsource"))
