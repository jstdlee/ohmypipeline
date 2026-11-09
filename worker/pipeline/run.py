"""Run one node and its ancestors. Preview caps readers, never the rest of the graph."""

from __future__ import annotations

import hashlib
import json
import math
import re
import sqlite3
from datetime import date, datetime
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd

PREVIEW_ROW_CAP = 10_000
TABLE_WINDOW = 500

# How many times a node was actually computed. Tests use this to see the cache.
stats = {"computed": 0}

_cache: dict[str, Any] = {}

_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class NodeError(Exception):
    def __init__(
        self,
        node: str,
        message: str,
        declared: str | None = None,
        actual: str | None = None,
    ) -> None:
        super().__init__(message)
        self.node = node
        self.message = message
        self.declared = declared
        self.actual = actual


def run_job(job: dict[str, Any]) -> dict[str, Any]:
    generation = int(job.get("generation") or 0)
    mode = job.get("mode") or "preview"
    if mode not in ("preview", "full"):
        mode = "preview"
    graph = job.get("graph") or {}
    node_id = str(job.get("node") or "")
    base_dir = Path(job["base_dir"]) if job.get("base_dir") else Path.cwd()
    overrides = job.get("inputs") or {}
    op = job.get("op") or "preview"
    window = job.get("window")
    if window is None:
        window = PREVIEW_ROW_CAP if mode == "preview" else TABLE_WINDOW
    window = int(window)

    try:
        if not node_id:
            raise NodeError("", "no node was requested")
        order = ancestors(graph, node_id)
        values: dict[str, Any] = {}
        keys: dict[str, str] = {}
        for current in order:
            value, key = compute_node(
                graph,
                current,
                values,
                keys,
                mode=mode,
                base_dir=base_dir,
                overrides=overrides,
            )
            values[current] = value
            keys[current] = key
        result = values[node_id]
        node = find_node(graph, node_id)
        if node and node.get("type") == "chart":
            # The chart has no table output. The preview is its input.
            incoming = edges_into(graph, node_id)
            if not incoming:
                raise NodeError(node_id, "chart has no input")
            result = values[incoming[0]["from"]]
        if op == "export":
            path = job.get("output")
            if not path:
                raise NodeError(node_id, "export needs an output path")
            write_csv(result, Path(path))
            payload = empty_payload(result)
            payload["csv_path"] = str(path)
        else:
            payload = payload_of(result, window)
        payload.update(
            {
                "generation": generation,
                "mode": mode,
                "node": node_id,
                "claim": mode,
                "error": None,
            }
        )
        return payload
    except NodeError as exc:
        return {
            "generation": generation,
            "mode": mode,
            "node": exc.node or node_id,
            "columns": [],
            "dtypes": [],
            "index": [],
            "rows": [],
            "n": 0,
            "claim": mode,
            "error": {
                "node": exc.node or node_id,
                "message": exc.message,
                "declared": exc.declared,
                "actual": exc.actual,
            },
        }


def ancestors(graph: dict[str, Any], node_id: str) -> list[str]:
    nodes = {node["id"]: node for node in graph.get("nodes") or []}
    if node_id not in nodes:
        raise NodeError(node_id, f"unknown node {node_id}")
    order: list[str] = []
    seen: set[str] = set()
    stack: set[str] = set()

    def walk(current: str) -> None:
        if current in stack:
            raise NodeError(current, f"cycle through {current}")
        if current in seen:
            return
        stack.add(current)
        for edge in edges_into(graph, current):
            walk(edge["from"])
        stack.remove(current)
        seen.add(current)
        order.append(current)

    walk(node_id)
    return order


def edges_into(graph: dict[str, Any], node_id: str, port: str | None = None) -> list[dict[str, Any]]:
    found = []
    for edge in graph.get("edges") or []:
        if edge.get("to") != node_id:
            continue
        if port is not None and edge.get("to_port") != port:
            continue
        found.append(edge)
    return found


def find_node(graph: dict[str, Any], node_id: str) -> dict[str, Any] | None:
    for node in graph.get("nodes") or []:
        if node.get("id") == node_id:
            return node
    return None


