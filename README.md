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

Design only. No application code yet.
