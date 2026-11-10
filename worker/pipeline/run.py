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

# Cache keys ever produced per node id, so a reset can find them.
_node_keys: dict[str, set[str]] = {}

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

    from pipeline.scale import clear_fits, take_fit
    from pipeline.vars import VarError, resolve_variables

    clear_fits()
    if op == "reset":
        reset_cache(job.get("nodes") or [])
        return {
            "generation": generation,
            "mode": mode,
            "node": "",
            "columns": [],
            "dtypes": [],
            "index": [],
            "rows": [],
            "n": 0,
            "claim": mode,
            "error": None,
            "computed": [],
            "inactive": [],
        }
    try:
        try:
            variables = resolve_variables(graph, job.get("overrides") or {})
        except VarError as exc:
            raise NodeError(node_id, str(exc)) from exc
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
                variables=variables,
            )
            values[current] = value
            keys[current] = key
        computed = list(order)
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
        if job.get("stats"):
            from pipeline.stats import column_stats

            frame_stats = column_stats(result)
            if frame_stats:
                payload["stats"] = frame_stats
        payload.update(
            {
                "generation": generation,
                "mode": mode,
                "node": node_id,
                "claim": mode,
                "error": None,
                "computed": computed,
                "inactive": [],
            }
        )
        if fit := take_fit(node_id):
            payload["fitted"] = fit
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


def reset_cache(node_ids: list[Any]) -> None:
    """Drops every cache entry the given node ids ever produced."""
    for raw in node_ids:
        node_id = str(raw)
        for key in _node_keys.pop(node_id, set()):
            _cache.pop(key, None)


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
    variables: dict[str, Any] | None = None,
) -> tuple[Any, str]:
    from pipeline.vars import VarError, substitute_node

    node = find_node(graph, node_id)
    if node is None:
        raise NodeError(node_id, f"unknown node {node_id}")
    if node.get("unknown"):
        raise NodeError(node_id, f"unknown feature {node.get('type')}")
    if variables is None:
        variables = {}
    # Params carry ${name} references. The substituted values are what the
    # cache key hashes, so an override never returns a stale cached result.
    try:
        node, var_origins = substitute_node(node, variables)
    except VarError as exc:
        raise NodeError(node_id, str(exc)) from exc
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
        "fitted": node.get("fitted"),
        "parents": parent_keys,
        "mode": mode,
        "override": overrides.get(node_id),
    }
    key = hashlib.sha256(json.dumps(record, sort_keys=True, default=str).encode()).hexdigest()
    _node_keys.setdefault(node_id, set()).add(key)
    if key in _cache:
        return _cache[key], key
    stats["computed"] += 1
    value = execute(
        graph,
        node,
        values,
        mode=mode,
        base_dir=base_dir,
        overrides=overrides,
        variables=variables,
        var_origins=var_origins,
    )
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
    if kind in (
        "read_csv",
        "read_excel",
        "read_sql",
        "read_json",
        "read_parquet",
        "read_table",
        "read_fwf",
        "read_html",
        "read_xml",
        "json_normalize",
        "resample",
        "asfreq",
        "get_dummies",
        "wide_to_long",
        "combine_first",
        "standard_scale",
        "minmax_scale",
        "robust_scale",
        "ols",
        "calculate",
        "change_headers",
        "query",
        "to_frame",
        "merge",
        "drop",
        "filter",
        "assign",
        "melt",
        "pivot",
        "groupby",
        "describe",
        "set_index",
        "nlargest",
        "nsmallest",
        "astype",
        "preview_table",
        "chart",
        "explode",
        "transpose",
        "unstack",
        "crosstab",
        "join",
        "merge_asof",
    ):
        return "dataframe"
    if kind == "str" and str((node.get("params") or {}).get("method") or "") == "extract":
        return "dataframe"
    if kind == "date_range":
        return "index"
    if kind in ("apply", "map") and str((node.get("params") or {}).get("fn") or "") != "code":
        return "series"
    if kind in (
        "value_counts",
        "sum",
        "mean",
        "min",
        "max",
        "count",
        "median",
        "std",
        "nunique",
        "quantile",
        "stack",
        "unique",
    ):
        return "series"
    if kind in ("corr", "cov", "map", "apply"):
        return input_type(node, graph, values)
    if kind == "column":
        return "series"
    if kind == "literal":
        return str((node.get("params") or {}).get("kind") or "scalar")
    if kind == "python":
        output = node.get("output") or {}
        accepts = output.get("accepts") or ["dataframe"]
        return accepts[0]
    if kind in (
        "head",
        "tail",
        "sample",
        "sort_values",
        "concat",
        "dropna",
        "fillna",
        "replace",
        "drop_duplicates",
        "reset_index",
        "abs",
        "round",
        "shift",
        "ffill",
        "bfill",
        "interpolate",
        "clip",
        "where",
        "isin",
        "between",
        "loc",
        "iloc",
        "rolling",
        "diff",
        "pct_change",
        "cumsum",
        "str",
        "dt",
        "to_datetime",
        "to_numeric",
        "to_csv",
        "to_excel",
        "to_json",
        "to_parquet",
        "to_sql",
    ):
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
    variables: dict[str, Any] | None = None,
    var_origins: dict[str, str] | None = None,
) -> Any:
    kind = node.get("type")
    params = dict(node.get("params") or {})
    preview = mode != "full"
    node_id = node["id"]
    variables = variables or {}
    var_origins = var_origins or {}

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
        return read_csv(node_id, path, preview, params)
    if kind == "read_excel":
        path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
        sheet = params.get("sheet") or 0
        kwargs: dict[str, Any] = {"sheet_name": sheet}
        kwargs.update(reader_kwargs(params, preview))
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
        skip = params.get("skip_rows")
        try:
            offset = 0 if skip in (None, "") else int(skip)
        except (TypeError, ValueError):
            offset = 0
        if preview:
            sql += f" LIMIT {PREVIEW_ROW_CAP} OFFSET {offset}"
        elif offset:
            sql += f" LIMIT -1 OFFSET {offset}"
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
            origin = var_origins.get("n")
            if origin:
                raise NodeError(node_id, f"variable ${{{origin}}} must be an int") from exc
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
        return run_python(node_id, str(code), graph, node, values, variables=variables)
    if kind in ("chart", "preview_table"):
        return require_frame_or_series()
    from pipeline.batch import run_batch

    batched = run_batch(kind, node_id, params, require_frame_or_series, base_dir)
    if batched is not _MISSING:
        return batched
    from pipeline.more import run_more

    if kind in ("standard_scale", "minmax_scale", "robust_scale", "ols", "calculate"):
        from pipeline.scale import run_scale

        return run_scale(kind, node_id, node, params, require_frame_or_series(), wired("fit"), mode)
    extra_more = run_more(
        kind,
        node_id,
        params,
        base_dir,
        preview,
        require_frame_or_series,
        wired,
        overrides,
    )
    if extra_more is not _MISSING:
        return extra_more
    extra = run_extra(kind, node_id, graph, params, wired, require_frame_or_series, preview, base_dir, overrides)
    if extra is not _MISSING:
        return extra
    raise NodeError(node_id, f"unknown feature {kind}")