def compute_node(
    graph: dict[str, Any],
    node_id: str,
    values: dict[str, Any],
    keys: dict[str, str],
    *,
    mode: str,
    base_dir: Path,
    overrides: dict[str, str],
) -> tuple[Any, str]:
    node = find_node(graph, node_id)
    if node is None:
        raise NodeError(node_id, f"unknown node {node_id}")
    if node.get("unknown"):
        raise NodeError(node_id, f"unknown feature {node.get('type')}")
    parent_edges = edges_into(graph, node_id)
    parent_keys = [
        {"from": edge["from"], "port": edge.get("to_port"), "key": keys.get(edge["from"])}
        for edge in parent_edges
    ]
    record = {
        "type": node.get("type"),
        "params": node.get("params") or {},
        "inputs": node.get("inputs"),
        "added_inputs": node.get("added_inputs"),
        "wired": sorted(node.get("wired") or []),
        "output": node.get("output"),
        "parents": parent_keys,
        "mode": mode,
        "override": overrides.get(node_id),
    }
    key = hashlib.sha256(json.dumps(record, sort_keys=True, default=str).encode()).hexdigest()
    if key in _cache:
        return _cache[key], key
    stats["computed"] += 1
    value = execute(graph, node, values, mode=mode, base_dir=base_dir, overrides=overrides)
    declared = declared_type(node, graph, values)
    if declared is not None:
        actual = actual_type(value, declared)
        if actual != declared:
            raise NodeError(
                node_id,
                f"declared {declared}, pandas returned {actual}",
                declared=declared,
                actual=actual,
            )
    _cache[key] = value
    return value, key


def declared_type(node: dict[str, Any], graph: dict[str, Any], values: dict[str, Any]) -> str | None:
    kind = node.get("type")
    if kind in ("read_csv", "read_excel", "read_sql", "change_headers", "query", "to_frame", "merge"):
        return "dataframe"
    if kind == "column":
        return "series"
    if kind == "literal":
        return str((node.get("params") or {}).get("kind") or "scalar")
    if kind == "python":
        output = node.get("output") or {}
        accepts = output.get("accepts") or ["dataframe"]
        return accepts[0]
    if kind in ("head", "sort_values", "concat"):
        return input_type(node, graph, values)
    return None


def input_type(node: dict[str, Any], graph: dict[str, Any], values: dict[str, Any]) -> str | None:
    incoming = edges_into(graph, node["id"])
    if len(incoming) != 1:
        return None
    parent = find_node(graph, incoming[0]["from"])
    if parent is None:
        return None
    return declared_type(parent, graph, values) or actual_type(values.get(incoming[0]["from"]), None)


def actual_type(value: Any, declared: str | None) -> str:
    if declared == "code" and isinstance(value, str):
        return "code"
    if isinstance(value, pd.DataFrame):
        return "dataframe"
    if isinstance(value, pd.Series):
        return "series"
    if isinstance(value, (pd.Index, pd.RangeIndex)):
        return "index"
    if isinstance(value, np.ndarray):
        return "ndarray"
    if isinstance(value, dict):
        return "dict"
    if isinstance(value, (list, tuple)):
        return "list"
    return "scalar"


