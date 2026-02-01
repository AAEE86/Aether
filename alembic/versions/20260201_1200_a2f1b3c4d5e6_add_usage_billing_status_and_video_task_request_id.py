"""Add usage billing, video_tasks fields, gemini_file_mappings, provider format conversion, and indexes

Revision ID: a2f1b3c4d5e6
Revises: cf40e6a5c5b1
Create Date: 2026-02-01 12:00:00+00:00

Changes:
1. usage 表:
   - 添加 billing_status (pending/settled/void)，用于表示结算状态
   - 添加 finalized_at，用于记录结算完成时间
   - 添加 (provider_name, created_at) 和 (model, created_at) 索引

2. video_tasks 表:
   - 添加 request_id（全局唯一），用于与 Usage/RequestCandidate 建立稳定关联
   - 添加 short_id (Gemini-style short ID)

3. gemini_file_mappings 表:
   - 创建新表用于文件映射
   - 添加 source_hash 字段用于关联相同源文件

4. providers 表:
   - 添加 enable_format_conversion 开关字段

5. request_candidates 表:
   - 添加 created_at 索引
"""

from __future__ import annotations

import secrets
import string
from typing import Sequence, Union

import sqlalchemy as sa
from sqlalchemy import inspect, text

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "a2f1b3c4d5e6"
down_revision: Union[str, None] = "cf40e6a5c5b1"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def table_exists(table_name: str) -> bool:
    bind = op.get_bind()
    inspector = inspect(bind)
    inspector.clear_cache()
    return table_name in inspector.get_table_names()


def column_exists(table_name: str, column_name: str) -> bool:
    bind = op.get_bind()
    inspector = inspect(bind)
    # Clear cached schema info to get fresh data
    inspector.clear_cache()
    columns = [col["name"] for col in inspector.get_columns(table_name)]
    return column_name in columns


def index_exists(table_name: str, index_name: str) -> bool:
    bind = op.get_bind()
    inspector = inspect(bind)
    inspector.clear_cache()
    indexes = inspector.get_indexes(table_name)
    return any(idx.get("name") == index_name for idx in indexes)


def unique_constraint_exists(table_name: str, constraint_name: str) -> bool:
    bind = op.get_bind()
    inspector = inspect(bind)
    inspector.clear_cache()
    constraints = inspector.get_unique_constraints(table_name)
    return any(c.get("name") == constraint_name for c in constraints)


def generate_short_id(length: int = 12) -> str:
    """Generate a Gemini-style short ID (lowercase letters + digits)"""
    alphabet = string.ascii_lowercase + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(length))


