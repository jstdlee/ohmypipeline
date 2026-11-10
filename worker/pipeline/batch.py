"""Text, dates, writers, selection, cleaning, reshape, and windows."""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import pandas as pd

_HANDLED = {
    "str",
    "to_datetime",
    "to_numeric",
    "dt",
    "shift",
    "to_csv",
    "to_excel",
    "to_json",
    "to_parquet",
    "to_sql",
    "isin",
    "between",
    "loc",
    "iloc",
    "ffill",
    "bfill",
    "interpolate",
    "clip",
    "where",
    "explode",
    "transpose",
    "stack",
    "unstack",
    "crosstab",
    "rolling",
    "diff",
    "pct_change",
    "cumsum",
    "join",
    "merge_asof",
}

_STR_PLAIN = {
    "lower",
    "upper",
    "title",
    "strip",
    "lstrip",
    "rstrip",
    "len",
    "isalnum",
    "isalpha",
    "isdigit",
    "isnumeric",
}


def run_batch(kind: str, node_id: str, params: dict[str, Any], fetch, base_dir: Path):
    from pipeline.run import NodeError, _MISSING

    if kind not in _HANDLED:
        return _MISSING
    try:
        if kind in ("to_csv", "to_excel", "to_json", "to_parquet", "to_sql"):
            return _write(kind, node_id, fetch(), params, base_dir)
        if kind == "join":
            return _join(node_id, fetch, params)
        if kind == "merge_asof":
            return _merge_asof(node_id, fetch, params)
        frame = fetch()
        if kind == "str":
            return _str(node_id, frame, params)
        if kind == "to_datetime":
            return _convert(node_id, frame, params, dates=True)
        if kind == "to_numeric":
            return _convert(node_id, frame, params, dates=False)
        if kind == "dt":
            return _dt(node_id, frame, params)
        if kind == "shift":
            return frame.shift(_int(params.get("n"), 1))
        if kind == "isin":
            return _isin(node_id, frame, params)
        if kind == "between":
            return _between(node_id, frame, params)
        if kind == "loc":
            return _loc(node_id, frame, params, positional=False)
        if kind == "iloc":
            return _loc(node_id, frame, params, positional=True)
        if kind == "ffill":
            return frame.ffill(limit=_optional_int(params.get("limit")))
        if kind == "bfill":
            return frame.bfill(limit=_optional_int(params.get("limit")))
        if kind == "interpolate":
            return _interpolate(frame, params)
        if kind == "clip":
            return _clip(frame, params)
        if kind == "where":
            return _where(node_id, frame, params)
        if kind == "explode":
            column = str(params.get("column") or "")
            if column not in getattr(frame, "columns", []):
                raise NodeError(node_id, f"missing column {column}")
            return frame.explode(column)
        if kind == "transpose":
            return frame.transpose()
        if kind == "stack":
            return frame.stack()
        if kind == "unstack":
            return _flatten_columns(frame.unstack())
        if kind == "crosstab":
            return _crosstab(node_id, frame, params)
        if kind == "rolling":
            return _rolling(node_id, frame, params)
        if kind == "diff":
            return _numeric_apply(frame, lambda values: values.diff(_int(params.get("periods"), 1)))
        if kind == "pct_change":
            return _numeric_apply(frame, lambda values: values.pct_change(_int(params.get("periods"), 1)))
        if kind == "cumsum":
            return _numeric_apply(frame, lambda values: values.cumsum())
    except NodeError:
        raise
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"{kind} failed: {exc}") from exc
    return _MISSING


def _write(kind: str, node_id: str, frame: Any, params: dict[str, Any], base_dir: Path):
    from pipeline.run import NodeError, _IDENT

    if not isinstance(frame, (pd.DataFrame, pd.Series)):
        raise NodeError(node_id, f"{kind} expected a dataframe or a series")
    path = str(params.get("path") or "").strip()
    if not path:
        raise NodeError(node_id, "no path")
    target = Path(path)
    if not target.is_absolute():
        target = base_dir / target
    index = _yes(params.get("index"))
    if kind == "to_sql":
        table = str(params.get("table") or "")
        if not _IDENT.match(table):
            raise NodeError(node_id, "to_sql needs a table name")
        if_exists = str(params.get("if_exists") or "replace")
        if if_exists not in ("fail", "replace", "append"):
            if_exists = "replace"
        target.parent.mkdir(parents=True, exist_ok=True)
        with sqlite3.connect(target) as con:
            frame.to_sql(table, con, if_exists=if_exists, index=index)
        return frame
    target.parent.mkdir(parents=True, exist_ok=True)
    if kind == "to_csv":
        frame.to_csv(target, index=index, sep=str(params.get("sep") or ",") or ",")
    elif kind == "to_excel":
        sheet = str(params.get("sheet") or "") or "Sheet1"
        frame.to_excel(target, sheet_name=sheet, index=index)
    elif kind == "to_json":
        orient = str(params.get("orient") or "records")
        frame.to_json(target, orient=orient)
    elif kind == "to_parquet":
        payload = frame.to_frame() if isinstance(frame, pd.Series) else frame
        payload.to_parquet(target, index=index)
    return frame