def execute(
    graph: dict[str, Any],
    node: dict[str, Any],
    values: dict[str, Any],
    *,
    mode: str,
    base_dir: Path,
    overrides: dict[str, str],
) -> Any:
    kind = node.get("type")
    params = dict(node.get("params") or {})
    preview = mode != "full"
    node_id = node["id"]

    def wired(port: str) -> Any | None:
        edges = edges_into(graph, node_id, port)
        if not edges:
            return None
        return values.get(edges[0]["from"])

    def require_frame_or_series(port: str = "in") -> Any:
        edges = edges_into(graph, node_id, port)
        if not edges:
            raise NodeError(node_id, f"{kind} needs an input")
        value = values.get(edges[0]["from"])
        if not isinstance(value, (pd.DataFrame, pd.Series)):
            raise NodeError(node_id, f"{kind} expected a dataframe or a series")
        return value

    if kind == "read_csv":
        path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
        return read_csv(node_id, path, preview)
    if kind == "read_excel":
        path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
        sheet = params.get("sheet") or 0
        kwargs: dict[str, Any] = {"sheet_name": sheet}
        if preview:
            kwargs["nrows"] = PREVIEW_ROW_CAP
        try:
            return pd.read_excel(path, **kwargs)
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"read_excel failed: {exc}") from exc
    if kind == "read_sql":
        path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
        table = str(params.get("table") or "")
        if not _IDENT.match(table):
            raise NodeError(node_id, "read_sql needs a table name")
        sql = f'SELECT * FROM "{table}"'
        if preview:
            sql += f" LIMIT {PREVIEW_ROW_CAP}"
        try:
            with sqlite3.connect(path) as con:
                return pd.read_sql(sql, con)
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"read_sql failed: {exc}") from exc
    if kind == "head":
        frame = require_frame_or_series()
        n = wired("n")
        if n is None:
            n = params.get("n", 5)
        try:
            n = int(n)
        except (TypeError, ValueError) as exc:
            raise NodeError(node_id, "n must be an integer") from exc
        return frame.head(n)
    if kind == "change_headers":
        frame = require_frame_or_series()
        if not isinstance(frame, pd.DataFrame):
            actual = actual_type(frame, None)
            raise NodeError(
                node_id,
                f"declared dataframe, pandas returned {actual}",
                declared="dataframe",
                actual=actual,
            )
        mapping = wired("mapping")
        if mapping is None:
            mapping = params.get("mapping") or []
        return apply_headers(node_id, frame, mapping)
    if kind == "query":
        frame = require_frame_or_series()
        if not isinstance(frame, pd.DataFrame):
            raise NodeError(node_id, "query takes a dataframe")
        expr = str(params.get("expr") or "")
        if not expr:
            raise NodeError(node_id, "query needs an expression")
        try:
            return frame.query(expr)
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"query failed: {exc}") from exc
    if kind == "column":
        frame = require_frame_or_series()
        if not isinstance(frame, pd.DataFrame):
            raise NodeError(node_id, "column takes a dataframe")
        name = str(params.get("name") or "")
        if name not in frame.columns:
            raise NodeError(node_id, f"missing column {name}")
        return frame[name]
    if kind == "to_frame":
        series = require_frame_or_series()
        if not isinstance(series, pd.Series):
            raise NodeError(node_id, "to_frame takes a series", declared="series", actual="dataframe")
        return series.to_frame()
    if kind == "sort_values":
        frame = require_frame_or_series()
        ascending = bool(params.get("ascending", True))
        by = str(params.get("by") or "")
        try:
            if isinstance(frame, pd.Series) or not by:
                return frame.sort_values(ascending=ascending)
            if by not in frame.columns:
                raise NodeError(node_id, f"missing column {by}")
            return frame.sort_values(by=by, ascending=ascending)
        except NodeError:
            raise
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"sort failed: {exc}") from exc
    if kind == "merge":
        return merge_frames(node_id, graph, values, params)
    if kind == "concat":
        return concat_frames(node_id, graph, node, values)
    if kind == "literal":
        return literal_value(node_id, params)
    if kind == "python":
        code = wired("code")
        if code is None:
            code = str(params.get("code") or "")
        return run_python(node_id, str(code), graph, node, values)
    if kind == "chart":
        frame = require_frame_or_series()
        return frame
    raise NodeError(node_id, f"unknown feature {kind}")


def resolve_path(
    node_id: str,
    params: dict[str, Any],
    wired_path: Any,
    overrides: dict[str, str],
    base_dir: Path,
) -> Path:
    if node_id in overrides and overrides[node_id]:
        raw = overrides[node_id]
    elif wired_path not in (None, ""):
        raw = str(wired_path)
    else:
        raw = str(params.get("path") or "")
    if not raw:
        raise NodeError(node_id, "no path")
    path = Path(raw)
    if not path.is_absolute():
        path = base_dir / path
    return path


def read_csv(node_id: str, path: Path, preview: bool) -> pd.DataFrame:
    if not path.is_file():
        raise NodeError(node_id, f"file not found: {path}")
    sep = sniff_separator(path)
    kwargs: dict[str, Any] = {"sep": sep}
    if preview:
        kwargs["nrows"] = PREVIEW_ROW_CAP
    try:
        return pd.read_csv(path, **kwargs)
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"read_csv failed: {exc}") from exc


