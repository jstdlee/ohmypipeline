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
