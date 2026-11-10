"""Flow variables: `${name}` substitution in params and CLI override parsing.

The Rust side (crates/graph/src/vars.rs) mirrors these rules so `--set` is
validated before a job is spawned and substitution agrees with it.
"""

from __future__ import annotations

import json
import re
from typing import Any

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_WHOLE = re.compile(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)\}$")

KINDS = ("string", "int", "float", "bool", "path")


class VarError(Exception):
    """A variable name the pipeline does not declare, or a value of the wrong kind."""


def _walk(text: str):
    """Splits text into literal and variable parts. `$${` escapes to `${`."""
    index = 0
    while index < len(text):
        found = text.find("$", index)
        if found < 0:
            yield "literal", text[index:]
            return
        if found > index:
            yield "literal", text[index:found]
        if text.startswith("$${", found):
            yield "literal", "${"
            index = found + 3
            continue
        if text.startswith("${", found):
            close = text.find("}", found + 2)
            if close < 0:
                yield "literal", text[found:]
                return
            name = text[found + 2 : close]
            if _IDENT.match(name):
                yield "var", name
                index = close + 1
            else:
                yield "literal", "${"
                index = found + 2
            continue
        yield "literal", "$"
        index = found + 1


def substitute(text: str, variables: dict[str, Any]) -> Any:
    """`$${` escapes; a whole-string `${name}` returns the typed value."""
    whole = _WHOLE.match(text or "")
    if whole:
        name = whole.group(1)
        if name not in variables:
            raise VarError(f"unknown variable ${{{name}}}")
        return variables[name]
    out: list[str] = []
    for kind, part in _walk(text or ""):
        if kind == "literal":
            out.append(part)
            continue
        if part not in variables:
            raise VarError(f"unknown variable ${{{part}}}")
        out.append(as_text(variables[part]))
    return "".join(out)


def as_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    try:
        return json.dumps(value)
    except (TypeError, ValueError):
        return str(value)


def parse_value(kind: str, text: str) -> Any:
    """Parses `--set` text for a declared kind."""
    kind = str(kind or "string")
    if kind in ("string", "path"):
        return str(text)
    if kind == "int":
        try:
            return int(text)
        except (TypeError, ValueError) as exc:
            raise VarError(f'"{text}" is not an int') from exc
    if kind == "float":
        try:
            return float(text)
        except (TypeError, ValueError) as exc:
            raise VarError(f'"{text}" is not a float') from exc
    if kind == "bool":
        lowered = str(text).lower()
        if lowered in ("true", "1", "yes"):
            return True
        if lowered in ("false", "0", "no"):
            return False
        raise VarError(f'"{text}" is not a bool')
    raise VarError(f"unknown variable kind {kind}")


def resolve_variables(graph: dict[str, Any], overrides: dict[str, str]) -> dict[str, Any]:
    """Declared variables with the run overrides applied. Overrides go first."""
    variables: dict[str, Any] = {}
    kinds: dict[str, str] = {}
    for record in graph.get("variables") or []:
        if not isinstance(record, dict):
            continue
        name = str(record.get("name") or "")
        if not name:
            continue
        kinds[name] = str(record.get("kind") or "string")
        variables[name] = record.get("value")
    for name, text in (overrides or {}).items():
        if name not in variables:
            known = ", ".join(sorted(variables)) or "none are declared"
            raise VarError(f"unknown variable {name}. Known: {known}")
        variables[name] = parse_value(kinds[name], text)
    return variables


def substitute_params(
    params: Any, variables: dict[str, Any], origin: str | None = None
) -> tuple[Any, dict[str, str]]:
    """Substitutes strings deep inside a param tree, returning a fresh copy.

    The second result maps a top-level param name to the variable it came
    from when the whole value was `${name}`, so type errors can name it.
    """
    origins: dict[str, str] = {}
    substituted = _substitute_any(params, variables, origins, origin)
    return substituted, origins


def _substitute_any(value: Any, variables: dict[str, Any], origins: dict[str, str], origin: str | None) -> Any:
    if isinstance(value, str):
        if "${" not in value and "$${" not in value:
            return value
        whole = _WHOLE.match(value)
        if whole and whole.group(1) in variables:
            if origin is not None:
                origins[origin] = whole.group(1)
        return substitute(value, variables)
    if isinstance(value, dict):
        return {
            key: _substitute_any(item, variables, origins, None)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [_substitute_any(item, variables, origins, None) for item in value]
    return value


def substitute_node(
    node: dict[str, Any], variables: dict[str, Any]
) -> tuple[dict[str, Any], dict[str, str]]:
    """A copy of `node` with substituted params. Code params are untouched."""
    origins: dict[str, str] = {}
    out = dict(node)
    params: dict[str, Any] = {}
    for name, value in (node.get("params") or {}).items():
        if name == "code":
            params[name] = value
            continue
        if isinstance(value, str) and "${" not in value:
            params[name] = value
            continue
        local: dict[str, str] = {}
        params[name] = _substitute_any(value, variables, local, name)
        origins.update(local)
    out["params"] = params
    return out, origins
