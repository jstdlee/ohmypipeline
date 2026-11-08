# Design review: before the TRD and module design

Review of `DESIGN.md` and `PRD.md` at commit `9de7be5`. The product is clear enough to build. It is not yet clear enough to split into modules, because a few contracts that cross a module boundary are either undefined or defined twice, differently.

This file lists what the TRD has to decide. It does not change the product. Where a recommendation is given, it is the smallest choice that keeps both documents true. When a decision is made, move it into `PRD.md` or `DESIGN.md` and delete the row here.

Severity:

- **Blocker**: two modules would be built against different contracts, or M1–M3 cannot pass without it.
- **TRD**: needs a number, a format, or a rule before the module that owns it is designed.
- **Later**: real, but outside M1–M6.

## 1. Conflicts between DESIGN and PRD

`README.md` says the PRD wins on sequence and the first build. Some of these are not about sequence, so the rule does not settle them.

| # | Topic | DESIGN says | PRD says | Severity | Recommendation |
| --- | --- | --- | --- | --- | --- |
| C1 | Crate layout | Three crates: `pipeline` (Python, under `crates/`), `app`, `ompp`. "Three crates, not one per screen." The canvas lives in `app`. | Four Rust crates, `graph`, `canvas`, `app`, `ompp`, and a top-level `worker/` Python package. | Blocker | Take the PRD layout. Delete DESIGN's Code layout block, or point it at the PRD. A Python package does not belong under `crates/`. |
| C2 | Name of the shared core | "Crate `pipeline` has no GPUI" and is the thing GUI and CLI share. Execution also calls the Python package `pipeline`. | The no-GPUI crate is `graph`. The Python side is `worker`. | Blocker | Rust: `graph`. Python: `ompp_worker` (importable, not a generic name). Rename in DESIGN's Decisions and Execution. |
| C3 | Release on empty canvas during a link drag | Opens the catalog, filtered to features that accept the type. | Cancels. A detached wire dropped on empty is deleted. | Blocker | PRD for M2. Catalog-on-drop becomes a later milestone, and when it lands, Escape still cancels. State which one M2 ships. |
| C4 | Drop onto a card body | Accepted when that card has a free input of the type. | Only a compatible port inside the 12 px radius accepts. | TRD | PRD for M2. If body-drop stays in the product, define which free port it picks when two match. |
| C5 | Insert on wire | Part of build step 2, "easy wire". | Not in M2, not in "Not in these milestones". | TRD | Put it in a named milestone or in the excluded list. |
| C6 | First runnable graph | `read_csv` → `query` → `change_headers`, worker and CLI first. | `read_csv` → `head` → `change_headers`, window first (M2), worker at M3. | TRD | PRD. Rewrite DESIGN's Build order as a pointer to the milestones table, so there is one order. |
| C7 | Headless CLI | A full contract: flags, `--input name=path`, exit codes. Build step 1. | Not in any milestone and not in the excluded list. | Blocker | Add it to M3 (same worker, no window). It is the cheapest test harness for the worker. |
| C8 | Graph record names | File: `edges`, entries have no `id`. | Model: `Link` with an `id`, `Graph.links`. | Blocker | One name. Recommend `edges` in the file and in code. Decide whether an edge has an id (undo replaces records by id, so it probably needs one). |
| C9 | Node label | File has `label` on every node. | `Node` has no label. | TRD | Add `label: optional string` to `Node`. Define the default (the feature title) and whether it is stored when equal to the default. |
| C10 | Alt modifier on a card drag | Not discussed. | Alt skips grid snap for that drag. Alt-drag also duplicates the selection. | Blocker | Pick one. Recommend Alt-drag duplicates (godraw) and Shift disables snap, or the reverse, but not both. Also see P3: many Linux window managers take Alt-drag. |
| C11 | Chart renderer | Stock Chart for line, bar, area, pie, radar, candlestick, sankey. Plot for scatter and histogram. | Plot first. Styled Chart only when a stock type matches. Plot shapes listed: Line, Bar, Area, Arc. | TRD | M5 needs a list of chart types and, per type, Plot or Chart. Scatter needs a point shape; it is not in the PRD's list. |
| C12 | Worker reply | Rows, row count, column dtypes, error with the node id. | `columns`, `rows`, `n`, `claim`, `error`. No dtypes, no index, no node id on the error. | Blocker | See section 3. |
| C13 | Pandas version | 3.1.1. | "the 3.1 line". | TRD | Pin an exact version in the venv lock. The catalog was read against 3.1.1. |
| C14 | Code layout of the catalog | `crates/pipeline/` is "Python: catalog". | "The `FeatureDef` table" lives in `graph`. | Blocker | See G1. |