def upgrade() -> None:
    bind = op.get_bind()
    dialect = bind.dialect.name

    # =========================================================================
    # 1. usage 表: billing_status + finalized_at + 索引
    # =========================================================================
    if table_exists("usage"):
        if not column_exists("usage", "billing_status"):
            op.add_column(
                "usage",
                sa.Column(
                    "billing_status",
                    sa.String(20),
                    nullable=False,
                    server_default="settled",
                ),
            )

        if not column_exists("usage", "finalized_at"):
            op.add_column(
                "usage",
                sa.Column("finalized_at", sa.DateTime(timezone=True), nullable=True),
            )

        if not index_exists("usage", "idx_usage_billing_status"):
            op.create_index("idx_usage_billing_status", "usage", ["billing_status"])

        # (provider_name, created_at) — provider list / dashboard queries
        if (
            column_exists("usage", "provider_name")
            and column_exists("usage", "created_at")
            and not index_exists("usage", "idx_usage_provider_created")
        ):
            op.create_index("idx_usage_provider_created", "usage", ["provider_name", "created_at"])

        # (model, created_at) — model analytics / recent requests queries
        if (
            column_exists("usage", "model")
            and column_exists("usage", "created_at")
            and not index_exists("usage", "idx_usage_model_created")
        ):
            op.create_index("idx_usage_model_created", "usage", ["model", "created_at"])

    # =========================================================================
    # 2. video_tasks 表: request_id + short_id
    # =========================================================================
    if table_exists("video_tasks"):
        # --- request_id ---
        if not column_exists("video_tasks", "request_id"):
            op.add_column(
                "video_tasks",
                sa.Column("request_id", sa.String(100), nullable=True),
            )

        # 回填 request_id
        if dialect == "postgresql":
            op.execute("""
                UPDATE video_tasks
                SET request_id = COALESCE(request_metadata->>'request_id', id)
                WHERE request_id IS NULL
                """)
        elif dialect == "sqlite":
            op.execute("""
                UPDATE video_tasks
                SET request_id = COALESCE(json_extract(request_metadata, '$.request_id'), id)
                WHERE request_id IS NULL
                """)
        else:
            op.execute("""
                UPDATE video_tasks
                SET request_id = id
                WHERE request_id IS NULL
                """)

        if dialect == "postgresql":
            op.alter_column("video_tasks", "request_id", nullable=False)

            if not index_exists("video_tasks", "idx_video_tasks_request_id"):
                op.create_index("idx_video_tasks_request_id", "video_tasks", ["request_id"])

            if not unique_constraint_exists("video_tasks", "uq_video_tasks_request_id"):
                op.create_unique_constraint(
                    "uq_video_tasks_request_id",
                    "video_tasks",
                    ["request_id"],
                )

        # --- short_id ---
        if not column_exists("video_tasks", "short_id"):
            op.add_column(
                "video_tasks",
                sa.Column("short_id", sa.String(16), nullable=True),
            )

            # Populate existing rows with unique short_ids
            result = bind.execute(text("SELECT id FROM video_tasks WHERE short_id IS NULL"))
            for row in result:
                short_id = generate_short_id()
                bind.execute(
                    text("UPDATE video_tasks SET short_id = :short_id WHERE id = :id"),
                    {"short_id": short_id, "id": row[0]},
                )

            op.alter_column("video_tasks", "short_id", nullable=False)
            op.create_index("ix_video_tasks_short_id", "video_tasks", ["short_id"], unique=True)

    # =========================================================================
    # 3. gemini_file_mappings 表
    # =========================================================================
    if not table_exists("gemini_file_mappings"):
        op.create_table(
            "gemini_file_mappings",
            sa.Column("id", sa.String(36), primary_key=True),
            sa.Column("file_name", sa.String(255), nullable=False, unique=True),
            sa.Column(
                "key_id",
                sa.String(36),
                sa.ForeignKey("provider_api_keys.id", ondelete="CASCADE"),
                nullable=False,
            ),
            sa.Column(
                "user_id",
                sa.String(36),
                sa.ForeignKey("users.id", ondelete="CASCADE"),
                nullable=True,
            ),
            sa.Column("display_name", sa.String(255), nullable=True),
            sa.Column("mime_type", sa.String(100), nullable=True),
            sa.Column("source_hash", sa.String(64), nullable=True),
            sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
            sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        )

        op.create_index("ix_gemini_file_mappings_id", "gemini_file_mappings", ["id"])
        op.create_index(
            "ix_gemini_file_mappings_file_name", "gemini_file_mappings", ["file_name"], unique=True
        )
        op.create_index("ix_gemini_file_mappings_key_id", "gemini_file_mappings", ["key_id"])
        op.create_index("ix_gemini_file_mappings_user_id", "gemini_file_mappings", ["user_id"])
        op.create_index("idx_gemini_file_mappings_expires", "gemini_file_mappings", ["expires_at"])
        op.create_index(
            "idx_gemini_file_mappings_source_hash", "gemini_file_mappings", ["source_hash"]
        )
    else:
        # 表已存在，只添加 source_hash
        if not column_exists("gemini_file_mappings", "source_hash"):
            op.add_column(
                "gemini_file_mappings",
                sa.Column("source_hash", sa.String(64), nullable=True),
            )
            op.create_index(
                "idx_gemini_file_mappings_source_hash",
                "gemini_file_mappings",
                ["source_hash"],
            )

    # =========================================================================
    # 4. providers 表: enable_format_conversion
    # =========================================================================
    if table_exists("providers") and not column_exists("providers", "enable_format_conversion"):
        op.add_column(
            "providers",
            sa.Column(
                "enable_format_conversion",
                sa.Boolean(),
                nullable=False,
                server_default=sa.text("false"),
            ),
        )

    # =========================================================================
    # 5. request_candidates 表: created_at 索引
    # =========================================================================
    if table_exists("request_candidates"):
        if not index_exists("request_candidates", "idx_request_candidates_created_at"):
            op.create_index(
                "idx_request_candidates_created_at",
                "request_candidates",
                ["created_at"],
                unique=False,
            )


