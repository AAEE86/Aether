from dataclasses import asdict, dataclass
from typing import Any, TypeVar
from collections.abc import Sequence

from sqlalchemy.orm import Query

T = TypeVar("T")


@dataclass
class PaginationMeta:
    total: int
    limit: int
    offset: int
    count: int

    def to_dict(self) -> dict:
        return asdict(self)


def paginate_query(query: Query, limit: int, offset: int) -> tuple[int, list[T]]:
    """
    对 SQLAlchemy 查询应用 limit/offset，并返回总数与结果列表。
    """
    total = query.order_by(None).count()
    records = query.offset(offset).limit(limit).all()
    return total, records


def paginate_sequence(
    items: Sequence[T], limit: int, offset: int
) -> tuple[list[T], PaginationMeta]:
    """
    对内存序列应用分页，返回切片和元数据。
    """
    total = len(items)
    sliced = list(items[offset : offset + limit])
    meta = PaginationMeta(total=total, limit=limit, offset=offset, count=len(sliced))
    return sliced, meta


def build_pagination_payload(items: list[dict], meta: PaginationMeta, **extra: Any) -> dict:
    """
    构建标准分页响应 payload。
    """
    payload: dict = {"items": items, "meta": meta.to_dict()}
    payload.update(extra)
    return payload