## 2. Gaps that block module design

### G1. One catalog, two languages (Blocker)

`graph` needs every `FeatureDef` to type-check a wire without Python. The worker needs the same ids, port names, and parameter names to call pandas. Two hand-written copies will drift.

Decide the single source. Recommend a data file, `catalog/features.json` (or `.toml`), loaded by both: `graph` embeds it at build time with `include_str!`, the worker reads it at start and asserts that every entry has a runner. A test on each side fails when an id has no implementation. The worker does not send the catalog over the wire; the binary and the worker ship together.

The file needs a field per feature that the docs imply but do not name:

- palette category (the Shell sketch shows IO, Labels, Select, Shape, Code, Chart; nothing maps a feature to one),
- port names (the file example uses `out` and `in`; `merge` uses `left` and `right`; nothing else is named),
- per-parameter default, enum values, and whether it is required,
- the output type rule (G2),
- whether the feature is a reader (preview cap applies) or a writer (G6).

### G2. Output type is not static (Blocker for M1)

`FeatureDef.output` is one `PortDef` with one `Type`. Most of the catalog is polymorphic:

- `head`, `sort_values`, `fillna`, and most methods return the same type they were given, frame or series.
- The reducers return a series from a frame and a scalar from a series.
- `python` declares its type in the inspector.
- A wired `literal` is scalar, list, dict, or code, chosen by a parameter.

The TRD needs an output rule per feature, for example `fixed(T)`, `same_as(port)`, `table(series → scalar, dataframe → series)`, or `param(name)`. It also needs the propagation rule: types resolve from sources down, an unresolved upstream (an empty code block, an error node) yields `unknown`, and the connect rule for `unknown` (recommend: accept, and type-check again when it resolves; the node errors then, per "Declared type is the contract").

M1's "reject a Series wired into `change_headers`" depends on this when the Series comes from `column`, which is fixed, but `head` → `change_headers` after a `column` is not.

### G3. Features that do not return one value (Blocker before the catalog grows)

The model has one output per feature. These pandas calls do not fit as written:

| Call | Returns | Needs a rule |
| --- | --- | --- |
| `factorize`, `align` | a tuple | split into two outputs, or pick one by parameter |
| `insert`, `update` | `None`, mutate in place | the runner copies and returns the frame |
| `pop` | the column, and mutates the frame | which value is the output |
| `info` | `None`, prints | DESIGN already says "a frame"; specify its columns |
| `groupby`, `rolling`, `expanding`, `ewm`, `resample` | an intermediate object | DESIGN pairs each with one reducer in the same feature; say that the reducer is a parameter and the object never crosses a wire |
| `to_csv` and the other writers | `None`, side effect | G6 |
| `at`, `iat`, `get` | a scalar or anything | output type rule |

### G4. Worker protocol (Blocker for M3)

The PRD example is a shape, not a contract. Missing:

- **Framing.** Length prefix width and byte order (recommend u32 big-endian), maximum frame size, and what the window does with a frame over it.
- **Message kinds.** Today there is one request and one reply. The design also needs: a ready/hello frame with the worker and pandas versions, a per-node status update (the card dots need `success`, `error`, `blocked`, `running` for every ancestor, not only the requested node), a cancel or shutdown, and the chart data request (G9).
- **Value encoding.** JSON has no NaN, no datetime, no int64 above 2^53, no bytes, no categorical. Decide how a cell is encoded and how null differs from NaN (the table prints an em dash for null; is NaN null?). Decide for a reply that is a scalar, list, dict, index, or ndarray, not a frame.
- **Index.** "The index is a column in the preview" but the reply has no index. Add it, including a MultiIndex and an unnamed index. Same for MultiIndex columns.
- **Window size.** The reply returns "the window of rows". How many? All of a 10,000-row preview, or the first N? DESIGN's full status says `showing first {window}`. If the table only receives N rows, scrolling past N needs a page request, or a stated limit. Also cap columns and cell text length.
- **dtypes.** DESIGN says the reply has them and the inspector shows them. Add a `dtypes` field.
- **Errors.** An error carries the node id, a kind (type mismatch, missing column, pandas exception, user code exception, worker internal), a message, and optionally the parameter name, so the inspector can put it "under the field that caused it".
- **Generation.** Who allocates it (the window, monotonic for the process), and whether a respawned worker resets anything.
- **Graph payload.** Whether every job sends the whole graph (simple, the cache keys make it cheap) or a diff.

Recommend a shared folder of protocol vectors (request and reply JSON pairs) that a Rust test and a Python test both read, so the contract is tested from both sides.

