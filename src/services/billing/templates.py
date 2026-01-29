"""
预定义计费模板

提供常见厂商的计费配置模板，避免重复配置：
- CLAUDE_STANDARD: Claude/Anthropic 标准计费
- OPENAI_STANDARD: OpenAI 标准计费
- DOUBAO_STANDARD: 豆包计费（含缓存存储）
- GEMINI_STANDARD: Gemini 标准计费
- PER_REQUEST: 按次计费
"""


from src.services.billing.models import BillingDimension, BillingUnit


class BillingTemplates:
    """预定义的计费模板"""

    # =========================================================================
    # Claude/Anthropic 标准计费
    # - 输入 token
    # - 输出 token
    # - 缓存创建（创建时收费，约 1.25x 输入价格）
    # - 缓存读取（约 0.1x 输入价格）
    # - 按次计费（可选，配置 price_per_request 时生效）
    # =========================================================================
    CLAUDE_STANDARD: list[BillingDimension] = [
        BillingDimension(
            name="input",
            usage_field="input_tokens",
            price_field="input_price_per_1m",
        ),
        BillingDimension(
            name="output",
            usage_field="output_tokens",
            price_field="output_price_per_1m",
        ),
        BillingDimension(
            name="cache_creation",
            usage_field="cache_creation_tokens",
            price_field="cache_creation_price_per_1m",
        ),
        BillingDimension(
            name="cache_read",
            usage_field="cache_read_tokens",
            price_field="cache_read_price_per_1m",
        ),
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]

    # =========================================================================
    # OpenAI 标准计费
    # - 输入 token
    # - 输出 token
    # - 缓存读取（部分模型支持，无缓存创建费用）
    # - 按次计费（可选，配置 price_per_request 时生效）
    # =========================================================================
    OPENAI_STANDARD: list[BillingDimension] = [
        BillingDimension(
            name="input",
            usage_field="input_tokens",
            price_field="input_price_per_1m",
        ),
        BillingDimension(
            name="output",
            usage_field="output_tokens",
            price_field="output_price_per_1m",
        ),
        BillingDimension(
            name="cache_read",
            usage_field="cache_read_tokens",
            price_field="cache_read_price_per_1m",
        ),
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]

    # =========================================================================
    # 豆包计费
    # - 推理输入 (input_tokens)
    # - 推理输出 (output_tokens)
    # - 缓存命中 (cache_read_tokens) - 类似 Claude 的缓存读取
    # - 缓存存储 (cache_storage_token_hours) - 按 token 数 * 存储时长计费
    # - 按次计费（可选，配置 price_per_request 时生效）
    #
    # 注意：豆包的缓存创建是免费的，但存储需要按时付费
    # =========================================================================
    DOUBAO_STANDARD: list[BillingDimension] = [
        BillingDimension(
            name="input",
            usage_field="input_tokens",
            price_field="input_price_per_1m",
        ),
        BillingDimension(
            name="output",
            usage_field="output_tokens",
            price_field="output_price_per_1m",
        ),
        BillingDimension(
            name="cache_read",
            usage_field="cache_read_tokens",
            price_field="cache_read_price_per_1m",
        ),
        BillingDimension(
            name="cache_storage",
            usage_field="cache_storage_token_hours",
            price_field="cache_storage_price_per_1m_hour",
            unit=BillingUnit.PER_1M_TOKENS_HOUR,
        ),
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]

    # =========================================================================
    # Gemini 标准计费
    # - 输入 token
    # - 输出 token
    # - 缓存读取
    # - 按次计费（用于图片生成等模型，需配置 price_per_request）
    # =========================================================================
    GEMINI_STANDARD: list[BillingDimension] = [
        BillingDimension(
            name="input",
            usage_field="input_tokens",
            price_field="input_price_per_1m",
        ),
        BillingDimension(
            name="output",
            usage_field="output_tokens",
            price_field="output_price_per_1m",
        ),
        BillingDimension(
            name="cache_read",
            usage_field="cache_read_tokens",
            price_field="cache_read_price_per_1m",
        ),
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]

    # =========================================================================
    # 按次计费
    # - 适用于某些图片生成模型、特殊 API 等
    # - 仅按请求次数计费，不按 token 计费
    # =========================================================================
    PER_REQUEST: list[BillingDimension] = [
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]

    # =========================================================================
    # 混合计费（按次 + 按 token）
    # - 某些模型既有固定费用又有 token 费用
    # =========================================================================
    HYBRID_STANDARD: list[BillingDimension] = [
        BillingDimension(
            name="input",
            usage_field="input_tokens",
            price_field="input_price_per_1m",
        ),
        BillingDimension(
            name="output",
            usage_field="output_tokens",
            price_field="output_price_per_1m",
        ),
        BillingDimension(
            name="request",
            usage_field="request_count",
            price_field="price_per_request",
            unit=BillingUnit.PER_REQUEST,
        ),
    ]


# =========================================================================
# 模板注册表
# =========================================================================

BILLING_TEMPLATE_REGISTRY: dict[str, list[BillingDimension]] = {
    # 按厂商名称
    "claude": BillingTemplates.CLAUDE_STANDARD,
    "anthropic": BillingTemplates.CLAUDE_STANDARD,
    "openai": BillingTemplates.OPENAI_STANDARD,
    "doubao": BillingTemplates.DOUBAO_STANDARD,
    "bytedance": BillingTemplates.DOUBAO_STANDARD,
    "gemini": BillingTemplates.GEMINI_STANDARD,
    "google": BillingTemplates.GEMINI_STANDARD,
    # 按计费模式
    "per_request": BillingTemplates.PER_REQUEST,
    "hybrid": BillingTemplates.HYBRID_STANDARD,
    # 默认
    "default": BillingTemplates.CLAUDE_STANDARD,
}


def get_template(name: str | None) -> list[BillingDimension]:
    """
    获取计费模板

    Args:
        name: 模板名称（不区分大小写）

    Returns:
        计费维度列表
    """
    if not name:
        return BILLING_TEMPLATE_REGISTRY["default"]

    template = BILLING_TEMPLATE_REGISTRY.get(name.lower())
    if template is None:
        available = ", ".join(sorted(BILLING_TEMPLATE_REGISTRY.keys()))
        raise ValueError(f"Unknown billing template: {name!r}. Available: {available}")

    return template


def list_templates() -> list[str]:
    """列出所有可用的模板名称"""
    return list(BILLING_TEMPLATE_REGISTRY.keys())