_MISSING = object()


def _columns(text: Any) -> list[str]:
    raw = "" if text is None else str(text)
    return [part.strip() for part in raw.split(",") if part.strip()]


def _int_param(node_id: str, value: Any, default: int) -> int:
    if value is None or value == "":
        return default
    try:
        return int(value)
    except (TypeError, ValueError) as exc:
        raise NodeError(node_id, "expected an integer") from exc


def _groupby_how(node_id: str, frame: pd.DataFrame, by: str, agg: str, how: str, params: dict[str, Any]):
    column = str(params.get("column") or "")
    grouped = frame.groupby(by)
    q = float(params.get("q") or 0.5)

    def statistic(values: pd.Series):
        if agg == "quantile":
            return values.quantile(q)
        if agg not in ("sum", "mean", "min", "max", "count", "median", "std", "nunique"):
            raise NodeError(node_id, f"unknown aggregation {agg}")
        return getattr(values, agg)()

    if how == "transform":
        out = frame.copy()
        if column:
            if column not in out.columns:
                raise NodeError(node_id, f"missing column {column}")
            if agg == "quantile":
                out[column] = grouped[column].transform(lambda values: values.quantile(q))
            else:
                out[column] = grouped[column].transform(agg)
            return out
        numeric = out.select_dtypes(include="number").columns
        if agg == "quantile":
            out[numeric] = grouped[numeric].transform(lambda values: values.quantile(q))
        else:
            out[numeric] = grouped[numeric].transform(agg)
        return out
    if not column:
        raise NodeError(node_id, "filter needs a column")
    if column not in frame.columns:
        raise NodeError(node_id, f"missing column {column}")
    op = str(params.get("filter_op") or "gt")
    threshold = float(params.get("filter_value") or 0)
    comparisons = {
        "gt": lambda left, right: left > right,
        "ge": lambda left, right: left >= right,
        "lt": lambda left, right: left < right,
        "le": lambda left, right: left <= right,
        "eq": lambda left, right: left == right,
    }
    if op not in comparisons:
        raise NodeError(node_id, f"unknown comparison {op}")

    def keep(group: pd.DataFrame) -> bool:
        return bool(comparisons[op](statistic(group[column]), threshold))

    return grouped.filter(keep)


