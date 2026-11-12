# Plan: canvas and flow v2

Stacked on `cursor/ohmypipeline-app-900b` (head `555b2a2`). Branch: `feat/canvas-flow-v2`.

Scope is the fifteen items below. Out of scope: component or subflow encapsulation and reuse, collapse-to-subgraph, run history and comparison, lineage, servers, schedulers, team features, distributed execution.

## Audit of the current code

| Item | Today | Gap |
| --- | --- | --- |
| 1 Zoom-scaled cards | Card box, ports, and paddings multiply by `camera.zoom`; text uses fixed `text_xs`/`text_sm`. | Text does not scale, so titles clip when zoomed out and look tiny when zoomed in. No level of detail. |
| 2 Toolbar, id, dock sizes | Tools are a grey capsule at the bottom of the canvas; inspector prints the feature id under the title; dock split sizes reset on every launch. | All three. |
| 3 Plot title and legend | `Presentation { title, legend: Placed { x, y, anchor } }` exists in the model and the file. | No drag gesture on the chart; positions are only defaults. |
| 4 Drop wire to search | Canvas emits `CanvasEvent::OpenCatalog { filter }` on an empty drop; the workspace ignores it. `Document::features_accepting` exists. | Popover, filtering, placement, auto-connect. |
| 5 Reroute, insert, multi-wire | `Document::insert_on_edge` exists and is tested, but no gesture calls it. | Reroute points (none in the model), drop-node-on-wire gesture, Ctrl/Alt-drag to move every wire of a port. |
| 6 Group frames | Rectangle marks exist but do not own nodes. | A frame record with title, swatch, size, collapsed flag; containment; move-with-frame. |
| 7 Minimap, auto-layout | None. `fit_camera` and camera animation exist. | Both. |
| 8 Undo/redo | `History` stores whole-graph snapshots of nodes, edges, marks per gesture; Ctrl+Z exists. | New records (frames, reroutes, variables) must be in the snapshot; tests for each edit kind; Shift+Ctrl+Z and Ctrl+Y bound. |
| 9 Performance | Every node and wire is laid out each frame. | Viewport culling, wire path cache keyed on endpoints, a synthetic 500-node bench. |
| 10 Node state | One dot per node (muted, success, warning, danger, running) from the last preview. Worker caches by dependency key. | Explicit state machine, per-node Execute/Reset/Execute-to-here, downstream-only invalidation, stale badge. |
| 11 Data on a wire | Preview panel shows the selected node only. | Click a wire or port to show that frame, with per-column stats and a missing-value overview. |
| 12 Flow variables | None. | Typed pipeline variables, `${name}` substitution in params, an inspector panel. |
| 13 Parameterized runs | `ompp --pipeline f.pp --input id=path`. | `ompp run f.pp --set key=value` and a run dialog. |
| 14 Loops | None. | `loop_start`/`loop_end` pair over a glob, column groups, or a list; concat collect. |
| 15 Branches | None. | `if_switch` with active/inactive outputs, `end_if` merge, inactive downstream skipped and dimmed. |

## File format: version 2

`Graph.version` goes from 1 to 2. Load accepts 1 and 2; version 1 migrates in memory by filling the new fields with defaults. Saving always writes 2. Unknown higher versions are refused as today.

New fields, all `#[serde(default, skip_serializing_if = ...)]`:

```text
Graph.frames:    [Frame { id, title, swatch, position:[x,y], size:[w,h], collapsed:bool }]
Graph.variables: [Variable { name, kind: "string"|"int"|"float"|"bool"|"path", value }]
Graph.view:      View { dock: {left, right, bottom} sizes in px, minimap: bool }
Edge.reroutes:   [[x,y], ...]          canvas-space points, in wire order
Node.presentation already holds title/legend; no change.
```

Frame membership is derived, not stored: a node belongs to the frame whose rectangle contains the node's top-left corner. That keeps the file free of a second source of truth.

Node state is not saved. Cached outputs live in the worker.

New features in the catalog: `loop_start`, `loop_end`, `if_switch`, `end_if`. They follow the existing `FeatureDef` table, so templates export them like any other node.

## Milestones

