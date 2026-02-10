from typing import Any

from src.api.handlers.base.request_builder import apply_body_rules


class TestApplyBodyRulesNestedPaths:
    def test_set_nested_value(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": "b.c.d", "value": 42},
            ],
        )
        assert result == {"a": 1, "b": {"c": {"d": 42}}}

    def test_drop_nested_value(self) -> None:
        body = {"a": {"b": {"c": 1, "d": 2}}}
        result = apply_body_rules(
            body,
            [
                {"action": "drop", "path": "a.b.c"},
            ],
        )
        assert result == {"a": {"b": {"d": 2}}}

    def test_rename_nested_value(self) -> None:
        body = {"old": {"nested": "value"}}
        result = apply_body_rules(
            body,
            [
                {"action": "rename", "from": "old.nested", "to": "new.path"},
            ],
        )
        assert result == {"old": {}, "new": {"path": "value"}}

    def test_protected_top_level_key(self) -> None:
        body = {"model": "gpt-4", "extra": {"model": "ignored"}}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": "model.sub", "value": "x"},  # 应被忽略
                {"action": "set", "path": "extra.model", "value": "y"},  # 应生效
            ],
        )
        assert result["model"] == "gpt-4"  # 顶层受保护，不变
        assert result["extra"]["model"] == "y"  # extra 不受保护

    def test_escaped_dot(self) -> None:
        body = {}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": "config\\.v1.enabled", "value": True},
            ],
        )
        assert result == {"config.v1": {"enabled": True}}

    def test_invalid_paths_are_ignored(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": ".b", "value": 2},
                {"action": "set", "path": "c..d", "value": 3},
                {"action": "drop", "path": "e.", "value": None},
                {"action": "rename", "from": "x..y", "to": "z", "value": None},
            ],
        )
        assert result == {"a": 1}

    def test_non_dict_intermediate_is_overwritten(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": "a.b", "value": 2},
            ],
        )
        assert result == {"a": {"b": 2}}

    def test_set_complex_value(self) -> None:
        body = {}
        result = apply_body_rules(
            body,
            [
                {"action": "set", "path": "metadata.tags", "value": [1, 2]},
                {"action": "set", "path": "metadata.obj", "value": {"a": 1}},
            ],
        )
        assert result == {"metadata": {"tags": [1, 2], "obj": {"a": 1}}}

    def test_does_not_mutate_original_body(self) -> None:
        body = {"a": {"b": 1}}
        result = apply_body_rules(body, [{"action": "set", "path": "a.c", "value": 2}])
        assert result == {"a": {"b": 1, "c": 2}}
        assert body == {"a": {"b": 1}}