def _str(node_id: str, frame: Any, params: dict[str, Any]):
    from pipeline.run import NodeError

    method = str(params.get("method") or "lower")
    series, column = _series(node_id, frame, params, "str")
    text = series.astype("string")
    pat = str(params.get("pat") or "")
    repl = str(params.get("repl") or "")
    if method in _STR_PLAIN:
        result = getattr(text.str, method)()
    elif method == "contains":
        result = text.str.contains(pat, regex=False, na=False)
    elif method == "replace":
        result = text.str.replace(pat, repl, regex=False)
    elif method == "slice":
        result = text.str.slice(_optional_int(params.get("start")), _optional_int(params.get("stop")))
    elif method in ("split", "rsplit"):
        n = _int(params.get("n"), -1)
        kwargs = {"n": n}
        if pat:
            kwargs["pat"] = pat
        result = getattr(text.str, method)(**kwargs)
    elif method == "extract":
        if not pat:
            raise NodeError(node_id, "extract needs a pattern")
        got = text.str.extract(pat, expand=True)
        got.columns = [str(name) or f"extract_{index}" for index, name in enumerate(got.columns)]
        if isinstance(frame, pd.DataFrame):
            out = frame.copy()
            for name in got.columns:
                out[name] = got[name]
            return out
        return got
    elif method in ("startswith", "endswith"):
        result = getattr(text.str, method)(pat, na=False)
    elif method == "cat":
        joined = text.str.cat(sep=str(params.get("sep") or ""))
        result = pd.Series([joined], name=series.name or "value")
    elif method == "get":
        result = text.str.get(_int(params.get("start"), 0))
    elif method == "zfill":
        result = text.str.zfill(_int(params.get("width"), 1))
    elif method == "pad":
        fill = str(params.get("fill") or " ")
        result = text.str.pad(_int(params.get("width"), 1), side=str(params.get("side") or "left"), fillchar=fill[:1] or " ")
    elif method in ("count", "find"):
        result = getattr(text.str, method)(pat)
    elif method == "match":
        if not pat:
            raise NodeError(node_id, "match needs a pattern")
        result = text.str.match(pat, na=False)
    else:
        raise NodeError(node_id, f"unknown str method {method}")
    return _put(frame, column, result)


def _convert(node_id: str, frame: Any, params: dict[str, Any], dates: bool):
    series, column = _series(node_id, frame, params, "to_datetime" if dates else "to_numeric")
    fmt = str(params.get("format") or "")
    if dates:
        kwargs = {"errors": "coerce"}
        if fmt:
            kwargs["format"] = fmt
        result = pd.to_datetime(series, **kwargs)
    else:
        result = pd.to_numeric(series, errors="coerce")
    return _put(frame, column, result)


def _dt(node_id: str, frame: Any, params: dict[str, Any]):
    from pipeline.run import NodeError

    accessor = str(params.get("accessor") or "year")
    series, column = _series(node_id, frame, params, "dt")
    dates = pd.to_datetime(series, errors="coerce")
    if accessor in ("floor", "ceil", "round"):
        freq = str(params.get("freq") or "D")
        result = getattr(dates.dt, accessor)(freq)
    elif accessor == "strftime":
        result = dates.dt.strftime(str(params.get("fmt") or "%Y-%m-%d"))
    elif accessor == "day_name":
        result = dates.dt.day_name()
    elif accessor == "date":
        result = dates.dt.date
    elif accessor in ("year", "month", "day", "hour", "minute", "second", "dayofweek", "quarter"):
        result = getattr(dates.dt, accessor)
    else:
        raise NodeError(node_id, f"unknown dt accessor {accessor}")
    return _put(frame, column, result)


def _isin(node_id: str, frame: Any, params: dict[str, Any]):
    values = _list_values(params.get("values"), frame if isinstance(frame, pd.Series) else None)
    if isinstance(frame, pd.Series):
        return frame[frame.isin(values)]
    column = str(params.get("column") or "")
    _need_column(node_id, frame, column)
    wanted = _list_values(params.get("values"), frame[column])
    return frame[frame[column].isin(wanted)]