### G5. When a preview runs (Blocker for M3)

Three sentences pull different ways:

- PRD: editing a parameter "does not run pandas".
- DESIGN: Run is Ctrl/Cmd+Enter and is in the immediate actions.
- DESIGN: "Switching the selected node cancels the in-flight preview", which implies selecting a node starts one.

State the trigger: Run only, or selection plus Run, or a debounce after an edit. State which node a Run previews (the selection; with none selected, the single sink as in the CLI).

The kill-on-new-job rule also kills the cache, because the cache lives in that process. Say that this is accepted, or move the cache out of the process (for example, Parquet files in a per-document cache directory) before M3.

### G6. Writers in preview mode (Blocker before any `to_*` feature)

Preview caps every reader at 10,000 rows and then runs the pipeline. A `to_csv` node in that pipeline would overwrite a real file with a truncated one. Decide: writers do not execute in preview (they report "skipped in preview"), and run only in full mode. The CLI default is preview, so the same rule applies there. Also decide what the CLI writes to stdout when the chosen node is a writer.

### G7. Readers that cannot stop at 10,000 rows (TRD)

"Each reader stops at 10,000 rows" is true for `read_csv`, `read_table`, `read_fwf`, `read_excel` (`nrows`), and SQL (`LIMIT`). It is not true for:

- `read_parquet`: no `nrows`. Use `pyarrow.parquet.ParquetFile.iter_batches` and stop.
- `read_json`: `nrows` only with `lines=True`. Otherwise the whole file is parsed.
- `read_html`, `read_xml`: no row cap. The whole document is parsed.
- `json_normalize`: input is a wired value, not a file.

For each, the TRD states the strategy or the honest status text ("preview read the whole file").

Also: `read_sql` takes "a table or a single SELECT" while the Preview section says "No free-form SQL box". Decide which, and how a user SELECT is limited (wrap it as a subquery, which fails on some statements).

### G8. Python code blocks (Blocker for M4)

- **Ports from source.** "Argument names are the input ports" means the window must know the arguments without running the code. Who parses: the worker (a `parse` request using `ast`), or Rust? Recommend the worker, with the request answered even while no job runs. Define what happens on a syntax error, a `*args`, a default value, and a rename of an argument that has a wire.
- **Port types.** The user sets each port's type and the return type. Store them in the node's params. Define the default.
- **Where the extra ports live in the file.** The PRD model has "extra ports from wireable params" and Add input "appends a PortDef on that node only". The file example has neither. Add a field, for example `ports: [{ name, accepts, from_param }]`.
- **The other expression features.** `query` and `eval` also execute expressions (pandas `eval` resolves attributes and `@` locals). The "A `.pp` from someone else is code" warning applies to them too.

### G9. Chart data path (TRD for M5)

- Does the chart reuse the preview reply, or send its own request with only the encoded columns? The table window may be smaller than the 10,000-row frame the chart plots.
- What counts toward the 20,000-point limit: rows, or rows × series?
- Encodings "type, x, y, optional series" do not cover candlestick (open, high, low, close), sankey (source, target, value), pie (label, value), or histogram (one column, bins). List encodings per chart type.
- `hidden_series` stores names. Define what happens when a series name disappears upstream (keep, drop on save).
- Click on a legend entry toggles a series; drag on the legend moves it. State the drag threshold.

### G10. File format details (Blocker for M2 save/reopen)

The JSON example is a fragment (its closing brace was missing; fixed in this commit). Before `graph` can serialize, define:

- the full schema, ideally as JSON Schema committed beside the code, with the required and optional fields,
- `position` as integers or floats (grid is 16 px; alt-free drags are not on the grid),
- node and edge id allocation (monotonic counter stored in the file, or the next free `nK`), and that ids are never reused inside a session so undo stays valid,
- "stable key order": which order (field order of the struct, or sorted), number formatting, trailing newline, indentation,
- mark fields per kind: `size` for rectangle, circle, and note; `action` and `target` for a button; z-order (the docs have bring forward and send back, but no stored order); `stroke` for the optional rectangle hairline,
- paths: "relative to the `.pp` when they can be" needs the rule for an untitled document, Save As to another folder (rewrite relative paths), and a path on another drive,
- unknown fields: kept on save, dropped, or an error (recommend kept, for forward compatibility within version 1),
- parameter changes between app versions, if a feature's params change in v1.x.

### G11. Undo and dirty state (TRD for M2)

The rules are clear about granularity. Missing: a stack depth limit, redo invalidation, the clean point (save marks the stack position; undo back to it clears the dirty dot), and whether camera moves and selection are in the stack (recommend not).

### G12. Canvas details (TRD for M2)