def run_extra(kind, node_id, graph, params, wired, require_frame_or_series, preview, base_dir, overrides):
    if kind == "read_json":
        path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
        try:
            frame = pd.read_json(path)
        except Exception as exc:  # noqa: BLE001
            raise NodeError(node_id, f"read_json failed: {exc}") from exc
        if preview:
            return frame.head(PREVIEW_ROW_CAP)
        return frame
    if kind not in {
        "tail",
        "sample",
        "drop",
        "filter",
        "drop_duplicates",
        "nlargest",
        "nsmallest",
        "dropna",
        "fillna",
        "replace",
        "astype",
        "reset_index",
        "set_index",
        "assign",
        "melt",
        "pivot",
        "groupby",
        "describe",
        "value_counts",
        "sum",
        "mean",
        "min",
        "max",
        "count",
        "median",
        "std",
        "nunique",
        "quantile",
        "abs",
        "round",
    }:
        return _MISSING
    frame = require_frame_or_series()
    try:
        if kind == "tail":
            return frame.tail(_int_param(node_id, params.get("n"), 5))
        if kind == "sample":
            n = _int_param(node_id, params.get("n"), 5)
            return frame.sample(n=min(n, len(frame)), random_state=0)
        if kind == "drop":
            cols = _columns(params.get("columns"))
            missing = [col for col in cols if col not in frame.columns]
            if missing:
                raise NodeError(node_id, f"missing column {missing[0]}")
            return frame.drop(columns=cols)
        if kind == "filter":
            cols = _columns(params.get("columns"))
            missing = [col for col in cols if col not in frame.columns]
            if missing:
                raise NodeError(node_id, f"missing column {missing[0]}")
            return frame.loc[:, cols]
        if kind == "drop_duplicates":
            column = str(params.get("column") or "")
            if column:
                return frame.drop_duplicates(subset=[column])
            return frame.drop_duplicates()
        if kind in ("nlargest", "nsmallest"):
            column = str(params.get("column") or "")
            if column not in getattr(frame, "columns", []):
                raise NodeError(node_id, f"missing column {column}")
            n = _int_param(node_id, params.get("n"), 5)
            method = frame.nlargest if kind == "nlargest" else frame.nsmallest
            return method(n, column)
        if kind == "dropna":
            return frame.dropna()
        if kind == "fillna":
            return frame.fillna(params.get("value") if params.get("value") not in (None, "") else 0)
        if kind == "replace":
            return frame.replace(params.get("old"), params.get("new"))
        if kind == "astype":
            column = str(params.get("column") or "")
            dtype = str(params.get("dtype") or "float")
            mapping = {"float": "float64", "int": "int64", "string": "string", "bool": "bool"}
            if column not in frame.columns:
                raise NodeError(node_id, f"missing column {column}")
            out = frame.copy()
            out[column] = out[column].astype(mapping.get(dtype, dtype))
            return out
        if kind == "reset_index":
            return frame.reset_index(drop=False)
        if kind == "set_index":
            column = str(params.get("column") or "")
            if column not in frame.columns:
                raise NodeError(node_id, f"missing column {column}")
            return frame.set_index(column)
        if kind == "assign":
            name = str(params.get("name") or "")
            expr = str(params.get("expr") or "")
            if not name or not expr:
                raise NodeError(node_id, "assign needs a name and an expression")
            out = frame.copy()
            out[name] = out.eval(expr)
            return out
        if kind == "melt":
            ids = _columns(params.get("id_vars"))
            return frame.melt(id_vars=ids or None)
        if kind == "pivot":
            index = str(params.get("index") or "")
            columns = str(params.get("columns") or "")
            values = str(params.get("values") or "")
            agg = str(params.get("agg") or "sum")
            if not index:
                raise NodeError(node_id, "pivot needs an index column")
            kwargs = {"index": index, "aggfunc": agg}
            if columns:
                kwargs["columns"] = columns
            if values:
                kwargs["values"] = values
            table = frame.pivot_table(**kwargs)
            table = table.reset_index()
            if isinstance(table.columns, pd.MultiIndex):
                table.columns = [" ".join(str(part) for part in col if str(part) != "").strip() for col in table.columns]
            return table
        if kind == "groupby":
            by = str(params.get("by") or "")
            agg = str(params.get("agg") or "sum")
            aggs = str(params.get("aggs") or "").strip()
            how = str(params.get("how") or "agg")
            if by not in frame.columns:
                raise NodeError(node_id, f"missing column {by}")
            if how in ("transform", "filter"):
                return _groupby_how(node_id, frame, by, agg, how, params)
            grouped = frame.groupby(by, as_index=False)
            if aggs:
                spec: dict[str, list[str]] = {}
                for piece in aggs.split(","):
                    piece = piece.strip()
                    if ":" not in piece:
                        raise NodeError(node_id, "aggregations use column:function")
                    col, fn = (part.strip() for part in piece.split(":", 1))
                    if col not in frame.columns:
                        raise NodeError(node_id, f"missing column {col}")
                    spec.setdefault(col, []).append(fn)
                table = grouped.agg(spec)
                if isinstance(table.columns, pd.MultiIndex):
                    table.columns = [
                        " ".join(str(part) for part in col if str(part) not in ("", "nan")).strip()
                        for col in table.columns
                    ]
                return table
            if agg == "count":
                return grouped.count()
            if agg == "mean":
                return grouped.mean(numeric_only=True)
            if agg == "min":
                return grouped.min(numeric_only=True)
            if agg == "max":
                return grouped.max(numeric_only=True)
            if agg == "median":
                return grouped.median(numeric_only=True)
            if agg == "std":
                return grouped.std(numeric_only=True)
            if agg == "nunique":
                return grouped.nunique()
            if agg == "quantile":
                return grouped.quantile(float(params.get("q") or 0.5), numeric_only=True)
            return grouped.sum(numeric_only=True)
        if kind == "describe":
            return frame.describe()
        if kind == "value_counts":
            column = str(params.get("column") or "")
            if column not in frame.columns:
                raise NodeError(node_id, f"missing column {column}")
            counts = frame[column].value_counts()
            return counts.rename("count")
        if kind == "quantile":
            q = float(params.get("q") or 0.5)
            if isinstance(frame, pd.Series):
                return pd.Series([frame.quantile(q)], name=frame.name or "quantile")
            return frame.quantile(q)
        if kind in ("sum", "mean", "min", "max", "count", "median", "std", "nunique"):
            if isinstance(frame, pd.Series):
                value = getattr(frame, kind)()
                return pd.Series([value], name=frame.name or kind)
            if kind == "count":
                return frame.count()
            if kind == "nunique":
                return frame.nunique()
            numeric = frame.select_dtypes(include="number")
            return getattr(numeric, kind)(numeric_only=True)
        if kind == "abs":
            if isinstance(frame, pd.Series):
                return frame.abs()
            out = frame.copy()
            numeric = out.select_dtypes(include="number").columns
            out[numeric] = out[numeric].abs()
            return out
        if kind == "round":
            decimals = _int_param(node_id, params.get("decimals"), 2)
            return frame.round(decimals)
    except NodeError:
        raise
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"{kind} failed: {exc}") from exc
    return _MISSING


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


