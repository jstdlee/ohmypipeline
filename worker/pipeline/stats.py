"""Per-column stats of a whole frame. JSON-safe: no NaN, no numpy scalars."""

from __future__ import annotations

import math
from datetime import date, datetime
from typing import Any

import numpy as np
import pandas as pd

HIST_BINS = 12
TOP_COUNT = 3


def column_stats(value: Any) -> list[dict[str, Any]]:
    """Stats for every column. A Series is one column named by its name."""
    if isinstance(value, pd.DataFrame):
        return [series_stats(str(name), value[name]) for name in value.columns]
    if isinstance(value, pd.Series):
        name = "value" if value.name is None else str(value.name)
        return [series_stats(name, value)]
    return []


def series_stats(name: str, series: pd.Series) -> dict[str, Any]:
    count = int(len(series))
    nulls = int(series.isna().sum())
    null_pct = round(nulls / count * 100, 1) if count else 0.0
    kind = getattr(series.dtype, "kind", "O")
    non_null = series.dropna()
    numeric = kind in ("i", "u", "f")
    stats: dict[str, Any] = {
        "name": name,
        "dtype": str(series.dtype),
        "count": count,
        "nulls": nulls,
        "null_pct": null_pct,
        "unique": _unique(non_null),
        "min": None,
        "max": None,
        "mean": None,
        "std": None,
        "hist": [],
        "top": [],
    }
    if numeric:
        values = pd.to_numeric(non_null, errors="coerce").to_numpy(dtype=float, na_value=np.nan)
        finite = values[np.isfinite(values)]
        if finite.size:
            stats["mean"] = _finite(float(finite.mean()))
            if finite.size > 1:
                stats["std"] = _finite(float(finite.std()))
            if kind in ("i", "u") and non_null.size:
                stats["min"] = _jsonable(non_null.min())
                stats["max"] = _jsonable(non_null.max())
            else:
                stats["min"] = _jsonable(np.min(finite))
                stats["max"] = _jsonable(np.max(finite))
            hist, _ = np.histogram(finite, bins=HIST_BINS)
            stats["hist"] = [int(item) for item in hist]
        return stats
    if kind == "b" and non_null.size:
        stats["min"] = bool(non_null.min())
        stats["max"] = bool(non_null.max())
    if kind == "M" and non_null.size:
        stats["min"] = non_null.min().isoformat()
        stats["max"] = non_null.max().isoformat()
    if kind == "O" and non_null.size:
        values = list(non_null)
        if all(isinstance(item, str) for item in values):
            stats["min"] = min(values)
            stats["max"] = max(values)
    if not any(
        isinstance(item, (list, dict, tuple, set, np.ndarray))
        for item in non_null.head(50)
    ):
        try:
            counts = series.value_counts()
            stats["top"] = [[_jsonable(item), int(total)] for item, total in counts.head(TOP_COUNT).items()]
        except (TypeError, ValueError):
            stats["top"] = []
    return stats


def _unique(non_null: pd.Series) -> int | None:
    try:
        return int(non_null.nunique(dropna=True))
    except (TypeError, ValueError):
        return None


def _finite(number: float) -> float | None:
    if math.isnan(number) or math.isinf(number):
        return None
    return number


def _jsonable(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, (bool, np.bool_)):
        return bool(value)
    if isinstance(value, (int, np.integer)):
        return int(value)
    if isinstance(value, (float, np.floating)):
        return _finite(float(value))
    if isinstance(value, (pd.Timestamp, datetime, date)):
        return value.isoformat()
    if isinstance(value, np.generic):
        return _jsonable(value.item())
    if isinstance(value, str):
        return value
    return str(value)
