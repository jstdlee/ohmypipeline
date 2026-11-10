"""Flow control: the condition language, loop iterations, and the skip token.

The grammar is the one in crates/graph/src/flow.rs. No eval and no function
calls: a condition is variables, the frame size, literals, and comparisons.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

import pandas as pd

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")

_LOOP_PREVIEW_CAP = 3


class FlowError(Exception):
    """A condition or loop setup the parser refuses. The message is short."""


class _Skip:
    """The inactive-branch sentinel. A node fed by it is skipped."""

    def __repr__(self) -> str:  # pragma: no cover - debugging aid
        return "skip"


SKIP = _Skip()


class Branch:
    """Both outputs of a multi-output node, port name -> value or SKIP."""

    def __init__(self, outputs: dict[str, Any]) -> None:
        self.outputs = outputs

    def pick(self) -> Any:
        """The active output. Both SKIP means the whole node is inactive."""
        for name in ("true", "false"):
            value = self.outputs.get(name)
            if value is not SKIP:
                return value
        return SKIP


# ---------------------------------------------------------------------------
# Condition language


def _tokenize(text: str) -> list[tuple[str, Any, int]]:
    """(kind, value, position) triples. kind is num, str, var, word, op, lpar, rpar."""
    tokens: list[tuple[str, Any, int]] = []
    index = 0
    length = len(text)
    while index < length:
        ch = text[index]
        if ch in " \t\r\n":
            index += 1
            continue
        if ch in "'\"":
            close = text.find(ch, index + 1)
            if close < 0:
                raise FlowError(f"unterminated string at {index}")
            tokens.append(("str", text[index + 1 : close], index))
            index = close + 1
            continue
        if ch == "$":
            if not text.startswith("${", index):
                raise FlowError(f"unexpected '$' at {index}")
            close = text.find("}", index + 2)
            if close < 0:
                raise FlowError(f"unterminated variable at {index}")
            name = text[index + 2 : close]
            if not _IDENT.match(name):
                raise FlowError(f"not a variable name at {index + 2}")
            tokens.append(("var", name, index))
            index = close + 1
            continue
        if text.startswith(("==", "!=", "<=", ">="), index):
            tokens.append(("op", text[index : index + 2], index))
            index += 2
            continue
        if ch in "<>":
            tokens.append(("op", ch, index))
            index += 1
            continue
        if ch == "(":
            tokens.append(("lpar", ch, index))
            index += 1
            continue
        if ch == ")":
            tokens.append(("rpar", ch, index))
            index += 1
            continue
        if ch.isdigit():
            end = index
            while end < length and (text[end].isdigit() or text[end] == "."):
                end += 1
            raw = text[index:end]
            try:
                value = float(raw) if "." in raw else int(raw)
            except ValueError as exc:
                raise FlowError(f"bad number at {index}") from exc
            tokens.append(("num", value, index))
            index = end
            continue
        if ch.isalpha() or ch == "_":
            end = index
            while end < length and (text[end].isalnum() or text[end] == "_"):
                end += 1
            word = text[index:end]
            if word in ("and", "or", "not"):
                tokens.append(("word", word, index))
            elif word in ("rows", "columns", "true", "false"):
                tokens.append(("val", word, index))
            else:
                raise FlowError(f'unknown name "{word}" at {index}')
            index = end
            continue
        raise FlowError(f"unexpected character '{ch}' at {index}")
    return tokens


class _Parser:
    """Recursive descent over the tokens, producing an AST of tuples."""

    def __init__(self, tokens: list[tuple[str, Any, int]]) -> None:
        self.tokens = tokens
        self.index = 0

    def peek(self) -> tuple[str, Any, int] | None:
        if self.index < len(self.tokens):
            return self.tokens[self.index]
        return None

    def parse_or(self) -> Any:
        node = self.parse_and()
        while (token := self.peek()) is not None and token[:2] == ("word", "or"):
            self.index += 1
            right = self.parse_and()
            node = ("or", node, right, token[2])
        return node

    def parse_and(self) -> Any:
        node = self.parse_not()
        while (token := self.peek()) is not None and token[:2] == ("word", "and"):
            self.index += 1
            right = self.parse_not()
            node = ("and", node, right, token[2])
        return node

    def parse_not(self) -> Any:
        token = self.peek()
        if token is not None and token[:2] == ("word", "not"):
            self.index += 1
            return ("not", self.parse_not(), token[2])
        return self.parse_cmp()

    def parse_cmp(self) -> Any:
        left = self.parse_operand()
        token = self.peek()
        if token is not None and token[0] == "op":
            self.index += 1
            right = self.parse_operand()
            return ("cmp", token[1], left, right, token[2])
        return left

    def parse_operand(self) -> Any:
        token = self.peek()
        if token is None:
            raise FlowError("expected a value")
        kind, value, pos = token
        if kind in ("num", "str", "var", "val"):
            self.index += 1
            return (kind, value, pos)
        if kind == "lpar":
            self.index += 1
            node = self.parse_or()
            close = self.peek()
            if close is None or close[0] != "rpar":
                raise FlowError(f"unbalanced parenthesis at {pos}")
            self.index += 1
            return node
        if kind == "word":
            raise FlowError(f'unexpected "{value}" at {pos}')
        if kind == "rpar":
            raise FlowError(f'unexpected ")" at {pos}')
        raise FlowError(f'unexpected "{value}" at {pos}')

    def finish(self) -> Any:
        node = self.parse_or()
        token = self.peek()
        if token is not None:
            raise FlowError(f'unexpected "{token[1]}" at {token[2]}')
        return node


def parse_condition(text: str) -> Any:
    """Parses a condition into an AST. Raises FlowError with a position."""
    if not text or not text.strip():
        raise FlowError("condition is empty")
    return _Parser(_tokenize(text)).finish()


_OPERATORS = {
    "==": lambda a, b: a == b,
    "!=": lambda a, b: a != b,
    "<": lambda a, b: a < b,
    "<=": lambda a, b: a <= b,
    ">": lambda a, b: a > b,
    ">=": lambda a, b: a >= b,
}


def _operand_value(node: tuple, variables: dict[str, Any], frame: pd.DataFrame | None) -> Any:
    kind, value, pos = node
    if kind == "var":
        if value not in variables:
            raise FlowError(f"unknown variable ${{{value}}} at {pos}")
        return variables[value]
    if kind == "str":
        return value
    if kind == "num":
        return value
    if value == "true":
        return True
    if value == "false":
        return False
    if value == "rows":
        if frame is None:
            raise FlowError("rows needs a wired input")
        return int(len(frame))
    if value == "columns":
        if frame is None:
            raise FlowError("columns needs a wired input")
        return int(len(frame.columns))
    raise FlowError(f'unknown name "{value}" at {pos}')


def _eval(node: Any, variables: dict[str, Any], frame: pd.DataFrame | None) -> bool:
    if isinstance(node, tuple) and node[0] == "or":
        return _eval(node[1], variables, frame) or _eval(node[2], variables, frame)
    if isinstance(node, tuple) and node[0] == "and":
        return _eval(node[1], variables, frame) and _eval(node[2], variables, frame)
    if isinstance(node, tuple) and node[0] == "not":
        return not _eval(node[1], variables, frame)
    if isinstance(node, tuple) and node[0] == "cmp":
        _, op, left, right, pos = node
        a = _operand_value(left, variables, frame)
        b = _operand_value(right, variables, frame)
        try:
            return bool(_OPERATORS[op](a, b))
        except TypeError as exc:
            raise FlowError(f"cannot compare with {op} at {pos}") from exc
    return bool(_operand_value(node, variables, frame))


def evaluate(text: str, variables: dict[str, Any], frame: pd.DataFrame | None) -> bool:
    """Evaluates a condition. `variables` holds the substituted `${name}` values."""
    return _eval(parse_condition(str(text or "")), variables or {}, frame)


# ---------------------------------------------------------------------------
# Loops


def loop_members(graph: dict[str, Any], start_id: str, end_id: str) -> list[str]:
    """Nodes the loop recomputes once per iteration, exclusive of the pair.

    Members are ancestors of the end that either descend from the start or
    mention the loop variable or ${loop_index} in any string param, plus
    every node on a path from such a node to the end. Everything else
    upstream is loop-invariant and computed once. Parents come first.
    """
    from pipeline.vars import referenced_names

    nodes = {node.get("id"): node for node in graph.get("nodes") or []}
    ancestors: list[str] = []
    seen: set[str] = set()
    stack: set[str] = set()

    def walk(current: str) -> None:
        if current in stack:
            return
        if current in seen:
            return
        stack.add(current)
        for edge in graph.get("edges") or []:
            if edge.get("to") == current:
                walk(edge.get("from"))
        stack.remove(current)
        seen.add(current)
        ancestors.append(current)

    walk(end_id)
    # The loop variables a body may use: `${name}` and `${loop_index}`.
    start = nodes.get(start_id)
    name = str(((start or {}).get("params") or {}).get("name") or "loop_value")
    loop_names = {name, "loop_index"}
    below: set[str] = set()
    queue = [start_id]
    while queue:
        current = queue.pop()
        for edge in graph.get("edges") or []:
            if edge.get("from") == current and edge.get("to") not in below:
                below.add(edge.get("to"))
                queue.append(edge.get("to"))
    inside = [node_id for node_id in ancestors if node_id not in (start_id, end_id)]
    seeds: set[str] = set()
    for node_id in inside:
        mentions = referenced_names((nodes.get(node_id) or {}).get("params") or {})
        if node_id in below or mentions & loop_names:
            seeds.add(node_id)
    parents: dict[str, list[str]] = {}
    for edge in graph.get("edges") or []:
        parents.setdefault(edge.get("to"), []).append(edge.get("from"))
    members: set[str] = set()
    ordered: list[str] = []
    for node_id in inside:
        if node_id in seeds or any(parent in members for parent in parents.get(node_id, ())):
            members.add(node_id)
            ordered.append(node_id)
    return ordered


def paired_loop_starts(graph: dict[str, Any]) -> dict[str, str]:
    """loop_end id -> loop_start id, for pairs that resolve."""
    nodes = {node.get("id"): node for node in graph.get("nodes") or []}
    pairs: dict[str, str] = {}
    for node in graph.get("nodes") or []:
        if node.get("type") != "loop_end":
            continue
        start_id = str((node.get("params") or {}).get("pair") or "")
        start = nodes.get(start_id)
        if start is not None and start.get("type") == "loop_start":
            pairs[node.get("id")] = start_id
    return pairs


def loop_iterations(
    loop_start: dict[str, Any],
    *,
    base_dir: Path,
    input_frame: pd.DataFrame | None = None,
) -> list[tuple[str, dict[str, Any], pd.DataFrame]]:
    """One entry per iteration: (label, bindings, loop_start output frame).

    `loop_start` must already have substituted params. Bindings hold the
    loop variable named by the `name` param plus `loop_index`.
    """
    params = loop_start.get("params") or {}
    mode = str(params.get("mode") or "files")
    name = str(params.get("name") or "loop_value")
    out: list[tuple[str, dict[str, Any], pd.DataFrame]] = []
    if mode == "files":
        pattern = str(params.get("pattern") or "")
        if not pattern:
            raise FlowError("files mode needs a pattern")
        base = Path(base_dir)
        matches = sorted(path for path in base.glob(pattern) if path.is_file())
        for index, path in enumerate(matches):
            relative = path.relative_to(base).as_posix()
            bindings = {name: str(path.resolve()), "loop_index": index}
            frame = pd.DataFrame({"path": [relative], "name": [path.stem]})
            out.append((path.name, bindings, frame))
        return out
    if mode == "groups":
        if input_frame is None or not isinstance(input_frame, pd.DataFrame):
            raise FlowError("groups mode needs an input")
        column = str(params.get("column") or "")
        if column not in input_frame.columns:
            raise FlowError(f"missing column {column}")
        values = list(pd.unique(input_frame[column].dropna()))
        try:
            values.sort()
        except TypeError:
            values.sort(key=str)
        for index, value in enumerate(values):
            sub = input_frame[input_frame[column] == value]
            bindings = {name: value, "loop_index": index}
            out.append((str(value), bindings, sub))
        return out
    if mode == "values":
        from pipeline.vars import as_text

        raw = params.get("values")
        text = as_text("" if raw is None else raw)
        parts = [part.strip() for part in text.split(",") if part.strip()]
        frame = input_frame if isinstance(input_frame, pd.DataFrame) else pd.DataFrame()
        for index, part in enumerate(parts):
            bindings = {name: part, "loop_index": index}
            out.append((part, bindings, frame))
        return out
    raise FlowError(f"unknown loop mode {mode}")