class TestSetWithOriginalPlaceholder:
    """set 操作中 {{$original}} 占位符的测试"""

    def test_wrap_string_in_object(self) -> None:
        """核心用例：字符串值包裹成对象结构"""
        body = {"para": "text"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "para", "value": [{"text": "{{$original}}"}]}],
        )
        assert result == {"para": [{"text": "text"}]}

    def test_exact_placeholder_preserves_number(self) -> None:
        """完全匹配占位符时保留原始类型: number"""
        body = {"count": 42}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "count", "value": {"num": "{{$original}}"}}],
        )
        assert result == {"count": {"num": 42}}
        assert isinstance(result["count"]["num"], int)

    def test_exact_placeholder_preserves_dict(self) -> None:
        """完全匹配占位符时保留原始类型: dict"""
        body = {"data": {"key": "val"}}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "data", "value": {"wrapped": "{{$original}}"}}],
        )
        assert result == {"data": {"wrapped": {"key": "val"}}}

    def test_exact_placeholder_preserves_list(self) -> None:
        """完全匹配占位符时保留原始类型: list"""
        body = {"items": [1, 2, 3]}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "items", "value": {"arr": "{{$original}}"}}],
        )
        assert result == {"items": {"arr": [1, 2, 3]}}

    def test_partial_placeholder_converts_to_string(self) -> None:
        """部分匹配时将原值转为 str 拼接"""
        body = {"name": "world"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "name", "value": "hello_{{$original}}_suffix"}],
        )
        assert result == {"name": "hello_world_suffix"}

    def test_partial_placeholder_with_number(self) -> None:
        """部分匹配 + 非字符串原值: 数字转 str"""
        body = {"ver": 3}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "ver", "value": "version_{{$original}}"}],
        )
        assert result == {"ver": "version_3"}

    def test_no_placeholder_acts_like_plain_set(self) -> None:
        """不含占位符时行为与原 set 完全一致"""
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "a", "value": {"new": "val"}}],
        )
        assert result == {"a": {"new": "val"}}

    def test_missing_path_uses_none(self) -> None:
        """路径不存在时 original 为 None"""
        body: dict[str, Any] = {}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "missing", "value": {"was": "{{$original}}"}}],
        )
        assert result == {"missing": {"was": None}}

    def test_nested_path(self) -> None:
        """嵌套路径"""
        body = {"a": {"b": "original"}}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "a.b", "value": [{"content": "{{$original}}"}]}],
        )
        assert result == {"a": {"b": [{"content": "original"}]}}

    def test_protected_field_ignored(self) -> None:
        """受保护字段（model, stream）跳过"""
        body = {"model": "gpt-4", "other": "val"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "model", "value": "{{$original}}_modified"}],
        )
        assert result["model"] == "gpt-4"

    def test_does_not_mutate_original(self) -> None:
        """不修改原始 body"""
        body = {"x": "original"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "x", "value": {"wrapped": "{{$original}}"}}],
        )
        assert result == {"x": {"wrapped": "original"}}
        assert body == {"x": "original"}

    def test_multiple_placeholders_in_one_string(self) -> None:
        """一个字符串中多个占位符"""
        body = {"val": "X"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "val", "value": "{{$original}}-{{$original}}"}],
        )
        assert result == {"val": "X-X"}

    def test_deeply_nested_template(self) -> None:
        """深层嵌套模板"""
        body = {"data": "hello"}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "data", "value": {"a": {"b": [{"c": "{{$original}}"}]}}}],
        )
        assert result == {"data": {"a": {"b": [{"c": "hello"}]}}}


