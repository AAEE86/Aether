"""
Token计数插件基类
定义Token计数的接口
"""

from __future__ import annotations
from abc import abstractmethod
from dataclasses import dataclass
from typing import Any

from src.plugins.common import BasePlugin


@dataclass
class TokenUsage:
    """令牌使用情况"""

    input_tokens: int = 0
    output_tokens: int = 0
    total_tokens: int = 0
    cache_read_tokens: int = 0  # Claude缓存读取
    cache_write_tokens: int = 0  # Claude缓存写入
    reasoning_tokens: int = 0  # OpenAI o1推理令牌

    def __add__(self, other: TokenUsage) -> TokenUsage:
        """令牌使用相加"""
        return TokenUsage(
            input_tokens=self.input_tokens + other.input_tokens,
            output_tokens=self.output_tokens + other.output_tokens,
            total_tokens=self.total_tokens + other.total_tokens,
            cache_read_tokens=self.cache_read_tokens + other.cache_read_tokens,
            cache_write_tokens=self.cache_write_tokens + other.cache_write_tokens,
            reasoning_tokens=self.reasoning_tokens + other.reasoning_tokens,
        )

    def to_dict(self) -> dict[str, int]:
        """转换为字典"""
        return {
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "cache_read_tokens": self.cache_read_tokens,
            "cache_write_tokens": self.cache_write_tokens,
            "reasoning_tokens": self.reasoning_tokens,
        }


class TokenCounterPlugin(BasePlugin):
    """
    Token计数插件基类
    支持不同模型的Token计数
    """

    def __init__(self, name: str = "token_counter", config: dict[str, Any] = None):
        # 调用父类初始化，设置metadata
        super().__init__(
            name=name, config=config, description="Token Counter Plugin", version="1.0.0"
        )

        self.supported_models = self.config.get("supported_models", [])
        self.default_model = self.config.get("default_model")

    @abstractmethod
    def supports_model(self, model: str) -> bool:
        """检查是否支持指定模型"""
        pass

    @abstractmethod
    async def count_tokens(self, text: str, model: str | None = None) -> int:
        """计算文本的Token数量"""
        pass

    @abstractmethod
    async def count_messages(
        self, messages: list[dict[str, Any]], model: str | None = None
    ) -> int:
        """计算消息列表的Token数量"""
        pass

    async def count_request(self, request: dict[str, Any], model: str | None = None) -> int:
        """计算请求的Token数量"""
        model = model or request.get("model") or self.default_model
        messages = request.get("messages", [])
        return await self.count_messages(messages, model)

    async def count_response(
        self, response: dict[str, Any], model: str | None = None
    ) -> TokenUsage:
        """从响应中提取Token使用情况"""
        usage = response.get("usage", {})

        # OpenAI格式
        if "prompt_tokens" in usage:
            return TokenUsage(
                input_tokens=usage.get("prompt_tokens", 0),
                output_tokens=usage.get("completion_tokens", 0),
                total_tokens=usage.get("total_tokens", 0),
                reasoning_tokens=usage.get("completion_tokens_details", {}).get(
                    "reasoning_tokens", 0
                ),
            )

        # Claude格式
        elif "input_tokens" in usage:
            return TokenUsage(
                input_tokens=usage.get("input_tokens", 0),
                output_tokens=usage.get("output_tokens", 0),
                total_tokens=usage.get("input_tokens", 0) + usage.get("output_tokens", 0),
                cache_read_tokens=usage.get("cache_read_input_tokens", 0),
                cache_write_tokens=usage.get("cache_creation_input_tokens", 0),
            )

        return TokenUsage()

    async def estimate_cost(
        self, usage: TokenUsage, model: str, provider: str | None = None
    ) -> dict[str, float]:
        """估算使用成本"""
        # 默认价格表（每1M tokens的价格）
        pricing = self.config.get("pricing", {})

        # 获取模型价格
        model_pricing = pricing.get(model, {})
        if not model_pricing:
            # 尝试使用前缀匹配
            for model_prefix, price_info in pricing.items():
                if model.startswith(model_prefix):
                    model_pricing = price_info
                    break

        if not model_pricing:
            return {"error": "No pricing information available"}

        # 计算成本
        input_cost = (usage.input_tokens / 1_000_000) * model_pricing.get("input", 0)
        output_cost = (usage.output_tokens / 1_000_000) * model_pricing.get("output", 0)

        # 缓存成本（Claude特有）
        cache_read_cost = (usage.cache_read_tokens / 1_000_000) * model_pricing.get("cache_read", 0)
        cache_write_cost = (usage.cache_write_tokens / 1_000_000) * model_pricing.get(
            "cache_write", 0
        )

        # 推理成本（OpenAI o1特有）
        reasoning_cost = (usage.reasoning_tokens / 1_000_000) * model_pricing.get("reasoning", 0)

        total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost + reasoning_cost

        return {
            "input_cost": round(input_cost, 6),
            "output_cost": round(output_cost, 6),
            "cache_read_cost": round(cache_read_cost, 6),
            "cache_write_cost": round(cache_write_cost, 6),
            "reasoning_cost": round(reasoning_cost, 6),
            "total_cost": round(total_cost, 6),
            "currency": "USD",
        }

    @abstractmethod
    async def get_model_info(self, model: str) -> dict[str, Any]:
        """获取模型信息"""
        pass

    async def get_stats(self) -> dict[str, Any]:
        """获取统计信息"""
        return {
            "type": self.name,
            "enabled": self.enabled,
            "supported_models": self.supported_models,
            "default_model": self.default_model,
        }
