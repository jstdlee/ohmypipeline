"""Time, encoding, arithmetic, index, and the extra readers."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pandas as pd

_HANDLED = {
    "resample",
    "asfreq",
    "expanding",
    "ewm",
    "date_range",
    "tz_localize",
    "tz_convert",
    "to_timedelta",
    "cut",
    "qcut",
    "get_dummies",
    "factorize",
    "arith",
    "apply",
    "map",
    "rank",
    "corr",
    "cov",
    "unique",
    "reindex",
    "sort_index",
    "wide_to_long",
    "combine_first",
    "quantile",
    "read_parquet",
    "read_table",
    "read_fwf",
    "read_html",
    "read_xml",
    "json_normalize",
}

_REDUCERS = {"sum", "mean", "min", "max", "count", "median", "std", "nunique", "quantile"}
_ARITH = {
    "add": "add",
    "sub": "sub",
    "mul": "mul",
    "div": "div",
    "floordiv": "floordiv",
    "mod": "mod",
    "pow": "pow",
    "eq": "eq",
    "ne": "ne",
    "lt": "lt",
    "gt": "gt",
    "le": "le",
    "ge": "ge",
}


def run_more(kind, node_id, params, base_dir, preview, require, wired, overrides):
    from pipeline.run import NodeError, _MISSING

    if kind not in _HANDLED:
        return _MISSING
    try:
        if kind.startswith("read_") or kind == "json_normalize":
            return _read(kind, node_id, params, base_dir, preview, wired, overrides)
        if kind == "date_range":
            return _date_range(node_id, params)
        if kind == "combine_first":
            left = require("left")
            right = require("right")
            if not isinstance(left, pd.DataFrame) or not isinstance(right, pd.DataFrame):
                raise NodeError(node_id, "combine_first needs two dataframes")
            return left.combine_first(right)
        if kind == "arith":
            return _arith(node_id, require(), params, wired("other"))
        frame = require()
        if kind == "resample":
            return _resample(node_id, frame, params)
        if kind == "asfreq":
            return _asfreq(node_id, frame, params)
        if kind == "expanding":
            return _windowed(node_id, frame, params, expanding=True)
        if kind == "ewm":
            return _ewm(node_id, frame, params)
        if kind in ("tz_localize", "tz_convert"):
            return _timezone(node_id, frame, params, kind)
        if kind == "to_timedelta":
            return _timedelta(node_id, frame, params)
        if kind == "cut":
            return _cut(node_id, frame, params, quantiles=False)
        if kind == "qcut":
            return _cut(node_id, frame, params, quantiles=True)
        if kind == "get_dummies":
            return _dummies(node_id, frame, params)
        if kind == "factorize":
            return _factorize(node_id, frame, params)
        if kind in ("apply", "map"):
            return _apply_or_map(node_id, frame, params, kind)
        if kind == "rank":
            return _rank(frame, params)
        if kind in ("corr", "cov"):
            return _corr(node_id, frame, params, kind, wired("other"))
        if kind == "unique":
            return _unique(node_id, frame, params)
        if kind == "reindex":
            return _reindex(node_id, frame, params)
        if kind == "sort_index":
            ascending = str(params.get("order") or "ascending") != "descending"
            axis = 1 if str(params.get("axis") or "0") == "1" else 0
            return frame.sort_index(ascending=ascending, axis=axis)
        if kind == "wide_to_long":
            return _wide(node_id, frame, params)
        if kind == "quantile":
            return _quantile_value(frame, params)
    except NodeError:
        raise
    except Exception as exc:  # noqa: BLE001
        from pipeline.run import NodeError

        raise NodeError(node_id, f"{kind} failed: {exc}") from exc
    return _MISSING


def _read(kind, node_id, params, base_dir, preview, wired, overrides):
    from pipeline.run import PREVIEW_ROW_CAP, NodeError, reader_kwargs, resolve_path

    if kind == "json_normalize":
        raw = _json_source(node_id, params, base_dir, wired, overrides)
        record_path = str(params.get("record_path") or "").strip()
        meta = [part.strip() for part in str(params.get("meta") or "").split(",") if part.strip()]
        kwargs: dict[str, Any] = {"sep": str(params.get("sep") or ".") or "."}
        if record_path:
            kwargs["record_path"] = record_path.split(".")
        if meta:
            kwargs["meta"] = meta
        frame = pd.json_normalize(raw, **kwargs)
        if preview:
            return frame.head(PREVIEW_ROW_CAP)
        return frame

    path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
    if not path.is_file():
        raise NodeError(node_id, f"file not found: {path}")
    if kind == "read_parquet":
        return _read_parquet(path, preview)
    if kind == "read_table":
        kwargs = reader_kwargs(params, preview)
        kwargs["sep"] = str(params.get("sep") or "\t") or "\t"
        return pd.read_table(path, **kwargs)
    if kind == "read_fwf":
        kwargs = reader_kwargs(params, preview)
        specs = _colspecs(str(params.get("colspecs") or ""))
        if specs:
            kwargs["colspecs"] = specs
        return pd.read_fwf(path, **kwargs)
    if kind == "read_html":
        kwargs = reader_kwargs(params, False)
        tables = pd.read_html(path, **kwargs)
        which = int(params.get("which") or 0)
        if which < 0 or which >= len(tables):
            raise NodeError(node_id, f"html table {which} is not in the file")
        frame = tables[which]
        if preview:
            return frame.head(PREVIEW_ROW_CAP)
        return frame
    if kind == "read_xml":
        xpath = str(params.get("xpath") or "").strip()
        kwargs = {"xpath": xpath} if xpath else {}
        frame = pd.read_xml(path, **kwargs)
        skip = int(params.get("skip_rows") or 0)
        if skip:
            frame = frame.iloc[skip:]
        if preview:
            return frame.head(PREVIEW_ROW_CAP)
        return frame
    raise NodeError(node_id, f"unknown reader {kind}")


def _read_parquet(path: Path, preview: bool):
    from pipeline.run import PREVIEW_ROW_CAP

    import pyarrow.parquet as pq

    if not preview:
        return pd.read_parquet(path)
    file = pq.ParquetFile(path)
    batches = []
    seen = 0
    for batch in file.iter_batches(batch_size=PREVIEW_ROW_CAP):
        batches.append(batch)
        seen += batch.num_rows
        if seen >= PREVIEW_ROW_CAP:
            break
    if not batches:
        return pd.read_parquet(path).head(0)
    import pyarrow as pa

    table = pa.Table.from_batches(batches).slice(0, PREVIEW_ROW_CAP)
    return table.to_pandas()


def _json_source(node_id, params, base_dir, wired, overrides):
    from pipeline.run import NodeError, resolve_path

    wired_value = wired("data")
    if isinstance(wired_value, (list, dict)):
        return wired_value
    path_text = str(params.get("path") or "").strip()
    if not path_text and not wired("path"):
        raise NodeError(node_id, "json_normalize needs a file or a list")
    path = resolve_path(node_id, params, wired("path"), overrides, base_dir)
    if not path.is_file():
        raise NodeError(node_id, f"file not found: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def _date_range(node_id, params):
    from pipeline.run import NodeError

    start = str(params.get("start") or "")
    end = str(params.get("end") or "")
    freq = str(params.get("freq") or "D")
    periods = params.get("periods")
    if not start:
        raise NodeError(node_id, "date_range needs a start")
    kwargs: dict[str, Any] = {"start": start, "freq": freq}
    if periods not in (None, "", 0, "0"):
        kwargs["periods"] = int(periods)
    elif end:
        kwargs["end"] = end
    else:
        raise NodeError(node_id, "date_range needs an end or a period count")
    return pd.date_range(**kwargs)


def _resample(node_id, frame, params):
    from pipeline.run import NodeError

    freq = str(params.get("freq") or "")
    if not freq:
        raise NodeError(node_id, "resample needs a frequency")
    indexed = _time_indexed(node_id, frame, str(params.get("column") or ""))
    agg = str(params.get("agg") or "mean")
    sampled = _reduce_resample(indexed, freq, agg, params)
    fill = str(params.get("fill") or "none")
    if fill == "ffill":
        sampled = sampled.ffill()
    elif fill == "bfill":
        sampled = sampled.bfill()
    return sampled.reset_index()


def _asfreq(node_id, frame, params):
    from pipeline.run import NodeError

    freq = str(params.get("freq") or "")
    if not freq:
        raise NodeError(node_id, "asfreq needs a frequency")
    indexed = _time_indexed(node_id, frame, str(params.get("column") or ""))
    method = str(params.get("fill") or "none")
    kwargs = {}
    if method in ("ffill", "bfill"):
        kwargs["method"] = "pad" if method == "ffill" else "backfill"
    return indexed.asfreq(freq, **kwargs).reset_index()


def _reduce_resample(indexed, freq, agg, params):
    resampler = indexed.resample(freq)
    if agg == "quantile":
        return resampler.quantile(float(params.get("q") or 0.5))
    if agg == "count":
        return resampler.count()
    if agg == "nunique":
        return resampler.nunique()
    if agg in ("sum", "mean", "min", "max", "median", "std"):
        numeric = indexed.select_dtypes(include="number") if isinstance(indexed, pd.DataFrame) else indexed
        return getattr(numeric.resample(freq), agg)()
    raise ValueError(f"unknown aggregation {agg}")


def _windowed(node_id, frame, params, expanding: bool):
    from pipeline.run import NodeError

    agg = str(params.get("agg") or "mean")
    minimum = int(params.get("window") or 1)
    if minimum < 1:
        raise NodeError(node_id, "window must be at least 1")
    q = float(params.get("q") or 0.5)

    def apply_window(values: pd.Series) -> pd.Series:
        window = values.expanding(min_periods=minimum)
        if agg == "quantile":
            return window.quantile(q)
        if agg not in _REDUCERS:
            raise NodeError(node_id, f"unknown aggregation {agg}")
        return getattr(window, agg)()

    return _on_numeric(node_id, frame, params, apply_window)


def _ewm(node_id, frame, params):
    from pipeline.run import NodeError

    span = int(params.get("span") or 3)
    if span < 1:
        raise NodeError(node_id, "span must be at least 1")
    agg = str(params.get("agg") or "mean")
    if agg not in ("mean", "std"):
        raise NodeError(node_id, "ewm supports mean and std")

    def apply_ewm(values: pd.Series) -> pd.Series:
        return getattr(values.ewm(span=span, adjust=False), agg)()

    return _on_numeric(node_id, frame, params, apply_ewm)


def _timezone(node_id, frame, params, kind):
    from pipeline.run import NodeError

    tz = str(params.get("tz") or "")
    if not tz:
        raise NodeError(node_id, f"{kind} needs a time zone")
    series, column = _one_series(node_id, frame, params, kind)
    dates = pd.to_datetime(series, errors="coerce")
    if kind == "tz_localize":
        result = dates.dt.tz_localize(tz)
    else:
        if dates.dt.tz is None:
            raise NodeError(node_id, "tz_convert needs timezone-aware values")
        result = dates.dt.tz_convert(tz)
    return _put(frame, column, result)


def _timedelta(node_id, frame, params):
    series, column = _one_series(node_id, frame, params, "to_timedelta")
    unit = str(params.get("unit") or "").strip()
    kwargs = {"errors": "coerce"}
    if unit:
        kwargs["unit"] = unit
    return _put(frame, column, pd.to_timedelta(series, **kwargs))


def _cut(node_id, frame, params, quantiles: bool):
    from pipeline.run import NodeError

    series, column = _one_series(node_id, frame, params, "qcut" if quantiles else "cut")
    labels = [part.strip() for part in str(params.get("labels") or "").split(",") if part.strip()] or None
    if quantiles:
        q = params.get("q") or params.get("bins") or ""
        if q in ("", None):
            raise NodeError(node_id, "qcut needs a quantile count")
        result = pd.qcut(series, q=int(q), labels=labels, duplicates="drop")
    else:
        text = str(params.get("bins") or "").strip()
        if not text:
            raise NodeError(node_id, "cut needs bins")
        bins: int | list[float]
        if "," in text:
            bins = [float(part) for part in text.split(",") if part.strip()]
        else:
            bins = int(text)
        result = pd.cut(series, bins=bins, labels=labels)
    return _put(frame, column, result.astype(str))


def _dummies(node_id, frame, params):
    from pipeline.run import NodeError

    if not isinstance(frame, pd.DataFrame):
        frame = frame.to_frame()
    column = str(params.get("column") or "")
    prefix = str(params.get("prefix") or "") or None
    if column:
        if column not in frame.columns:
            raise NodeError(node_id, f"missing column {column}")
        dummies = pd.get_dummies(frame[column], prefix=prefix or column)
        kept = frame.drop(columns=[column])
        return pd.concat([kept, dummies], axis=1)
    return pd.get_dummies(frame, prefix=prefix)


def _factorize(node_id, frame, params):
    series, column = _one_series(node_id, frame, params, "factorize")
    codes, _uniques = pd.factorize(series, use_na_sentinel=True)
    coded = pd.Series(codes, index=series.index, name=series.name)
    return _put(frame, column, coded)


def _arith(node_id, frame, params, other):
    from pipeline.run import NodeError

    op = str(params.get("op") or "add")
    if op not in _ARITH:
        raise NodeError(node_id, f"unknown arithmetic {op}")
    if other is None:
        text = str(params.get("value") or "").strip()
        if text == "":
            raise NodeError(node_id, "arith needs a value or a connected other input")
        other = _number_or_text(text)
    method = getattr(frame, _ARITH[op])
    return method(other)


def _apply_or_map(node_id, frame, params, kind):
    from pipeline.run import NodeError

    fn = str(params.get("fn") or "sum")
    if fn == "code":
        code = str(params.get("code") or "")
        if "def run" not in code:
            raise NodeError(node_id, "code must define run(...)")
        namespace: dict[str, Any] = {"pd": pd}
        exec(code, namespace, namespace)  # noqa: S102 — the block is the feature
        function = namespace.get("run")
        if not callable(function):
            raise NodeError(node_id, "code must define run(...)")
        if kind == "map":
            return frame.map(function)
        axis = 1 if str(params.get("axis") or "0") == "1" else 0
        if isinstance(frame, pd.Series):
            return frame.apply(function)
        return frame.apply(function, axis=axis)
    if fn not in _REDUCERS and fn != "quantile":
        raise NodeError(node_id, f"unknown reducer {fn}")
    if isinstance(frame, pd.Series):
        if fn == "quantile":
            return pd.Series([frame.quantile(float(params.get("q") or 0.5))], name=frame.name or fn)
        value = getattr(frame, fn)()
        return value if isinstance(value, pd.Series) else pd.Series([value], name=frame.name or fn)
    axis = 1 if str(params.get("axis") or "0") == "1" else 0
    if fn == "quantile":
        return frame.quantile(float(params.get("q") or 0.5), axis=axis)
    return frame.apply(fn, axis=axis)


def _rank(frame, params):
    method = str(params.get("method") or "average")
    column = str(params.get("column") or "")
    if isinstance(frame, pd.Series) or not column:
        if isinstance(frame, pd.DataFrame):
            out = frame.copy()
            numeric = out.select_dtypes(include="number").columns
            out[numeric] = out[numeric].rank(method=method)
            return out
        return frame.rank(method=method)
    out = frame.copy()
    out[column] = out[column].rank(method=method)
    return out


def _corr(node_id, frame, params, kind, other):
    from pipeline.run import NodeError

    if isinstance(frame, pd.Series):
        if not isinstance(other, pd.Series):
            raise NodeError(node_id, f"{kind} of a series needs another series")
        value = frame.corr(other) if kind == "corr" else frame.cov(other)
        return pd.Series([value], name=kind)
    numeric = frame.select_dtypes(include="number")
    if numeric.shape[1] == 0:
        raise NodeError(node_id, f"{kind} needs numeric columns")
    if kind == "corr":
        return numeric.corr()
    return numeric.cov()


def _unique(node_id, frame, params):
    series, _column = _one_series(node_id, frame, params, "unique")
    return pd.Series(pd.unique(series), name=series.name or "unique")


def _reindex(node_id, frame, params):
    from pipeline.run import NodeError

    index = _labels(str(params.get("index") or ""))
    columns = _labels(str(params.get("columns") or ""))
    if not index and not columns:
        raise NodeError(node_id, "reindex needs index labels or column labels")
    kwargs: dict[str, Any] = {}
    if index:
        kwargs["index"] = [_maybe_int(label, frame.index) for label in index]
    if columns and isinstance(frame, pd.DataFrame):
        kwargs["columns"] = columns
    fill = params.get("fill")
    if fill not in (None, ""):
        kwargs["fill_value"] = _number_or_text(str(fill))
    return frame.reindex(**kwargs)


def _wide(node_id, frame, params):
    from pipeline.run import NodeError

    if not isinstance(frame, pd.DataFrame):
        raise NodeError(node_id, "wide_to_long takes a dataframe")
    stubs = _labels(str(params.get("stubnames") or ""))
    i = str(params.get("i") or "")
    j = str(params.get("j") or "j")
    if not stubs or not i:
        raise NodeError(node_id, "wide_to_long needs stub names and an id column")
    table = pd.wide_to_long(frame, stubnames=stubs, i=i, j=j, sep=str(params.get("sep") or ""))
    return table.reset_index()


def _quantile_value(frame, params):
    q = float(params.get("q") or 0.5)
    if isinstance(frame, pd.Series):
        return pd.Series([frame.quantile(q)], name=frame.name or "quantile")
    numeric = frame.select_dtypes(include="number")
    return numeric.quantile(q)


def _time_indexed(node_id, frame, column: str):
    from pipeline.run import NodeError

    if isinstance(frame, pd.Series):
        out = frame.copy()
        out.index = pd.to_datetime(out.index)
        return out
    if column:
        if column not in frame.columns:
            raise NodeError(node_id, f"missing column {column}")
        out = frame.copy()
        out.index = pd.to_datetime(out[column])
        return out.drop(columns=[column])
    out = frame.copy()
    out.index = pd.to_datetime(out.index)
    return out


def _one_series(node_id, frame, params, kind):
    from pipeline.run import NodeError

    if isinstance(frame, pd.Series):
        return frame, None
    if not isinstance(frame, pd.DataFrame):
        raise NodeError(node_id, f"{kind} expected a dataframe or a series")
    column = str(params.get("column") or "")
    if not column:
        raise NodeError(node_id, f"{kind} needs a column")
    if column not in frame.columns:
        raise NodeError(node_id, f"missing column {column}")
    return frame[column], column


def _put(frame, column, result):
    if column is None or not isinstance(frame, pd.DataFrame):
        return result
    out = frame.copy()
    out[column] = result
    return out


def _on_numeric(node_id, frame, params, fn):
    column = str(params.get("column") or "")
    if isinstance(frame, pd.Series):
        return fn(frame)
    if column:
        if column not in frame.columns:
            from pipeline.run import NodeError

            raise NodeError(node_id, f"missing column {column}")
        out = frame.copy()
        out[column] = fn(out[column])
        return out
    out = frame.copy()
    numeric = out.select_dtypes(include="number").columns
    out[numeric] = fn(out[numeric])
    return out


def _labels(text: str) -> list[str]:
    return [part.strip() for part in text.split(",") if part.strip()]


def _colspecs(text: str):
    if not text.strip():
        return None
    specs = []
    for part in text.split(","):
        start, end = part.split(":")
        specs.append((int(start), None if end.strip() == "" else int(end)))
    return specs


def _maybe_int(label: str, index) -> Any:
    if label.lstrip("-").isdigit() and pd.api.types.is_integer_dtype(index):
        return int(label)
    return label


def _number_or_text(text: str) -> Any:
    if text.lstrip("-").isdigit():
        return int(text)
    try:
        return float(text)
    except ValueError:
        return text