class TestConditionalBodyRules:
    """条件触发 body_rules 的测试"""

    # ---- 向后兼容 ----

    def test_no_condition_always_executes(self) -> None:
        """无 condition 字段的规则无条件执行"""
        body = {"a": 1}
        result = apply_body_rules(body, [{"action": "set", "path": "b", "value": 2}])
        assert result == {"a": 1, "b": 2}

    def test_condition_none_always_executes(self) -> None:
        """condition 为 None 时无条件执行"""
        body = {"a": 1}
        result = apply_body_rules(
            body, [{"action": "set", "path": "b", "value": 2, "condition": None}]
        )
        assert result == {"a": 1, "b": 2}

    # ---- 无效 condition -> 跳过规则 (fail-closed) ----

    def test_invalid_condition_not_dict_skips(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body, [{"action": "set", "path": "b", "value": 2, "condition": "bad"}]
        )
        assert result == {"a": 1}

    def test_invalid_condition_missing_op_skips(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body, [{"action": "set", "path": "b", "value": 2, "condition": {"path": "a"}}]
        )
        assert result == {"a": 1}

    def test_invalid_condition_missing_path_skips(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [{"action": "set", "path": "b", "value": 2, "condition": {"op": "eq", "value": 1}}],
        )
        assert result == {"a": 1}

    def test_invalid_condition_unknown_op_skips(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "b",
                    "value": 2,
                    "condition": {"path": "a", "op": "like", "value": 1},
                }
            ],
        )
        assert result == {"a": 1}

    # ---- eq / neq ----

    def test_eq_match(self) -> None:
        body = {"model": "claude-3"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "temp",
                    "value": 0.5,
                    "condition": {"path": "model", "op": "eq", "value": "claude-3"},
                }
            ],
        )
        assert result["temp"] == 0.5

    def test_eq_no_match(self) -> None:
        body = {"model": "gpt-4"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "temp",
                    "value": 0.5,
                    "condition": {"path": "model", "op": "eq", "value": "claude-3"},
                }
            ],
        )
        assert "temp" not in result

    def test_neq_match(self) -> None:
        body = {"model": "gpt-4"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "temp",
                    "value": 0.5,
                    "condition": {"path": "model", "op": "neq", "value": "claude-3"},
                }
            ],
        )
        assert result["temp"] == 0.5

    def test_neq_no_match(self) -> None:
        body = {"model": "claude-3"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "temp",
                    "value": 0.5,
                    "condition": {"path": "model", "op": "neq", "value": "claude-3"},
                }
            ],
        )
        assert "temp" not in result

    # ---- gt / lt / gte / lte ----

    def test_gt_match(self) -> None:
        body = {"score": 80}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "passed",
                    "value": True,
                    "condition": {"path": "score", "op": "gt", "value": 60},
                }
            ],
        )
        assert result["passed"] is True

    def test_gt_no_match(self) -> None:
        body = {"score": 60}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "passed",
                    "value": True,
                    "condition": {"path": "score", "op": "gt", "value": 60},
                }
            ],
        )
        assert "passed" not in result

    def test_lt_match(self) -> None:
        body = {"temp": 0.3}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "low",
                    "value": True,
                    "condition": {"path": "temp", "op": "lt", "value": 0.5},
                }
            ],
        )
        assert result["low"] is True

    def test_gte_match_equal(self) -> None:
        body = {"count": 10}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "count", "op": "gte", "value": 10},
                }
            ],
        )
        assert result["ok"] is True

    def test_lte_match(self) -> None:
        body = {"count": 5}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "count", "op": "lte", "value": 5},
                }
            ],
        )
        assert result["ok"] is True

    def test_numeric_op_on_non_numeric_returns_false(self) -> None:
        body = {"val": "hello"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "gt", "value": 0},
                }
            ],
        )
        assert "ok" not in result

    # ---- starts_with / ends_with ----

    def test_starts_with_match(self) -> None:
        body = {"model": "claude-3-opus"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "provider",
                    "value": "anthropic",
                    "condition": {"path": "model", "op": "starts_with", "value": "claude"},
                }
            ],
        )
        assert result["provider"] == "anthropic"

    def test_starts_with_no_match(self) -> None:
        body = {"model": "gpt-4"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "provider",
                    "value": "anthropic",
                    "condition": {"path": "model", "op": "starts_with", "value": "claude"},
                }
            ],
        )
        assert "provider" not in result

    def test_ends_with_match(self) -> None:
        body = {"model": "claude-3-opus"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "tier",
                    "value": "top",
                    "condition": {"path": "model", "op": "ends_with", "value": "opus"},
                }
            ],
        )
        assert result["tier"] == "top"

    def test_starts_with_on_non_string_returns_false(self) -> None:
        body = {"val": 123}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "starts_with", "value": "1"},
                }
            ],
        )
        assert "ok" not in result

    # ---- contains ----

    def test_contains_string_match(self) -> None:
        body = {"prompt": "hello world"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "found",
                    "value": True,
                    "condition": {"path": "prompt", "op": "contains", "value": "world"},
                }
            ],
        )
        assert result["found"] is True

    def test_contains_array_match(self) -> None:
        body = {"tags": ["a", "b", "c"]}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "found",
                    "value": True,
                    "condition": {"path": "tags", "op": "contains", "value": "b"},
                }
            ],
        )
        assert result["found"] is True

    def test_contains_array_no_match(self) -> None:
        body = {"tags": ["a", "b"]}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "found",
                    "value": True,
                    "condition": {"path": "tags", "op": "contains", "value": "z"},
                }
            ],
        )
        assert "found" not in result

    # ---- matches (regex) ----

    def test_matches_regex_match(self) -> None:
        body = {"model": "claude-3.5-sonnet"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "matched",
                    "value": True,
                    "condition": {"path": "model", "op": "matches", "value": r"claude-\d+"},
                }
            ],
        )
        assert result["matched"] is True

    def test_matches_regex_no_match(self) -> None:
        body = {"model": "gpt-4o"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "matched",
                    "value": True,
                    "condition": {"path": "model", "op": "matches", "value": r"^claude"},
                }
            ],
        )
        assert "matched" not in result

    def test_matches_invalid_regex_returns_false(self) -> None:
        body = {"val": "test"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "matches", "value": "[invalid"},
                }
            ],
        )
        assert "ok" not in result

    # ---- exists / not_exists ----

    def test_exists_match(self) -> None:
        body = {"metadata": {"key": "val"}}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "has_meta",
                    "value": True,
                    "condition": {"path": "metadata", "op": "exists"},
                }
            ],
        )
        assert result["has_meta"] is True

    def test_exists_no_match(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "has_meta",
                    "value": True,
                    "condition": {"path": "metadata", "op": "exists"},
                }
            ],
        )
        assert "has_meta" not in result

    def test_not_exists_match(self) -> None:
        body = {"a": 1}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "metadata",
                    "value": {},
                    "condition": {"path": "metadata", "op": "not_exists"},
                }
            ],
        )
        assert result["metadata"] == {}

    def test_not_exists_no_match(self) -> None:
        body = {"metadata": "exists"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "created",
                    "value": True,
                    "condition": {"path": "metadata", "op": "not_exists"},
                }
            ],
        )
        assert "created" not in result

    # ---- in ----

    def test_in_match(self) -> None:
        body = {"model": "claude-3"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "supported",
                    "value": True,
                    "condition": {
                        "path": "model",
                        "op": "in",
                        "value": ["claude-3", "claude-3.5", "gpt-4"],
                    },
                }
            ],
        )
        assert result["supported"] is True

    def test_in_no_match(self) -> None:
        body = {"model": "gemini-pro"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "supported",
                    "value": True,
                    "condition": {"path": "model", "op": "in", "value": ["claude-3", "gpt-4"]},
                }
            ],
        )
        assert "supported" not in result

    def test_in_non_list_value_returns_false(self) -> None:
        body = {"model": "claude"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "model", "op": "in", "value": "claude"},
                }
            ],
        )
        assert "ok" not in result

    # ---- type_is ----

    def test_type_is_string(self) -> None:
        body = {"val": "hello"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "string"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_number_excludes_bool(self) -> None:
        body = {"val": True}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "number"},
                }
            ],
        )
        assert "ok" not in result

    def test_type_is_number_int(self) -> None:
        body = {"val": 42}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "number"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_boolean(self) -> None:
        body = {"val": False}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "boolean"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_array(self) -> None:
        body = {"val": [1, 2]}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "array"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_object(self) -> None:
        body = {"val": {"a": 1}}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "object"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_null(self) -> None:
        body = {"val": None}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "null"},
                }
            ],
        )
        assert result["ok"] is True

    def test_type_is_invalid_type_name(self) -> None:
        body = {"val": "hello"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "ok",
                    "value": True,
                    "condition": {"path": "val", "op": "type_is", "value": "str"},
                }
            ],
        )
        assert "ok" not in result

    # ---- 链式触发 ----

    def test_chain_second_rule_sees_first_rule_changes(self) -> None:
        """第二条规则的 condition 评估的是第一条规则修改后的 body"""
        body: dict[str, Any] = {"a": 1}
        result = apply_body_rules(
            body,
            [
                # 规则 1: 无条件创建 metadata
                {"action": "set", "path": "metadata", "value": {}},
                # 规则 2: 仅当 metadata 存在且是 object 时设置子字段
                {
                    "action": "set",
                    "path": "metadata.gateway",
                    "value": "aether",
                    "condition": {"path": "metadata", "op": "type_is", "value": "object"},
                },
            ],
        )
        assert result == {"a": 1, "metadata": {"gateway": "aether"}}

    def test_chain_condition_on_previously_set_value(self) -> None:
        """条件引用前序规则 set 的值"""
        body: dict[str, Any] = {}
        result = apply_body_rules(
            body,
            [
                # 规则 1: 创建标记
                {"action": "set", "path": "flag", "value": "enabled"},
                # 规则 2: 当 flag == "enabled" 时设置
                {
                    "action": "set",
                    "path": "extra",
                    "value": 42,
                    "condition": {"path": "flag", "op": "eq", "value": "enabled"},
                },
            ],
        )
        assert result == {"flag": "enabled", "extra": 42}

    def test_chain_not_exists_then_set(self) -> None:
        """经典链式模式: not_exists 创建 → 后续规则 condition 检测已创建"""
        body: dict[str, Any] = {}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "metadata",
                    "value": {},
                    "condition": {"path": "metadata", "op": "not_exists"},
                },
                {
                    "action": "set",
                    "path": "metadata.source",
                    "value": "api",
                    "condition": {"path": "metadata", "op": "exists"},
                },
            ],
        )
        assert result == {"metadata": {"source": "api"}}

    def test_chain_skipped_rule_does_not_affect_subsequent(self) -> None:
        """被跳过的规则不影响后续规则的 condition 评估"""
        body = {"x": 1}
        result = apply_body_rules(
            body,
            [
                # 规则 1: 条件不满足，跳过
                {
                    "action": "set",
                    "path": "y",
                    "value": 2,
                    "condition": {"path": "x", "op": "eq", "value": 999},
                },
                # 规则 2: y 不存在（因为规则 1 被跳过了）
                {
                    "action": "set",
                    "path": "z",
                    "value": 3,
                    "condition": {"path": "y", "op": "not_exists"},
                },
            ],
        )
        assert "y" not in result
        assert result["z"] == 3

    # ---- 条件与其他 action 类型配合 ----

    def test_condition_with_drop_action(self) -> None:
        body = {"temp": 0.5, "model": "claude-3"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "drop",
                    "path": "temp",
                    "condition": {"path": "model", "op": "starts_with", "value": "claude"},
                }
            ],
        )
        assert "temp" not in result

    def test_condition_with_rename_action(self) -> None:
        body = {"old_key": "value", "model": "gpt-4"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "rename",
                    "from": "old_key",
                    "to": "new_key",
                    "condition": {"path": "model", "op": "starts_with", "value": "gpt"},
                }
            ],
        )
        assert "old_key" not in result
        assert result["new_key"] == "value"

    def test_condition_prevents_rename(self) -> None:
        body = {"old_key": "value", "model": "claude-3"}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "rename",
                    "from": "old_key",
                    "to": "new_key",
                    "condition": {"path": "model", "op": "starts_with", "value": "gpt"},
                }
            ],
        )
        assert result["old_key"] == "value"
        assert "new_key" not in result

    # ---- 嵌套路径条件 ----

    def test_condition_on_nested_path(self) -> None:
        body = {"config": {"mode": "advanced"}, "extra": 1}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "feature",
                    "value": True,
                    "condition": {"path": "config.mode", "op": "eq", "value": "advanced"},
                }
            ],
        )
        assert result["feature"] is True

    def test_condition_on_missing_nested_path(self) -> None:
        body = {"config": {}}
        result = apply_body_rules(
            body,
            [
                {
                    "action": "set",
                    "path": "feature",
                    "value": True,
                    "condition": {"path": "config.mode", "op": "eq", "value": "advanced"},
                }
            ],
        )
        assert "feature" not in result
