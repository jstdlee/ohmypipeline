# Ohmypipeline PRD

Build contract for the next implementation. Product decisions that are not in a milestone live in `DESIGN.md`. This file wins when the two disagree about order or about what the first native build contains.

## Outcome

A native Linux ARM window. One graph on a canvas. Features are cards with typed ports. Wires connect only when the types allow it. The first graph that runs is `read_csv` → `head` → `change_headers`, and the preview table shows the window of rows. A chart, when a chart feature is selected, is drawn with GPUI Kit Plot.

No web view. No macOS or Windows build in this PRD. No auto-update in this PRD.

## Platform

| | |
| --- | --- |
| OS | Linux |
| CPU | `aarch64-unknown-linux-gnu` only. v1 does not cross-compile. |
| UI | [GPUI Kit](https://gpui-kit.com/component/), current 0.9 line |
| Chart | [`gpui_kit::component::plot`](https://gpui-kit.com/component/plot/) for marks, scales, axes, hover. Styled `Chart` only when a stock type already matches. |
| Table | GPUI Kit DataTable, virtualized, over the preview window |
| Shell | GPUI Kit Dock |
| Graph | Python worker, pandas. The window never imports pandas and never holds a frame. |

A Linux GPUI window needs a session and a Vulkan driver. A machine with only the loader does not count as a run.

Release binary for this PRD is `target/release/ompp`, run from the repo. Packaging waits.

## What is new code

GPUI Kit does not have a node editor. These are ours:

| Component | Owns |
| --- | --- |
| `FeatureCard` | One node: title, parameter summary, ports |
| `Port` | Hit target and type label on a card |
| `WireLayer` | Cubic wires, painted, not widgets |
| `NodeCanvas` | Camera, tools, gestures, hit testing |
| `Inspector` | Parameters of the selection |

Plot is not used to draw cards, wires, or marks. Plot has scales and shapes for data. The canvas is ordinary GPUI layout plus `canvas` / `paint_path` for wires and mark outlines. Paint does not hit-test. Hits are computed from the model in canvas space, then converted to the window. That split is the same one GPUI documents for custom paint: layout gives bounds, paint only draws.

## Patterns we copy, and the code we do not

Behavior only. No port of Dear ImGui, imnodes, imgui-node-editor, godraw, or tldraw.

From [imgui-node-editor](https://github.com/thedmd/imgui-node-editor) and [imnodes](https://github.com/Nelarius/imnodes):

- A link drag starts on a port, not on the card body.
- While the button is down, a cubic follows the pointer. If a compatible port is inside the snap radius, the cubic ends on that port. If not, it ends on the pointer.
- Releasing on a compatible port accepts the link. Releasing elsewhere cancels it. Type failure, a cycle, a duplicate, or a full single input rejects it. The reject is visible before release, not only after.
- Dragging the end of an existing wire detaches it and starts the same drag. Dropping it on empty deletes it.
- One output may feed many inputs. An input accepts one wire unless the feature says `multi`.
- Select is click and marquee. Pan is a different button. Context menus exist for a port, a wire, a card, and empty canvas.

From [godraw](https://github.com/egoist/godraw) (tldraw's gestures, native, editor split from the widgets):

- The camera pans, zooms toward the pointer, and fits the selection.
- Grid snap. A modifier disables snap for that drag.
- Alt-drag duplicates the selection.
- Decorative arrows bind to shapes and move with them. Our data wires do the same: endpoints are derived from the card position and the port slot. The file does not store wire curves.
- Undo stores the records it replaced. It does not store pixels.
- Editor rules run with no window, so they are tested without GPUI.

We do not take godraw's pen, highlighter, laser, twenty shapes, image crop, minimap, or `.tldr` files.

## Layers

Dependencies point down. A layer does not import the one above it.

```text
app          GPUI window, dock, inspector, preview, chart
canvas       gestures, hit tests, FeatureCard, WireLayer, camera
connect      accept / reject a wire. no geometry kit, no GPUI
model        features, ports, parameters, graph records. no GPUI
worker       pandas. speaks JSON with app. no GPUI
```

`connect` and `model` are one Rust crate, `graph`, with modules. `canvas` is a crate that depends on `graph` and GPUI Kit. `worker` is Python. `app` starts the window and the worker.

### 1. Model

A feature definition is static data. A node is one use of a feature on a graph.

```text
FeatureDef
  id            stable string, also the .pp type
  title
  inputs        [PortDef]
  output        PortDef or none
  params        [ParamDef]

PortDef
  name
  accepts       non-empty list of Type
  multi         true only for concat-style inputs

ParamDef
  name
  kind          int | float | bool | string | enum | column | code
  wireable      true if the inspector can turn it into a port

Type
  dataframe | series | index | scalar | list | dict | ndarray | code

Node
  id, feature id, position (canvas px), param values, extra ports from wireable params

Link
  id, from node, from port, to node, to port

Graph
  nodes, links, marks, camera
```

There is no feature named `rows` or `columns`. A Series is a `Type`, produced by the `column` feature. `to_frame` is how a Series becomes a DataFrame.

`change_headers` is a feature: input `dataframe`, wireable param `mapping` (`dict` or `list`), output `dataframe`.

The `.pp` file is the graph serialized. Marks and camera are stored and ignored by the worker. Unknown `version` is a hard error. Unknown feature id loads as an error node and does not run.

First catalog entries, enough to draw and to run: `read_csv`, `head`, `change_headers`, `query`, `column`, `literal`, `python`, `chart`. The rest of the pandas list in `DESIGN.md` is data added to this same `FeatureDef` table. A new pandas call does not get a new widget.

### 2. Connection

Pure functions on the model. No pointer events inside them. The canvas turns a gesture into a call.

| Rule | Result |
| --- | --- |
| Output type is not in the input's `accepts` | reject `type` |
| Input is not `multi` and already has a link | reject `full` |
| Same pair of ports already linked | reject `duplicate` |
| The new edge would cycle | reject `cycle` |
| Otherwise | accept, append `Link` |

Snap radius is 12 window px, measured after the camera. A canvas-pixel radius shrinks to nothing when the view is zoomed out and becomes a huge grab when zoomed in. Among ports inside the radius, the nearest compatible port wins. An incompatible port inside the radius does not steal the snap. `graph` tests pass distances in; they do not assume a zoom.

Detach: the link is removed when the drag starts, and the drag holds the surviving end. Cancel (Escape, or release on empty) leaves it removed. Accept writes a new link. Undo is one record replacement for the whole gesture, so a cancelled detach puts the old link back only through undo, not through a hidden buffer. The gesture's undo entry is committed on accept or on cancel-after-detach. Cancel of a drag that started from an unconnected port writes nothing.

Fan-out is free. Fan-in is `multi` or a second input port (`merge` has `left` and `right`, plus Add input which appends a `PortDef` on that node only).

### 3. Canvas

Coordinate spaces:

| Space | Used for |
| --- | --- |
| Canvas | node positions, grid, card hit tests, the `.pp` |
| Window | GPUI layout, `paint_path`, port-snap distance |

`window = canvas * zoom + camera`. Zoom is toward the pointer. Fit frames the selection, or every node if the selection is empty. Grid is 16 canvas px. Snap is on while dragging cards; hold Alt to skip it for that drag.

Paint order, back to front: mark fills, wires, cards, mark text that must stay readable (labels, notes, buttons). Wires are one `PathBuilder` stroke per link, cubic from source port to target port, control points horizontal so the curve leaves a port in its direction. The in-progress wire is the same path with the free end at the pointer or the snapped port.

Hit order is the reverse of paint. A port wins over its card. A card wins over a wire. Empty canvas starts a marquee on the select tool, or a pan on the hand tool and on middle-button.

Tools, one active, godraw-style (the tool stays selected):

| Tool | Pointer |
| --- | --- |
| Select | click, shift-click, marquee, drag cards, drag a port to link |
| Hand | pan. Space held down is Hand until key-up, then the previous tool returns |
| Mark: rectangle, circle, label, note, button | drag to place. These create marks, not features |

Shortcuts: Ctrl+wheel zoom, Ctrl+0 fit, Ctrl+Z / Shift+Z undo, Backspace delete, Ctrl+D duplicate, Alt-drag duplicate.

`FeatureCard` is a GPUI element of fixed width. Ports are children with stable ids, so the hit target is the element's bounds, not a painted circle we then fail to hit. The wire layer reads those bounds after layout (prepaint stores them on the canvas entity) and paints in the following paint. A card moved this frame does not use last frame's port bounds.

The inspector is a docked panel, not a modal. Editing a parameter replaces that node's record and marks dependents stale. It does not run pandas.

### 4. Run one graph

Worker protocol, length-prefixed JSON on stdin/stdout:

```text
→ { "generation": 1, "mode": "preview", "graph": {…}, "node": "n3" }
← { "generation": 1, "mode": "preview", "columns": ["a"], "rows": [["x"]], "n": 10000, "claim": "preview", "error": null }
```

The worker drops marks, checks the same cycle rule, and runs pandas only for the requested node and its ancestors. Preview is the default: each reader stops at 10,000 rows, then the pipeline runs on that input. Full removes the cap and stays unavailable in the GUI until the preview for the current graph succeeds. A late generation is discarded by the window.

The status line is part of this milestone, not polish. It says `preview` or `full`, and it never calls a capped run the whole file.

### 5. Chart

A `chart` feature takes one `dataframe`. Selecting it shows the Chart tab. The plot implements GPUI `Plot` and paints with `ScaleLinear` / `ScaleBand` / `ScalePoint`, `Line`, `Bar`, `Area`, `Arc`. Title and legend are GPUI text over the plot, positions stored as fractions on the feature's presentation record. The stock chart legend stays off. More than 20,000 points refuses with the count. The chart draws the preview window and says so.

## Rules that are easy to leave until they hurt

These change M2 or M3. They are locked.

**Declared type is the contract.** The wire believes the feature's output `Type`. If pandas returns something else, that node errors and names both types. Nothing is silently wrapped into a DataFrame.

**The index is a column in the preview.** The first DataTable column is the index, pinned. Hiding it makes `set_index` look like a no-op.

**A column parameter is a name.** `column`, `str`, and `dt` store that name as text. If an upstream `change_headers` removes it, the node errors with the missing name. It is not retargeted by position.

**The inspector sniffs headers, it does not preview.** Column dropdowns come from the CSV's first record, SQLite `PRAGMA table_info`, or the Excel header row. If the sniff fails, the dropdown fills after the first successful preview. The body is not read to populate a menu.

**A preview limit is pushed only into a reader.** Preview mode caps each reader at 10,000 rows (`nrows`, SQL `LIMIT`, or the Excel row cap). The rest of the pipeline, including sort and groupby, runs on that input. The worker never reads the whole file and then slices, and never pushes the cap through an aggregate. A `head` the user placed is an ordinary feature. Full mode does not delete it.

**The worker caches, and a new job kills it.** The last frame per node is kept, keyed by that node's record and its parents' keys. An unchanged ancestor is not recomputed. stdout is frames only. Warnings and tracebacks go to stderr, which the window logs and does not parse. A new job that arrives while pandas is running kills the process and starts another. Dropping a late generation in the window does not stop the read.

**A focused text field eats canvas keys.** Space, Backspace, and the tool keys do nothing while the inspector or the code editor is focused.

**Crash recovery is one sibling file.** `name.pp.recovery` is rewritten while the document is dirty and removed after a clean save. The next open offers it when it is newer. Groups are not in M1–M6, so the canvas has no scope stack.

**M3 uses a repo venv.** The interpreter is `python3`. Pandas is pinned to the 3.1 line the catalog was read against. A bundled interpreter waits for the updater.

Left for later, on purpose: English chrome with no i18n framework, the app license, watching the `.pp` for another process's edits, a default seed parameter on `sample` when that feature exists, a keyboard-only link gesture (port labels for AccessKit still ship with the card), and more than one window.


## Milestones

Each one is done only when its check passes on Linux aarch64, in a window where a window is required.

| # | Done when |
| --- | --- |
| M1 | `graph` tests: load a `.pp`, reject a cycle, reject a Series wired into `change_headers`, accept `column` → a Series port, snap picks the nearer compatible port. No GPUI. |
| M2 | `ompp` opens a window. Drop `read_csv`, `head`, `change_headers` on the canvas, wire them by dragging ports, pan, zoom, undo the wire, save, reopen the same positions and links. |
| M3 | The same graph previewed through the worker, capped at 10,000 source rows. Status line says `preview`. Full dataset is disabled until that preview succeeds, then runs with no reader cap. DataTable shows the returned window only. |
| M4 | `query`, `column`, `literal`, `python`. A code block's ports are the function arguments. Run shows the block in the inspector before it runs. |
| M5 | Chart tab for a `chart` node, Plot marks, draggable title and legend, saved in the `.pp`. |
| M6 | Rectangle, circle, label, note, button. Button focuses a node. Worker output is unchanged if marks are added. |

Not in these milestones: the rest of the pandas catalog (add as `FeatureDef` rows after M4 without a new gesture), macOS, Windows, the updater, PNG export, pickle, a rows feature, a columns feature.

## First files

```text
crates/graph/        model + connect. tests live here
crates/canvas/       NodeCanvas, FeatureCard, Port, WireLayer
crates/app/          window, dock, inspector, preview, chart
crates/ompp/         binary
worker/              Python package, pandas
```

`graph` has no `gpui` dependency. A test that needs a window is in `canvas` or `app` and is not the test for accept/reject.

## Checks before calling M2 real

- Port hit target receives the press. The card body does not start a link.
- Incompatible port under the pointer does not snap while a compatible one is inside the radius.
- Fit and a 16 px grid survive save and reopen.
- Wire endpoints move when the card moves, with no curve stored in the file.
- Delete, undo, and alt-drag do not leave a link to a missing node.
