"""Explicit scalers, a small OLS fit, and column arithmetic."""

from __future__ import annotations

import ast
import operator
from typing import Any

import numpy as np
import pandas as pd

_FITS: dict[str, dict[str, Any]] = {}

_OPS = {
    ast.Add: operator.add,
    ast.Sub: operator.sub,
    ast.Mult: operator.mul,
    ast.Div: operator.truediv,
    ast.UAdd: operator.pos,
    ast.USub: operator.neg,
}


def clear_fits() -> None:
    _FITS.clear()


def take_fit(node_id: str) -> dict[str, Any] | None:
    return _FITS.pop(node_id, None)


def run_scale(kind: str, node_id: str, node: dict[str, Any], params: dict[str, Any], frame: Any, fit_frame: Any, mode: str):
    from pipeline.run import NodeError

    if kind not in ("standard_scale", "minmax_scale", "robust_scale", "ols", "calculate"):
        return None
    if not isinstance(frame, pd.DataFrame):
        raise NodeError(node_id, f"{kind} takes a dataframe")
    if kind == "calculate":
        return _calculate(node_id, frame, str(params.get("expr") or ""))
    saved = node.get("fitted") if isinstance(node.get("fitted"), dict) else None
    action = str(params.get("action") or "fit_transform")
    if kind == "ols":
        return _ols(node_id, frame, fit_frame if isinstance(fit_frame, pd.DataFrame) else None, params, mode, saved, action)
    columns = _columns(frame, str(params.get("columns") or ""))
    if not columns:
        raise NodeError(node_id, f"{kind} needs at least one numeric column")
    source = fit_frame if isinstance(fit_frame, pd.DataFrame) else frame
    if action == "transform":
        if not saved or "columns" not in saved:
            raise NodeError(node_id, "no fitted parameters. Fit this node before transforming.")
        bundle = saved
    else:
        bundle = _fit_scale(kind, source, columns, mode)
        _FITS[node_id] = bundle
    return _apply_scale(kind, frame, bundle)


def _fit_scale(kind: str, frame: pd.DataFrame, columns: list[str], mode: str) -> dict[str, Any]:
    stats: dict[str, Any] = {}
    for name in columns:
        values = pd.to_numeric(frame[name], errors="coerce")
        if kind == "standard_scale":
            center = float(values.mean())
            scale = float(values.std(ddof=0)) or 1.0
        elif kind == "minmax_scale":
            center = float(values.min())
            scale = float(values.max() - values.min()) or 1.0
        else:
            center = float(values.median())
            quartile = values.quantile(0.75) - values.quantile(0.25)
            scale = float(quartile) or 1.0
        stats[name] = {"center": center, "scale": scale}
    return {
        "kind": kind,
        "source": mode,
        "summary": _summary(mode),
        "columns": stats,
    }


def _apply_scale(kind: str, frame: pd.DataFrame, bundle: dict[str, Any]) -> pd.DataFrame:
    out = frame.copy()
    for name, stats in (bundle.get("columns") or {}).items():
        if name not in out.columns:
            continue
        values = pd.to_numeric(out[name], errors="coerce")
        center = float(stats["center"])
        scale = float(stats["scale"]) or 1.0
        if kind == "minmax_scale":
            out[name] = (values - center) / scale
        else:
            out[name] = (values - center) / scale
    return out