def sniff_separator(path: Path) -> str:
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        line = handle.readline()
    best = ","
    best_count = 0
    for sep in [",", "\t", ";", "|"]:
        count = line.count(sep)
        if count > best_count:
            best_count = count
            best = sep
    return best


def apply_headers(node_id: str, frame: pd.DataFrame, mapping: Any) -> pd.DataFrame:
    columns = [str(col) for col in frame.columns]
    if isinstance(mapping, dict):
        rename = {str(k): "" if v is None else str(v) for k, v in mapping.items()}
        new_names = [rename.get(col, col) or col for col in columns]
    elif isinstance(mapping, list) and mapping and all(isinstance(item, str) for item in mapping):
        if len(mapping) != len(columns):
            raise NodeError(node_id, "the new header list does not match the column count")
        new_names = [item or columns[i] for i, item in enumerate(mapping)]
    else:
        pairs = header_pairs(mapping)
        rename = {src: dst for src, dst in pairs if dst}
        new_names = [rename.get(col, col) for col in columns]
    seen: set[str] = set()
    for name in new_names:
        if name in seen:
            raise NodeError(node_id, f"duplicate header {name}")
        seen.add(name)
    out = frame.copy()
    out.columns = new_names
    return out


def header_pairs(mapping: Any) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    if not isinstance(mapping, list):
        return pairs
    for item in mapping:
        if isinstance(item, dict):
            src = str(item.get("from") or "")
            dst = item.get("to")
            pairs.append((src, "" if dst is None else str(dst)))
        elif isinstance(item, (list, tuple)) and len(item) >= 1:
            src = str(item[0])
            dst = "" if len(item) < 2 or item[1] is None else str(item[1])
            pairs.append((src, dst))
    return pairs


def merge_frames(node_id: str, graph: dict[str, Any], values: dict[str, Any], params: dict[str, Any]) -> pd.DataFrame:
    node = find_node(graph, node_id) or {}
    ports = ["left", "right"]
    for extra in node.get("added_inputs") or []:
        ports.append(extra.get("name"))
    frames = []
    for port in ports:
        edges = edges_into(graph, node_id, port)
        if not edges:
            continue
        value = values.get(edges[0]["from"])
        if not isinstance(value, pd.DataFrame):
            raise NodeError(node_id, f"{port} must be a dataframe")
        frames.append(value)
    if len(frames) < 2:
        raise NodeError(node_id, "merge needs two dataframes")
    how = str(params.get("how") or "inner")
    on = str(params.get("on") or "")
    kwargs: dict[str, Any] = {"how": how}
    if on:
        kwargs["on"] = on
    result = frames[0]
    for frame in frames[1:]:
        try:
            result = result.merge(frame, **kwargs)
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"merge failed: {exc}") from exc
    return result


def concat_frames(node_id: str, graph: dict[str, Any], node: dict[str, Any], values: dict[str, Any]) -> Any:
    frames = []
    ports = ["in"]
    for extra in node.get("added_inputs") or []:
        name = extra.get("name")
        if name and name not in ports:
            ports.append(name)
    for port in ports:
        for edge in edges_into(graph, node_id, port):
            value = values.get(edge["from"])
            if not isinstance(value, (pd.DataFrame, pd.Series)):
                raise NodeError(node_id, "concat takes dataframes or series")
            frames.append(value)
    if not frames:
        raise NodeError(node_id, "concat needs an input")
    try:
        return pd.concat(frames)
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"concat failed: {exc}") from exc


def literal_value(node_id: str, params: dict[str, Any]) -> Any:
    kind = str(params.get("kind") or "scalar")
    raw = params.get("value")
    text = "" if raw is None else str(raw)
    if kind == "code":
        return text
    if kind == "list":
        if text.strip().startswith("["):
            try:
                value = json.loads(text)
            except json.JSONDecodeError as exc:
                raise NodeError(node_id, f"list is not JSON: {exc}") from exc
            if not isinstance(value, list):
                raise NodeError(node_id, "literal kind is list, value is not a list")
            return value
        return [part.strip() for part in text.split(",") if part.strip()]
    if kind == "dict":
        try:
            value = json.loads(text or "{}")
        except json.JSONDecodeError as exc:
            raise NodeError(node_id, f"dict is not JSON: {exc}") from exc
        if not isinstance(value, dict):
            raise NodeError(node_id, "literal kind is dict, value is not a dict")
        return value
    if text == "":
        return ""
    try:
        if "." in text:
            return float(text)
        return int(text)
    except ValueError:
        return text


