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