def _ols(node_id, frame, fit_frame, params, mode, saved, action):
    from pipeline.run import NodeError

    target = str(params.get("y") or "")
    predictors = [part.strip() for part in str(params.get("x") or "").split(",") if part.strip()]
    if not target or not predictors:
        raise NodeError(node_id, "ols needs a y column and at least one x column")
    if action == "transform":
        if not saved or "coefficients" not in saved:
            raise NodeError(node_id, "no fitted coefficients. Fit this node before transforming.")
        bundle = saved
    else:
        source = fit_frame if fit_frame is not None else frame
        for name in [target, *predictors]:
            if name not in source.columns:
                raise NodeError(node_id, f"missing column {name}")
        design = np.column_stack(
            [np.ones(len(source))] + [pd.to_numeric(source[name], errors="coerce").to_numpy() for name in predictors]
        )
        observed = pd.to_numeric(source[target], errors="coerce").to_numpy()
        mask = np.isfinite(design).all(axis=1) & np.isfinite(observed)
        if mask.sum() < len(predictors) + 1:
            raise NodeError(node_id, "ols needs more complete rows than coefficients")
        coefficients, *_rest = np.linalg.lstsq(design[mask], observed[mask], rcond=None)
        bundle = {
            "kind": "ols",
            "source": mode,
            "summary": _summary(mode),
            "y": target,
            "x": predictors,
            "coefficients": {"intercept": float(coefficients[0]), **{name: float(coefficients[i + 1]) for i, name in enumerate(predictors)}},
        }
        _FITS[node_id] = bundle
    return _apply_ols(frame, bundle, str(params.get("residuals") or "yes") != "no")


def _apply_ols(frame: pd.DataFrame, bundle: dict[str, Any], residuals: bool) -> pd.DataFrame:
    target = bundle["y"]
    predictors = bundle["x"]
    coef = bundle["coefficients"]
    out = frame.copy()
    fitted = np.full(len(out), float(coef["intercept"]))
    for name in predictors:
        if name not in out.columns:
            continue
        fitted = fitted + pd.to_numeric(out[name], errors="coerce").to_numpy() * float(coef[name])
    out[f"{target}_fitted"] = fitted
    if residuals and target in out.columns:
        out[f"{target}_residual"] = pd.to_numeric(out[target], errors="coerce").to_numpy() - fitted
    return out


def _calculate(node_id: str, frame: pd.DataFrame, text: str) -> pd.DataFrame:
    from pipeline.run import NodeError

    lines = [line.strip() for line in text.splitlines() if line.strip() and not line.strip().startswith("#")]
    if not lines:
        raise NodeError(node_id, "calculate needs an assignment such as revenue = qty * price")
    out = frame.copy()
    for line in lines:
        if "=" not in line:
            raise NodeError(node_id, f"expected name = expression, got {line}")
        name, expr = line.split("=", 1)
        name = name.strip()
        expr = expr.strip()
        if not name.isidentifier():
            raise NodeError(node_id, f"{name} is not a column name")
        try:
            out[name] = _eval_expr(expr, out)
        except ValueError as exc:
            raise NodeError(node_id, str(exc)) from exc
    return out


def _eval_expr(expr: str, frame: pd.DataFrame):
    try:
        tree = ast.parse(expr, mode="eval")
    except SyntaxError as exc:
        raise ValueError(f"cannot read {expr}") from exc
    return _walk(tree.body, frame)


def _walk(node: ast.AST, frame: pd.DataFrame):
    if isinstance(node, ast.BinOp) and type(node.op) in _OPS:
        return _OPS[type(node.op)](_walk(node.left, frame), _walk(node.right, frame))
    if isinstance(node, ast.UnaryOp) and type(node.op) in _OPS:
        return _OPS[type(node.op)](_walk(node.operand, frame))
    if isinstance(node, ast.Constant) and isinstance(node.value, (int, float)):
        return node.value
    if isinstance(node, ast.Name):
        if node.id not in frame.columns:
            raise ValueError(f"unknown column {node.id}")
        return frame[node.id]
    raise ValueError("only + - * / parentheses, numbers, and column names are allowed")


def _columns(frame: pd.DataFrame, text: str) -> list[str]:
    numeric = [str(name) for name in frame.select_dtypes(include="number").columns]
    requested = [part.strip() for part in text.split(",") if part.strip()]
    if not requested:
        return numeric
    return [name for name in requested if name in numeric]


def _summary(mode: str) -> str:
    if mode == "preview":
        return "Fit on the preview, at most 10,000 source rows, not the full file."
    return "Fit on the full dataset."