def _between(node_id: str, frame: Any, params: dict[str, Any]):
    lower = _float(params.get("lower"))
    upper = _float(params.get("upper"))
    inclusive = str(params.get("inclusive") or "both")
    if isinstance(frame, pd.Series):
        mask = frame.between(lower, upper, inclusive=inclusive)
        return frame[mask]
    column = str(params.get("column") or "")
    _need_column(node_id, frame, column)
    mask = frame[column].between(lower, upper, inclusive=inclusive)
    return frame[mask.fillna(False)]


def _loc(node_id: str, frame: Any, params: dict[str, Any], positional: bool):
    from pipeline.run import NodeError

    rows = _selector(str(params.get("rows") or ""), positional, getattr(frame, "index", None))
    columns = _selector(str(params.get("columns") or ""), positional, None)
    try:
        if positional:
            if columns is None:
                return frame.iloc[rows] if rows is not None else frame
            return frame.iloc[rows if rows is not None else slice(None), columns]
        if columns is None:
            return frame.loc[rows] if rows is not None else frame
        return frame.loc[rows if rows is not None else slice(None), columns]
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"{'iloc' if positional else 'loc'} failed: {exc}") from exc


def _where(node_id: str, frame: Any, params: dict[str, Any]):
    from pipeline.run import NodeError

    cond = str(params.get("cond") or "")
    if not cond:
        raise NodeError(node_id, "where needs a condition")
    other = params.get("other")
    if other in (None, ""):
        other = None
    else:
        other = _maybe_number(other)
    mode = str(params.get("mode") or "where")
    if isinstance(frame, pd.Series):
        mask = frame.to_frame(name=frame.name or "value").eval(cond)
        mask = mask.iloc[:, 0] if isinstance(mask, pd.DataFrame) else mask
    else:
        mask = frame.eval(cond)
    if mode == "mask":
        return frame.mask(mask, other=other)
    return frame.where(mask, other)


def _interpolate(frame: Any, params: dict[str, Any]):
    method = str(params.get("method") or "linear")
    limit = _optional_int(params.get("limit"))
    if isinstance(frame, pd.Series):
        return frame.interpolate(method=method, limit=limit)
    out = frame.copy()
    numeric = out.select_dtypes(include="number").columns
    out[numeric] = out[numeric].interpolate(method=method, limit=limit)
    return out


def _clip(frame: Any, params: dict[str, Any]):
    lower = _optional_float(params.get("lower"))
    upper = _optional_float(params.get("upper"))
    if isinstance(frame, pd.Series):
        return frame.clip(lower=lower, upper=upper)
    out = frame.copy()
    numeric = out.select_dtypes(include="number").columns
    out[numeric] = out[numeric].clip(lower=lower, upper=upper)
    return out


def _crosstab(node_id: str, frame: Any, params: dict[str, Any]):
    from pipeline.run import NodeError

    if not isinstance(frame, pd.DataFrame):
        raise NodeError(node_id, "crosstab takes a dataframe")
    index = str(params.get("index") or "")
    columns = str(params.get("columns") or "")
    _need_column(node_id, frame, index)
    _need_column(node_id, frame, columns)
    values = str(params.get("values") or "")
    agg = str(params.get("agg") or "count")
    if values:
        _need_column(node_id, frame, values)
        table = pd.crosstab(frame[index], frame[columns], values=frame[values], aggfunc=agg)
    else:
        table = pd.crosstab(frame[index], frame[columns])
    return _flatten_columns(table.reset_index())


def _rolling(node_id: str, frame: Any, params: dict[str, Any]):
    from pipeline.run import NodeError

    window = _int(params.get("window"), 3)
    if window < 1:
        raise NodeError(node_id, "window must be at least 1")
    agg = str(params.get("agg") or "mean")
    if agg not in ("mean", "sum", "min", "max", "std", "median"):
        raise NodeError(node_id, f"unknown rolling aggregation {agg}")

    def roll(values: pd.Series) -> pd.Series:
        return getattr(values.rolling(window, min_periods=1), agg)()

    column = str(params.get("column") or "")
    if isinstance(frame, pd.Series):
        return roll(frame)
    if column:
        _need_column(node_id, frame, column)
        out = frame.copy()
        out[column] = roll(out[column])
        return out
    return _numeric_apply(frame, roll)


