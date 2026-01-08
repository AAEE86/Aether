"""add shared api keys

Revision ID: 1234abcd5678
Revises: 02a45b66b7c4
Create Date: 2026-01-08 16:00:00.000000

"""
from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision = '1234abcd5678'
down_revision = '02a45b66b7c4'
branch_labels = None
depends_on = None


def upgrade() -> None:
    # 1. Add column is_shared (Boolean, nullable=False, default=False)
    op.add_column('provider_api_keys', sa.Column('is_shared', sa.Boolean(), server_default='0', nullable=False))

    # 2. Add column provider_id (String(36), ForeignKey, nullable=True)
    op.add_column('provider_api_keys', sa.Column('provider_id', sa.String(length=36), nullable=True))
    op.create_foreign_key('fk_provider_api_keys_provider_id', 'provider_api_keys', 'providers', ['provider_id'], ['id'], ondelete='CASCADE')

    # 3. Make endpoint_id nullable
    # Note: We keep the existing foreign key constraint, just making column nullable
    op.alter_column('provider_api_keys', 'endpoint_id',
               existing_type=sa.String(length=36),
               nullable=True)


def downgrade() -> None:
    # 1. Revert endpoint_id to non-nullable
    # Warning: This will fail if there are records with NULL endpoint_id
    op.alter_column('provider_api_keys', 'endpoint_id',
               existing_type=sa.String(length=36),
               nullable=False)

    # 2. Drop provider_id
    op.drop_constraint('fk_provider_api_keys_provider_id', 'provider_api_keys', type_='foreignkey')
    op.drop_column('provider_api_keys', 'provider_id')

    # 3. Drop is_shared
    op.drop_column('provider_api_keys', 'is_shared')
