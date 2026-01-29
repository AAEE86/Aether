"""
优化工具类 - 包含Token计数和响应头管理
"""

from typing import Any

import tiktoken


class TokenCounter:
    """
    改进的Token计数器
    支持多种模型的准确计数
    """

    # 模型到编码器的映射
    MODEL_TO_ENCODING = {
        "gpt-4": "cl100k_base",
        "gpt-3.5-turbo": "cl100k_base",
        "claude-3": "cl100k_base",  # Claude使用类似的tokenizer
        "claude-2": "cl100k_base",
    }

    def __init__(self):
        self._encodings = {}
        self._default_encoding = None

    def _get_encoding(self, model: str):
        """获取模型对应的编码器"""
        # 标准化模型名称
        model_base = model.lower().split("-")[0]

        if model_base not in self._encodings:
            encoding_name = self.MODEL_TO_ENCODING.get(model_base, "cl100k_base")  # 默认编码器
            try:
                self._encodings[model_base] = tiktoken.get_encoding(encoding_name)
            except Exception:
                # 如果失败，使用默认编码器
                if not self._default_encoding:
                    self._default_encoding = tiktoken.get_encoding("cl100k_base")
                self._encodings[model_base] = self._default_encoding

        return self._encodings[model_base]

    def count_tokens(self, text: str, model: str = "claude-3") -> int:
        """
        精确计算文本的token数量
        """
        if not text:
            return 0

        try:
            encoding = self._get_encoding(model)
            return len(encoding.encode(text))
        except Exception:
            # 降级到简单估算
            return len(text) // 4

    def count_messages_tokens(self, messages: list, model: str = "claude-3") -> int:
        """
        计算消息列表的总token数
        """
        total = 0
        for message in messages:
            if isinstance(message, dict):
                # 计算角色标记
                total += 4  # 角色和分隔符的开销

                # 计算内容
                content = message.get("content", "")
                if isinstance(content, str):
                    total += self.count_tokens(content, model)
                elif isinstance(content, list):
                    # 处理多模态内容
                    for item in content:
                        if isinstance(item, dict) and "text" in item:
                            total += self.count_tokens(item["text"], model)

        return total

    def estimate_response_tokens(self, response: Any, model: str = "claude-3") -> int:
        """
        估算响应的token数量
        """
        if isinstance(response, dict):
            # 尝试从响应中提取内容
            if "content" in response:
                content = response["content"]
                if isinstance(content, list):
                    text = " ".join(
                        item.get("text", "") for item in content if isinstance(item, dict)
                    )
                else:
                    text = str(content)
                return self.count_tokens(text, model)
            elif "choices" in response:
                # OpenAI格式
                total = 0
                for choice in response.get("choices", []):
                    message = choice.get("message", {})
                    content = message.get("content", "")
                    total += self.count_tokens(content, model)
                return total

        # 降级到简单估算
        return len(str(response)) // 4