def run_python(
    node_id: str,
    code: str,
    graph: dict[str, Any],
    node: dict[str, Any],
    values: dict[str, Any],
) -> Any:
    if "def run" not in code:
        raise NodeError(node_id, "code must define run(...)")
    namespace: dict[str, Any] = {"pd": pd, "np": np}
    try:
        exec(code, namespace, namespace)  # noqa: S102 — a .pp code block is the feature
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"code failed to compile: {exc}") from exc
    fn = namespace.get("run")
    if not callable(fn):
        raise NodeError(node_id, "code must define run(...)")
    kwargs = {}
    for port in node.get("inputs") or []:
        name = port.get("name")
        edges = edges_into(graph, node_id, name)
        if not edges:
            raise NodeError(node_id, f"python input {name} is not connected")
        kwargs[name] = values.get(edges[0]["from"])
    try:
        return fn(**kwargs)
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"code failed: {exc}") from exc


def payload_of(value: Any, window: int) -> dict[str, Any]:
    if isinstance(value, pd.DataFrame):
        view = value.iloc[:window]
        return {
            "columns": [str(col) for col in view.columns],
            "dtypes": [str(value.dtypes[col]) for col in value.columns],
            "index": [index_text(item) for item in view.index],
            "rows": [[cell(item) for item in row] for row in view.itertuples(index=False, name=None)],
            "n": int(len(value)),
        }
    if isinstance(value, pd.Series):
        view = value.iloc[:window]
        name = "value" if value.name is None else str(value.name)
        return {
            "columns": [name],
            "dtypes": [str(value.dtype)],
            "index": [index_text(item) for item in view.index],
            "rows": [[cell(item)] for item in view.tolist()],
            "n": int(len(value)),
        }
    if isinstance(value, dict):
        items = list(value.items())
        view = items[:window]
        return {
            "columns": ["key", "value"],
            "dtypes": ["object", "object"],
            "index": [str(i) for i in range(len(view))],
            "rows": [[cell(k), cell(v)] for k, v in view],
            "n": len(items),
        }
    if isinstance(value, (list, tuple)):
        view = list(value)[:window]
        return {
            "columns": ["value"],
            "dtypes": ["object"],
            "index": [str(i) for i in range(len(view))],
            "rows": [[cell(item)] for item in view],
            "n": len(value),
        }
    return {
        "columns": ["value"],
        "dtypes": ["object"],
        "index": ["0"],
        "rows": [[cell(value)]],
        "n": 1,
    }


def empty_payload(value: Any) -> dict[str, Any]:
    n = len(value) if isinstance(value, (pd.DataFrame, pd.Series, list, tuple, dict)) else 1
    return {
        "columns": [],
        "dtypes": [],
        "index": [],
        "rows": [],
        "n": int(n),
    }


def write_csv(value: Any, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(value, pd.DataFrame):
        value.to_csv(path, index=True)
        return
    if isinstance(value, pd.Series):
        value.to_csv(path, index=True)
        return
    frame = pd.DataFrame(payload_of(value, 10**9)["rows"], columns=payload_of(value, 10**9)["columns"])
    frame.to_csv(path, index=False)


def index_text(value: Any) -> str:
    if isinstance(value, (datetime, date, pd.Timestamp)):
        return value.isoformat()
    return str(value)


def cell(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, (float, np.floating)):
        number = float(value)
        if math.isnan(number) or math.isinf(number):
            return None
        return number
    if isinstance(value, (bool, np.bool_)):
        return bool(value)
    if isinstance(value, (int, np.integer)):
        return int(value)
    if isinstance(value, (datetime, date, pd.Timestamp)):
        return value.isoformat()
    if isinstance(value, str):
        return value
    try:
        if pd.isna(value):
            return None
    except (TypeError, ValueError):
        pass
    return str(value)