def downgrade() -> None:
    bind = op.get_bind()
    dialect = bind.dialect.name

    # =========================================================================
    # 5. request_candidates 表回滚
    # =========================================================================
    if table_exists("request_candidates"):
        if index_exists("request_candidates", "idx_request_candidates_created_at"):
            op.drop_index("idx_request_candidates_created_at", table_name="request_candidates")

    # =========================================================================
    # 4. providers 表回滚
    # =========================================================================
    if table_exists("providers") and column_exists("providers", "enable_format_conversion"):
        op.drop_column("providers", "enable_format_conversion")

    # =========================================================================
    # 3. gemini_file_mappings 表回滚
    # =========================================================================
    if table_exists("gemini_file_mappings"):
        op.drop_index("idx_gemini_file_mappings_source_hash", table_name="gemini_file_mappings")
        op.drop_index("idx_gemini_file_mappings_expires", table_name="gemini_file_mappings")
        op.drop_index("ix_gemini_file_mappings_user_id", table_name="gemini_file_mappings")
        op.drop_index("ix_gemini_file_mappings_key_id", table_name="gemini_file_mappings")
        op.drop_index("ix_gemini_file_mappings_file_name", table_name="gemini_file_mappings")
        op.drop_index("ix_gemini_file_mappings_id", table_name="gemini_file_mappings")
        op.drop_table("gemini_file_mappings")

    # =========================================================================
    # 2. video_tasks 表回滚
    # =========================================================================
    if table_exists("video_tasks"):
        # short_id
        if column_exists("video_tasks", "short_id"):
            if index_exists("video_tasks", "ix_video_tasks_short_id"):
                op.drop_index("ix_video_tasks_short_id", table_name="video_tasks")
            op.drop_column("video_tasks", "short_id")

        # request_id
        if column_exists("video_tasks", "request_id"):
            if dialect == "postgresql":
                if unique_constraint_exists("video_tasks", "uq_video_tasks_request_id"):
                    op.drop_constraint("uq_video_tasks_request_id", "video_tasks", type_="unique")
                if index_exists("video_tasks", "idx_video_tasks_request_id"):
                    op.drop_index("idx_video_tasks_request_id", table_name="video_tasks")
            op.drop_column("video_tasks", "request_id")

    # =========================================================================
    # 1. usage 表回滚
    # =========================================================================
    if table_exists("usage"):
        if index_exists("usage", "idx_usage_model_created"):
            op.drop_index("idx_usage_model_created", table_name="usage")
        if index_exists("usage", "idx_usage_provider_created"):
            op.drop_index("idx_usage_provider_created", table_name="usage")
        if index_exists("usage", "idx_usage_billing_status"):
            op.drop_index("idx_usage_billing_status", table_name="usage")
        if column_exists("usage", "finalized_at"):
            op.drop_column("usage", "finalized_at")
        if column_exists("usage", "billing_status"):
            op.drop_column("usage", "billing_status")