- Zoom range (min and max) and step per wheel notch. Plain wheel without Ctrl: scroll the canvas, or nothing? Touchpad pinch and two-finger scroll on Linux.
- Hit order for marks. Rectangles paint behind wires. A click inside a rectangle that frames a branch: does it select the rectangle, or start a marquee? Recommend: a rectangle is hit only on its edge or its label, or only when nothing else is under the pointer and the click is not a drag.
- Marquee: fully inside, or touching? Does it select marks and wires?
- Tool shortcut keys. "The tool keys" are mentioned and never listed.
- `Delete` as well as `Backspace`. Linux users expect both.
- Ctrl+Shift+Z versus Shift+Z: the docs write "Ctrl+Z / Shift+Z", which can read as Shift+Z alone. Also Ctrl+Y.
- Fit with an empty graph.
- Card width in px, port row height, the port hit size ("ports are large" needs a number).
- Wire control point distance (a function of horizontal distance, with a minimum, so a wire to the left of its source still reads).

### G13. Status and stale propagation (TRD)

The card dots are defined. The transitions are not: which actions mark which nodes stale, whether a stale node keeps its last preview in the table, and what `blocked` shows in the table when the selected node's ancestor failed. A small state table per node would settle it.

### G14. Process and environment (TRD for M3)

