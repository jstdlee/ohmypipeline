# Ohmypipeline

A native visual data pipeline. Nodes are pandas features. The canvas connects them. Charts are drawn with GPUI Kit Plot.

Linux aarch64 first. One Rust binary, `ompp`. Pandas runs in a worker process. The window never holds a frame.

Pipelines are UTF-8 JSON files, `*.pp`.

## Docs

| File | What it is |
| --- | --- |
| [DESIGN.md](DESIGN.md) | The product. |
| [PRD.md](PRD.md) | What to build, and in what order. |

When the two disagree about sequence or about the first build, `PRD.md` wins.

## Preview

The default run is a preview. Each reader stops at 10,000 rows, then the pipeline runs on that input. Full dataset stays disabled until that preview succeeds. Editing the graph disables it again.

## Status

The window, the worker, and the `*.pp` file are in the tree. The release build is `aarch64-unknown-linux-gnu`. An `x86_64` build is a development convenience on machines without an ARM runner: it is not a release target, and it is not cross-compiled.

```text
cargo run -p ompp
cargo run -p ompp -- --pipeline examples/orders.pp --input examples/orders.csv

`examples/sales.pp` is a longer graph: it skips a title row, pivots quantity by region and product, and ends in a preview table, a line plot, and a pie plot. `examples/text.pp`, `select.pp`, `clean.pp`, `windows.pp`, `crosstab.pp`, and `export.pp` cover text and dates, row selection, filling and clipping, windows, a crosstab, and a CSV writer.
```

The pandas worker is the `pipeline` package under `worker/`. `pandas` is pinned to the 3.1 line. Point `PYTHONPATH` at `worker/`, or set `OMPP_WORKER` to that directory. A repo-local `.venv` is used when it is present.

Linux needs a session and a Vulkan driver. A loader with no device is not a run. On a machine without a GPU, Mesa lavapipe (`mesa-vulkan-drivers`) under Xvfb is enough to open the window.