def reader_kwargs(params: dict[str, Any], preview: bool) -> dict[str, Any]:
    skip = params.get("skip_rows")
    try:
        skip_rows = 0 if skip in (None, "") else int(skip)
    except (TypeError, ValueError):
        skip_rows = 0
    header_row = params.get("header_row")
    try:
        header_index = 0 if header_row in (None, "") else int(header_row)
    except (TypeError, ValueError):
        header_index = 0
    # Skip rows first. Header row counts what remains, so a preamble and a
    # title row are not both applied as absolute file indexes.
    if str(params.get("header") or "header row") == "no header":
        kwargs: dict[str, Any] = {"skiprows": skip_rows, "header": None}
    else:
        kwargs = {"header": skip_rows + header_index}
    if preview:
        kwargs["nrows"] = PREVIEW_ROW_CAP
    return kwargs


def read_csv(node_id: str, path: Path, preview: bool, params: dict[str, Any] | None = None) -> pd.DataFrame:
    if not path.is_file():
        raise NodeError(node_id, f"file not found: {path}")
    sep = sniff_separator(path)
    kwargs = reader_kwargs(params or {}, preview)
    kwargs["sep"] = sep
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
    *,
    variables: dict[str, Any] | None = None,
) -> Any:
    if "def run" not in code:
        raise NodeError(node_id, "code must define run(...)")
    namespace: dict[str, Any] = {"pd": pd, "np": np, "vars": dict(variables or {})}
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
    if isinstance(value, pd.Index):
        return payload_of(pd.Series(value, name=value.name or "value"), window)
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
