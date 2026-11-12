# Interface review

Four passes over the running window, using the Ryugi interface-craft checks (spacing, color, type, layout, then motion) and the verifying-ui rule that a claim needs a capture from the app. Ryugi was not run as a tool. Apple's Human Interface Guidelines were used for hit area, labels, and motion: primary actions stay obvious, settings say what they store, and motion can be interrupted and skipped when reduced motion is on.

## Round 1

The preview dock stayed on "Running preview" after the worker had already returned rows. The chart tab could draw those rows because it repainted; the preview panel did not.

Accepted: the dock must show the table for the same reply the chart uses.

Changed: after a reply, the window is notified, and the preview panel notifies itself when the session changes.

Checked: `docs/screenshots/preview.png` shows `preview · at most 10,000 source rows · 5 rows` and the columns revenue, region, and month.

## Round 2

The model settings panel sat on top of the inspector, and its three fields had no labels. "Columns in the file" was a heading with no controls.

Accepted: a setting needs a name, and a column list is a set of buttons.

Changed: the panel sits on the canvas, with the labels Endpoint, Model, and Key. Each upstream column is a button that fills an empty From cell. The inspector shows the prompt when no endpoint or key is saved.

Checked: `docs/screenshots/assist.png`.

## Round 3

Rectangle, circle, and label could be stored and drawn, but the toolbar only offered Note, so those tools could not be chosen.

Accepted: a mark that the canvas paints needs a tool.

Changed: Rectangle, Circle, and Label sit in the canvas tool group with Select, Hand, and Note.

Checked: `docs/screenshots/marks.png` shows the rectangle, the circle, and the label "Revenue is the amount column".

## Round 4

Fitting the graph used a 48px margin, so the last card could sit on the canvas edge. That showed up when a Python node was added at the end of a chain.

Accepted: the opened graph, including a newly added card, should sit inside the canvas.

Changed: fit uses a 72px margin.

`docs/screenshots/python.png` and `docs/screenshots/textarea.png` show the code block, the growing code field, Pretty, and the named input `frame` with a type row. Those stills were taken before the margin change. The 72px margin is in the fit code so a later open keeps the last card inside the canvas.

## Output nodes

Four passes on the sales graph: a title row, a pivot, a preview table, a line plot, and a pie plot. Ryugi was not run as a tool.

### Round 1

The chart tab stayed on "Run preview to draw this chart" after the preview dock already had rows. The cards on the graph had updated, because the canvas was told about the frame. The chart panel was not.

Accepted: the chart tab and the plot card draw the same reply.

Changed: the chart panel redraws when the session changes.

Checked: `docs/screenshots/pie.png` and `docs/screenshots/line.png`.

### Round 2

Column buttons on the pivot were the first line of the file, "Weekly export", which is the title row the reader skips.

Accepted: a column button is a column the node can actually use.

Changed: header sniffing skips the same preamble and header row the reader uses. After a preview, a plot's buttons are the columns in that frame, so the line plot offers region, Book, and Pen.

Checked: `docs/screenshots/outputs.png` (pivot buttons are region, product, qty, price) and `docs/screenshots/line.png` (X and Y offer region, Book, Pen).

### Round 3

The line and the pie have to be readable on the card and in the chart tab: axes and a hover tooltip for the line, slice labels and a legend for the pie.

Accepted: the chart tab is the large drawing. The card is the same mark, smaller.

Changed: both use GPUI Kit line and pie charts. The legend is a swatch plus the name, under the plot.

Checked: `docs/screenshots/line.png` shows the axes, the Pen legend, and the hover "North / Pen 10". `docs/screenshots/pie.png` shows East, North, and South as slices with labels and a legend. `docs/screenshots/outputs.png` shows those same marks on the cards, next to the preview table.

### Round 4

Skip rows and header row need to be visible while the preview proves the title line is gone.

Accepted: the inspector names the controls, and the table does not contain the preamble.

Checked: `docs/screenshots/skip.png`. Skip rows is 1, Header is "header row", Header row is 0, and the preview columns are region, product, qty, and price. The status line is `preview · at most 10,000 source rows · 6 rows`.

## Feature sample

About a quarter of the catalog, mixing nodes that were already shipping with the text, selection, clean, reshape, window, and export batch. Ryugi interface-craft (repair) and verifying-ui were read. Ryugi was not run as a tool. The checks were made on lavapipe screenshots, and this file is the record. Two rounds, then stop.

Sample: Read CSV (IO palette row and card), Is in and Loc (select), Fill forward and Clip (clean), Crosstab (reshape), Text plus To datetime plus Datetime (text and dates), Group by with two aggregations and Rolling (reduce and window), To CSV (export).

### Round 1