- How `ompp` finds the worker and the venv in M3: a path relative to the repo, an environment variable such as `OMPP_PYTHON`, or both. What it prints when the interpreter or pandas is missing or the wrong version.
- Minimum Python version.
- Reader dependencies and their pins: `openpyxl` (xlsx), `pyarrow` (parquet, and pandas 3's default string dtype), `lxml` (xml, html), `html5lib` or `bs4` (html). Which ones M3 requires.
- Worker crash: restart policy, how the window reports it, and where stderr is logged (a file under `$XDG_STATE_HOME/ohmypipeline`?).
- Network access: pandas readers accept URLs. Decide whether a path parameter may be a URL.
- Pandas 3 copy-on-write is on by default. The cache depends on that: a cached frame handed to the next node is not mutated. State it as an assumption and test it.
- Cache key: "the node's record and its parents' keys" misses the mode (preview and full must not share entries) and the source file's identity (mtime and size, or a hash), so editing the CSV on disk invalidates the read.

### G15. CLI edge cases (TRD with C7)

- Output without `--output`: CSV to stdout. Is the index written (`to_csv(index=...)`)? DESIGN's preview pins the index, so the CSV choice should be explicit.
- A scalar or a series as the chosen node's output.
- A chart node or a writer as the only sink: the default `--node` rule picks it, and it has no CSV.
- Exit codes for an unreadable or invalid `.pp`, an unknown `version`, and a missing worker. Only 0, 2, and 3 are defined.
- Preview is the CLI default, so a plain run emits at most 10,000 source rows. Say whether the CLI prints a notice on stderr.

### G16. Leftover terms (TRD)

- "Example mode" (Chart) and "example" (Shell, muted text) are not defined anywhere. Either define them or delete them.
- The Shell sketch shows a `[select]` node. There is no `select` feature.
- Preview's "Add as node" makes a "Filter node" and a "Sort node". The features are `query` and `sort_values`; pandas `filter` selects labels, not rows. Also define how a DataTable text search becomes a `query` expression, since a substring search over every column is not one.

## 3. Non-functional requirements the TRD needs as numbers

None of these appear today. Each module will otherwise pick its own.

| Area | Needs |
| --- | --- |
| Canvas | Frame time target at N nodes and M wires (for example, 120 nodes at 60 fps on the target machine). Maximum nodes before the TRD stops promising. |
| Worker | Cold start time budget (Python and pandas import is often over a second), and whether it is spawned at launch or on first Run. |
| Preview | Reply size cap in bytes, max columns, max cell length. Target latency for the 10,000-row `read_csv` → `head` → `change_headers` on the reference machine. |
| Memory | What happens when a full run exceeds RAM: the worker dies, the window reports it. |
| Files | Largest `.pp` the editor must open. |
| Target machine | Name the reference aarch64 box, its GPU and driver, X11 or Wayland, and the window manager. "A Linux GPUI window needs a session and a Vulkan driver" is a precondition, not a test bench. |

## 4. Platform risks to retire with spikes

Run these before the module design is frozen. Each is a day or less, and each can change a module boundary.

| # | Spike | Why |
| --- | --- | --- |
| P1 | Pin GPUI and GPUI Kit to exact revisions, build an empty window on the target aarch64 machine under the chosen display server. | GPUI is pre-1.0, Linux support is newer than macOS. M2 depends on it. |
| P2 | Confirm the APIs the PRD names exist at that pin: `canvas` element, `paint_path` / `PathBuilder`, element bounds available in prepaint, DataTable virtualization with a pinned column, Dock persistence, the Plot scales and shapes, Plot hover. | The PRD's paint/hit split assumes "prepaint stores port bounds on the canvas entity". If that is not possible, `canvas` changes shape. |
| P3 | Alt-drag and Space under the target window manager. | Xfce, KDE, and many tiling WMs take Alt-drag to move windows. The event never reaches the app. |
| P4 | CI: GitHub's `ubuntu-24.04-arm` runner for `graph` tests and the worker; Mesa lavapipe (software Vulkan) for a windowed smoke test. | The PRD says a loader-only machine does not count. Decide whether lavapipe counts for CI, or whether windowed checks stay manual. |
| P5 | Worker round trip: spawn Python, import pandas, read 10,000 rows of a wide CSV, encode the reply, decode in Rust. Measure. | Sets the numbers in section 3, and tells whether JSON is fast enough or the rows need a binary encoding (Arrow IPC) behind the same framing. |
| P6 | Python `ast` signature parse for `run(...)`, round trip through the worker while idle. | G8. |

## 5. Module design seed

Not a decision; a starting point that satisfies the PRD's layers and the gaps above. It shows where each contract lives, so section 2 can be assigned an owner.

```text
catalog/features.json        G1. Single source. Read by graph (include_str!) and the worker.
protocol/vectors/            G4. Request/reply pairs. Tested from Rust and Python.
schema/pp-v1.json            G10. JSON Schema of the file.

crates/graph/                no GPUI, no IO except the file codec
  model      FeatureDef, PortDef, ParamDef, Type, Node, Edge, Mark, Graph, ids
  catalog    loads features.json, lookup by id, output type rules (G2)
  types      resolve types through the graph, unknown, mismatch
  connect    accept / reject: type, full, duplicate, cycle; snap selection given distances
  edit       commands as record replacements; undo stack; clean point (G11)
  file       .pp read / write, version check, error nodes, relative paths, recovery file
  status     per-node state machine (G13)

crates/worker-client/        no GPUI. Spawn, framing, generation, kill, restart (G4, G14)
                             Shared by app and the CLI. Could be a module of graph if it
                             does not pull in an async runtime; decide in the TRD.

crates/canvas/               graph + GPUI Kit
  camera     canvas <-> window transform, zoom toward pointer, fit
  hit        hit order from the model, in canvas space
  gesture    tools, drag state machine (link, detach, move, marquee, pan, place mark)
  card       FeatureCard, Port elements with stable ids
  wires      WireLayer paint from bounds stored in prepaint
  marks      mark elements and paint order

crates/app/                  window, Dock, menus, actions, shortcuts, settings
  inspector  param editors, wireable toggle, code editor, errors under fields
  preview    DataTable over the reply window, pinned index, status line
  chart      Chart tab, Plot / Chart per type, title and legend chrome
  document   open, save, dirty, recovery, read-only

crates/ompp/                 argv: GUI, or headless run through worker-client (C7, G15)

worker/ompp_worker/          Python
  protocol   framing, message kinds, value encoding
  runner     topological run, cache, mode, kill-safe
  readers    preview cap strategy per reader (G7)
  features   one runner per catalog id, asserted against features.json
  code       python block execution, signature parse (G8)
```

Ownership rule to keep: only `graph/file` knows the `.pp` layout, only `worker-client` and `worker/protocol` know the wire format, and only `canvas` converts between canvas and window space.

## 6. Suggested TRD outline

1. Scope: M1–M6, platform, reference machine, what is excluded.
2. Architecture: layers, crates, process model, threads (UI, worker I/O task).
3. Data model and catalog format (G1, G2, G3).
4. `.pp` format v1 with JSON Schema (G10, C8, C9).
5. Connection and type rules (C3, C4, G2).
6. Canvas: spaces, gestures, hit and paint order, numbers (G12, C10).
7. Editing, undo, dirty, recovery (G11).
8. Worker protocol with vectors (G4, C12).
9. Execution: triggers, cache, modes, readers, writers, status (G5, G6, G7, G13, G14).
10. Code blocks and expression security (G8).
11. Preview table and status claims.
12. Chart (C11, G9).
13. CLI (C7, G15).
14. Non-functional numbers (section 3).
15. Test plan: per milestone check, protocol vectors, golden `.pp` files, CI matrix (P4).
16. Dependencies and pins: GPUI, GPUI Kit, Python, pandas, reader libraries.
17. Open questions left after the spikes.
