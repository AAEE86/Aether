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


def upgrade() -> None:
    """添加 LDAP 认证支持

    1. 创建 authsource 枚举类型
    2. 在 users 表添加 auth_source 字段
    3. 创建 ldap_configs 表
    """
    conn = op.get_bind()

    # 1. 创建 authsource 枚举类型
    conn.execute(text("CREATE TYPE authsource AS ENUM ('local', 'ldap')"))

    # 2. 在 users 表添加 auth_source 字段
    op.add_column('users', sa.Column('auth_source', sa.Enum('local', 'ldap', name='authsource', create_type=False), nullable=False, server_default='local'))

    # 3. 创建 ldap_configs 表
    op.create_table(
        'ldap_configs',
        sa.Column('id', sa.Integer(), autoincrement=True, nullable=False),
        sa.Column('server_url', sa.String(length=255), nullable=False),
        sa.Column('bind_dn', sa.String(length=255), nullable=False),
        sa.Column('bind_password_encrypted', sa.Text(), nullable=False),
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
    """回滚 LDAP 认证支持"""
    # 1. 删除 ldap_configs 表
    op.drop_table('ldap_configs')

    # 2. 删除 users 表的 auth_source 字段
    op.drop_column('users', 'auth_source')

    # 3. 删除 authsource 枚举类型
    conn = op.get_bind()
    conn.execute(text("DROP TYPE authsource"))