- Export inspector, Write index — the help said "Row labels of the pivot" — wrong field, wrong feature.
- Crosstab Rows and Columns — the same pivot sentences, and Values talked about a keep-list.
- Is in, Keep values — the help also mentioned a pivot.
- To CSV, Separator — the box was empty even though the file is comma-separated.

Accepted: a reused parameter name still has to say what it does on this node.

Changed: help is chosen from the feature and the field. The CSV separator defaults to a comma. Fill forward, clip, and loc put a short summary on the card (forward, 5–9, 0:1) instead of only the feature id.

Before: `docs/screenshots/review-before/export.png`, `reshape.png`, `select.png`.

After: `docs/screenshots/export.png`, `reshape.png`, `select.png`.

### Round 2

The three after shots were checked and not changed again.

Export says "Write the row labels as a column." and the separator is a comma. Crosstab says which column becomes rows and which becomes headers. Is in says "Comma-separated values to keep." The Write CSV card says "Wrote 4 rows to ompp-people.csv".

Also checked, and left as they were: `docs/screenshots/text.png` (names lowercased, dates parsed, year 2024), `clean.png` (qty clipped to 9, 5, 5, 7), `window.png` (qty sum and qty mean by city, rolling window 2 on the canvas).

### Left as they are

- On a wide graph the fit zoom shrinks the cards, and a long title such as "Lowercase names" can clip. The text shot still shows that. Card type does not scale with the camera.
- The inspector still prints the feature id under the title (`to_csv`, `crosstab`, `isin`). It is not JSON, and it was not removed in these two rounds.
- The floating tool capsule in the middle of the canvas still reads as a grey bar. It was not part of this fix.
- Text and Output sit below the first screen of the feature list. The list scrolls; these shots stay on the IO rows.

## Canvas and flow v2

Two passes over the new surfaces: the variables panel, the run dialog, loop and branch cards, the zoomed-out title level, and the chart card. Ryugi interface-craft (spacing, color, type, layout, then motion) and verifying-ui were read. Ryugi was not run as a tool. Apple's Human Interface Guidelines were used for hit area, labels, and motion. The checks were made on lavapipe screenshots. No gradients, no glow, no emoji. Theme colors only.

### Round 1

- Head with `n` set to `${rows}` drew `n=5`. The summary asked for an integer and fell back to the default, so the reference never appeared and could not be colored.
- Add column on Loop end was a text box containing the word true.
- The kind control in the variables panel was a plain button. It did not read as a menu until it was opened.
- `--demo big-out` drew each distant card as one clipped glyph, and the camera stopped at zoom 0.25 so the 400 nodes did not fit.
- `--demo zoom` left the graph under the floating toolbar.

Accepted: a variable reference on a card is the text the user typed. A yes/no setting is a choice, not a string. A distant card keeps a title that fits inside it. The opened graph sits in the viewport, clear of the toolbar.

Changed: a numeric summary keeps the `${name}` text when that is the whole value, and the card paints known names in the accent color and unknown names in the danger color. Add column is On / Off. The kind button carries a chevron and opens string, int, float, bool, and path. Title-level type is at most 11px and at most 60% of the card, and it is hidden when the card is under 8px tall. The camera can zoom out to 0.1, and big-out fits the whole graph. The zoom demo centers the graph. Dock resizing updates the view in memory and does not mark the document dirty.

Checked: `docs/screenshots/v2/m5-big-out.png` (small titles, the whole grid, no dirty mark), `m2-zoom.png` (cards centered under the toolbar), `m7-variables.png` (`${data}` and `n=${rows}` on the cards, the panel with used-by counts), `m7-run-dialog.png`, `m8-loop.png` (On selected, eight collected rows), `m8-branch.png` (true and false ports, the true side dimmed), `m2-chart-card.png` (title and legend on the card in the same places as the Chart tab).

### Round 2

The shots above were checked again and not changed again.

The variables panel names each row, says how many nodes use it, and commits on Enter or blur. The run dialog is 420px, lists each variable with its kind, and keeps Run full dataset off until a preview has succeeded. Escape closes it. Loop end names the paired start and does not offer the pair id as an editable field. The inactive branch is lighter, and the wires on that side are thinner. The chart card draws "Plot" at the top of the plot and the amount swatch at the top end, which is the default placement, and a drag on those labels does not move the card.

### Left as they are

- A loop preview still stops after the first three iterations. The worker labels that cap. The loop shot shows the three sample files concatenated, eight rows, with the loop value column filled.
- The condition language is comparisons, `and` / `or` / `not`, `rows`, `columns`, and `${name}`. It does not do arithmetic.
- When a chart has no title of its own, the card repeats the node label inside the plot, the same way the Chart tab does.
- A graph that still cannot fit at zoom 0.1 is centered and clipped at that floor. The 400-node shot fits.