def _join(node_id: str, fetch, params: dict[str, Any]):
    from pipeline.run import NodeError

    left = fetch("left")
    right = fetch("right")
    if not isinstance(left, pd.DataFrame) or not isinstance(right, pd.DataFrame):
        raise NodeError(node_id, "join needs two dataframes")
    how = str(params.get("how") or "left")
    on = str(params.get("on") or "")
    kwargs: dict[str, Any] = {"how": how}
    if on:
        if on in right.columns:
            right = right.set_index(on)
        kwargs["on"] = on
    try:
        return left.join(right, **kwargs)
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"join failed: {exc}") from exc


def _merge_asof(node_id: str, fetch, params: dict[str, Any]):
    from pipeline.run import NodeError

    left = fetch("left")
    right = fetch("right")
    if not isinstance(left, pd.DataFrame) or not isinstance(right, pd.DataFrame):
        raise NodeError(node_id, "merge_asof needs two dataframes")
    on = str(params.get("on") or "")
    if not on:
        raise NodeError(node_id, "merge_asof needs a key")
    direction = str(params.get("direction") or "backward")
    try:
        return pd.merge_asof(left.sort_values(on), right.sort_values(on), on=on, direction=direction)
    except Exception as exc:  # noqa: BLE001
        raise NodeError(node_id, f"merge_asof failed: {exc}") from exc


def _series(node_id: str, frame: Any, params: dict[str, Any], kind: str):
    from pipeline.run import NodeError

    if isinstance(frame, pd.Series):
        return frame, None
    if not isinstance(frame, pd.DataFrame):
        raise NodeError(node_id, f"{kind} expected a dataframe or a series")
    column = str(params.get("column") or "")
    if not column:
        raise NodeError(node_id, f"{kind} needs a column")
    _need_column(node_id, frame, column)
    return frame[column], column


def _put(frame: Any, column: str | None, result: Any):
    if column is None or not isinstance(frame, pd.DataFrame):
        return result
    out = frame.copy()
    out[column] = result
    return out


def _need_column(node_id: str, frame: pd.DataFrame, column: str) -> None:
    from pipeline.run import NodeError

    if not column or column not in frame.columns:
        raise NodeError(node_id, f"missing column {column or '(none)'}")


def _numeric_apply(frame: Any, fn):
    if isinstance(frame, pd.Series):
        return fn(frame)
    out = frame.copy()
    numeric = out.select_dtypes(include="number").columns
    out[numeric] = fn(out[numeric])
    return out


def _flatten_columns(frame: Any):
    if isinstance(frame, pd.Series):
        return frame
    if isinstance(frame.columns, pd.MultiIndex):
        frame = frame.copy()
        frame.columns = [
            " ".join(str(part) for part in col if str(part) not in ("", "nan")).strip() for col in frame.columns
        ]
    frame.columns = [str(col) for col in frame.columns]
    return frame


def _selector(text: str, positional: bool, index: Any):
    text = text.strip()
    if not text:
        return None
    if ":" in text and "," not in text:
        start, end = text.split(":", 1)
        return slice(_token(start, positional, index), _token(end, positional, index))
    if "," in text:
        return [_token(part, positional, index) for part in text.split(",") if part.strip()]
    return _token(text, positional, index)


def _token(text: str, positional: bool, index: Any):
    text = text.strip()
    if text == "":
        return None
    if positional or (text.lstrip("-").isdigit() and index is not None and pd.api.types.is_integer_dtype(index)):
        if text.lstrip("-").isdigit():
            return int(text)
    if positional and text.lstrip("-").isdigit():
        return int(text)
    return text


def _list_values(text: Any, sample: Any) -> list[Any]:
    parts = [part.strip() for part in str(text or "").split(",") if part.strip()]
    numeric = sample is not None and pd.api.types.is_numeric_dtype(sample)
    if not numeric:
        return parts
    values: list[Any] = []
    for part in parts:
        if part.lstrip("-").isdigit():
            values.append(int(part))
        else:
            try:
                values.append(float(part))
            except ValueError:
                values.append(part)
    return values


def _int(value: Any, default: int) -> int:
    if value in (None, ""):
        return default
    return int(value)


def _optional_int(value: Any) -> int | None:
    if value in (None, "", 0, "0"):
        return None
    return int(value)


def _float(value: Any) -> float:
    return float(value)


def _optional_float(value: Any) -> float | None:
    if value in (None, ""):
        return None
    return float(value)


def _maybe_number(value: Any) -> Any:
    text = str(value)
    if text.lstrip("-").isdigit():
        return int(text)
    try:
        return float(text)
    except ValueError:
        return text


def _yes(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value or "").lower() in ("yes", "true", "1")