Each milestone is one commit (or a few) and must pass `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `PYTHONPATH=worker python3 -m unittest worker.tests.test_pipeline`.

### M1 Schema v2

- Add the fields above, migration from v1, undo snapshot extended to frames and variables, round-trip tests for v1 to v2.
- Accept: every `examples/*.pp` loads; saving writes `"version": 2`; a v3 file is refused.

### M2 Card zoom, toolbar, inspector, dock sizes, plot layout (items 1, 2, 3)

- Card text uses `text_size(px(base * zoom))`. Below zoom 0.55 the card draws title and ports only; below 0.3 the title alone with a larger font. Card height follows the simplified layout so ports still line up with wires.
- Floating toolbar: top-center of the canvas, 32 px tall, 1 px border, theme background, icon buttons with tooltips and the active tool tinted; undo, redo, fit, minimap, auto-layout on the right of a divider.
- Inspector: the feature id moves into a tooltip on the title.
- Dock sizes saved into `Graph.view.dock` on resize end and restored on open; also mirrored into the user config for new documents.
- Chart title and legend can be dragged; the position is saved to `presentation` as fractions of the plot box. Double-click resets.
- Accept: screenshots at zoom 0.4, 1.0, 1.8 with readable titles; toolbar screenshot; a test that `Presentation` round-trips.

### M3 Wire gestures (items 4, 5)

- Drop on empty: popover at the pointer with a search field, listing `features_accepting(type)`; Enter or click adds the node at the drop point and connects the first compatible input. Escape cancels. Same popover for a dragged input end (lists producers).
- Reroutes: double-click a wire adds a point at the nearest segment; points drag with grid snap; Delete or double-click on a point removes it. The wire draws as a chain of cubics through the points.
- Drop a single dragged node onto a wire (hit within 8 px, node has a compatible in and out, node unconnected): highlight the wire while hovering, release inserts with `insert_on_edge`.
- Ctrl-drag (Alt on Linux desktops that steal Ctrl-drag is the alternate) from an output port picks up all its wires and drops them on another compatible output.
- Accept: `graph` unit tests for reroute insert/remove/move and for multi-rewire; screenshot of the popover and a rerouted wire.

### M4 Frames, minimap, auto-layout, undo (items 6, 7, 8)

- Frames: a Frame tool and "Group selection" (Ctrl+G) that sizes a frame around the selection with padding. Title editable in place, six swatches, 8 px corner handles, collapse to a 28 px header that hides member cards and draws their outside wires to the header edge. Moving a frame moves its members in the same gesture.
- Minimap: bottom-right, 180 x 120, drawn from node rects and the viewport rect; click or drag moves the camera. Toggled by the toolbar and `M`.
- Auto-layout: longest-path layering left to right, barycenter ordering within layers, 80 px column gap and 32 px row gap; applies to the selection or everything; animated with the existing settle motion and one undo step.
- Undo: every edit goes through `Document`; tests assert one undo step restores the prior graph for add, delete, move, connect, detach, param, label, notes, frame add/move/resize/collapse, reroute add/move/remove, variable add/edit, auto-layout.
- Accept: tests above; screenshots of frames (one collapsed), minimap, and before/after layout.

### M5 Performance (item 9)

- Cull cards and marks whose rect is outside the viewport plus a 64 px margin; cull wires whose bounding box is outside.
- Cache wire cubics per edge keyed on endpoint positions, reroutes, and camera; drop the cache on edit.
- `canvas` bench: `cargo test -p canvas --release -- --ignored bench_500` builds a 500-node, 600-edge graph and asserts layout + hit-test of a frame under a budget (logged, generous threshold for CI). A `--demo big` loads it for a visual check.
- Run the bench with `cargo test -p canvas --release --test perf -- --ignored --nocapture`. `--demo big` parks on the first layers at zoom 0.8; `--demo big-out` fits all 400 nodes.

### M6 Execution model and wire data view (items 10, 11)

- State per node: `NotConfigured` (required param empty or required input unwired), `Configured`, `Running`, `Executed`, `Error`, `Stale` (was executed, an ancestor or its own params changed since). Badge drawn in the header, colors from the theme.
- Execute (node), Execute to here (same, explicit name), Reset (drop this node and downstream cache) in the card context menu and inspector header. Execute runs the node in preview mode via the existing job; the worker reports every computed ancestor so they also turn Executed.
- Invalidation: an edit marks the edited node and its descendants Stale; other nodes keep their state. Uses `dependency_key` per node.
- Data view: click a wire or an output port to show that node's output in the preview dock with a stats strip: dtype, non-null, null %, unique, min, max, mean, and a 12-bin histogram for numeric columns; a missing-value bar per column. Worker computes stats in the same reply (`Reply.stats`).
- Accept: graph tests for invalidation, worker tests for stats, screenshots of badges and the stats view.

### M7 Flow variables and parameterized runs (items 12, 13)

- Variables panel in the inspector when nothing is selected (and a "Variables" tab): add, rename, type, value, delete.
- Substitution: any string param may contain `${name}`; numeric params may be exactly `${name}`. The worker substitutes before executing; unknown names are a node error that names the variable.
- CLI: `ompp run file.pp [--set key=value]... [--node id] [--output path] [--full]` (the old `--pipeline` form keeps working). Values are parsed per the declared type; unknown keys are an error.
- Run dialog: "Run with..." lists variables with editable values for that run only.
- Accept: worker tests for substitution and type errors; CLI test; screenshot of the panel and dialog.

### M8 Loops and branches (items 14, 15)

- `loop_start` params: `mode` = `files` (glob, relative to the pipeline) | `groups` (column) | `values` (comma list or a variable); exposes `${loop_value}` and `${loop_index}` to the body. `loop_end` concatenates each iteration's frame and adds a `loop_value` column when asked. The worker runs the body subgraph once per iteration. Preview caps at the first 3 iterations and labels it.
- `if_switch` params: `condition` (a pandas-free expression over variables, e.g. `${mode} == "full"`, or `rows > 0` on the input). Two outputs `true` and `false`; the inactive output yields a skip token; nodes fed only by skip tokens are Skipped (drawn at 40 % opacity, state badge "inactive"). `end_if` takes both and passes whichever is active.
- Accept: worker tests for each loop mode, nested if inside loop, skip propagation; graph tests for the new ports; screenshots of a loop and an inactive branch.

### M9 Review and ship

- Ryugi interface-craft pass over the new surfaces (spacing, color, type, motion), at most two rounds, recorded in `docs/ui-review.md`.
- Screenshots under `docs/screenshots/v2/`, copies under `/workspace/ompp-shots/`.
- Push, open a draft PR into `cursor/ohmypipeline-app-900b`.

## Visual rules

Restrained native, like Zed and Linear: theme colors only, 1 px borders, 6 px radii on cards and 8 px on popovers, no gradients, no glow, no emoji. Motion is short and purposeful: 120 ms for hover and badges, 180 ms for popovers, 260 ms ease-out for layout and camera; everything respects reduced motion.
