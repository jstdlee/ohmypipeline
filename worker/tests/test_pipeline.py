"""The first pipeline, the preview cap, and the rules the worker must not invent."""

import io
import json
import sqlite3
import unittest
from pathlib import Path

from pipeline.protocol import read_frame, write_frame
from pipeline.run import PREVIEW_ROW_CAP, run_job, stats


def graph_of(nodes, edges=None, marks=None):
    return {
        "format": "ohmypipeline",
        "version": 1,
        "nodes": nodes,
        "edges": edges or [],
        "marks": marks or [],
        "camera": {"x": 0, "y": 0, "zoom": 1},
    }


def node(node_id, feature, params=None, **extra):
    record = {
        "id": node_id,
        "type": feature,
        "label": feature,
        "position": [0, 0],
        "params": params or {},
    }
    record.update(extra)
    return record


def edge(src, dst, src_port="out", dst_port="in"):
    return {"from": src, "from_port": src_port, "to": dst, "to_port": dst_port}


class PipelineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)

    def test_read_head_change_headers(self):
        csv_path = self.tmp / "orders.csv"
        csv_path.write_text(
            "amount,region,month\n12.40,N,2024-01\n8.10,S,2024-01\n15.00,N,2024-02\n3.25,E,2024-02\n",
            encoding="utf-8",
        )
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node("n2", "head", {"n": 3}),
                node(
                    "n3",
                    "change_headers",
                    {"mapping": [{"from": "amount", "to": "revenue"}, {"from": "region", "to": ""}]},
                ),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
            marks=[{"id": "m1", "kind": "note", "text": "ignored", "position": [0, 0], "swatch": "c3"}],
        )
        reply = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": document,
                "node": "n3",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(reply["error"])
        self.assertEqual(reply["claim"], "preview")
        self.assertEqual(reply["columns"], ["revenue", "region", "month"])
        self.assertEqual(reply["n"], 3)
        self.assertEqual(reply["rows"][0][0], 12.4)
        self.assertEqual(reply["index"][0], "0")
        self.assertNotIn("null", json.dumps(reply["rows"]))

    def test_preview_caps_the_reader_and_full_does_not(self):
        csv_path = self.tmp / "wide.csv"
        with csv_path.open("w", encoding="utf-8") as handle:
            handle.write("amount,region\n")
            for i in range(PREVIEW_ROW_CAP + 50):
                handle.write(f"{i},N\n")
        document = graph_of([node("n1", "read_csv", {"path": str(csv_path)})])
        preview = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        full = run_job({"generation": 2, "mode": "full", "graph": document, "node": "n1", "window": 10})
        self.assertIsNone(preview["error"])
        self.assertEqual(preview["n"], PREVIEW_ROW_CAP)
        self.assertEqual(len(preview["rows"]), PREVIEW_ROW_CAP)
        self.assertEqual(full["n"], PREVIEW_ROW_CAP + 50)
        self.assertEqual(len(full["rows"]), 10)
        self.assertEqual(full["claim"], "full")

    def test_a_user_head_survives_a_full_run(self):
        csv_path = self.tmp / "small.csv"
        csv_path.write_text("amount\n1\n2\n3\n4\n", encoding="utf-8")
        document = graph_of(
            [node("n1", "read_csv", {"path": str(csv_path)}), node("n2", "head", {"n": 2})],
            [edge("n1", "n2")],
        )
        reply = run_job({"generation": 1, "mode": "full", "graph": document, "node": "n2"})
        self.assertEqual(reply["n"], 2)

    def test_series_wired_into_change_headers_names_both_types(self):
        csv_path = self.tmp / "cols.csv"
        csv_path.write_text("amount,region\n1,N\n", encoding="utf-8")
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node("n2", "column", {"name": "amount"}),
                node("n3", "change_headers", {"mapping": []}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n3"})
        self.assertIsNotNone(reply["error"])
        self.assertEqual(reply["error"]["node"], "n3")
        self.assertEqual(reply["error"]["declared"], "dataframe")
        self.assertEqual(reply["error"]["actual"], "series")
        self.assertIn("dataframe", reply["error"]["message"])
        self.assertIn("series", reply["error"]["message"])

    def test_missing_column_is_named_and_not_retargeted(self):
        csv_path = self.tmp / "renamed.csv"
        csv_path.write_text("amount,region\n1,N\n", encoding="utf-8")
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node("n2", "change_headers", {"mapping": [{"from": "amount", "to": "revenue"}]}),
                node("n3", "column", {"name": "amount"}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n3"})
        self.assertIn("missing column amount", reply["error"]["message"])

    def test_duplicate_headers_are_an_error(self):
        csv_path = self.tmp / "dup.csv"
        csv_path.write_text("a,b\n1,2\n", encoding="utf-8")
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node(
                    "n2",
                    "change_headers",
                    {"mapping": [{"from": "a", "to": "x"}, {"from": "b", "to": "x"}]},
                ),
            ],
            [edge("n1", "n2")],
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n2"})
        self.assertIn("duplicate header x", reply["error"]["message"])

    def test_query_and_python_and_literal(self):
        csv_path = self.tmp / "q.csv"
        csv_path.write_text("amount,region\n12,N\n4,S\n9,N\n", encoding="utf-8")
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node("n2", "query", {"expr": "region == 'N'"}),
                node("n4", "literal", {"kind": "scalar", "value": "5"}),
                node(
                    "n3",
                    "python",
                    {"code": "def run(df, amount):\n    return df[df['amount'] > amount]\n"},
                    inputs=[
                        {"name": "df", "accepts": ["dataframe"]},
                        {"name": "amount", "accepts": ["scalar"]},
                    ],
                    output={"name": "out", "accepts": ["dataframe"]},
                ),
            ],
            [
                edge("n1", "n2"),
                edge("n2", "n3", dst_port="df"),
                edge("n4", "n3", dst_port="amount"),
            ],
        )
        reply = run_job({"generation": 4, "mode": "preview", "graph": document, "node": "n3"})
        self.assertIsNone(reply["error"], reply.get("error"))
        self.assertEqual(reply["n"], 2)
        self.assertTrue(all(row[0] > 5 for row in reply["rows"]))

    def test_marks_do_not_change_the_result(self):
        csv_path = self.tmp / "marks.csv"
        csv_path.write_text("amount\n1\n2\n", encoding="utf-8")
        bare = graph_of([node("n1", "read_csv", {"path": str(csv_path)})])
        marked = graph_of(
            bare["nodes"],
            marks=[{"id": "m1", "kind": "rectangle", "text": "", "position": [0, 0], "size": [10, 10], "swatch": "c1"}],
        )
        a = run_job({"generation": 1, "mode": "preview", "graph": bare, "node": "n1"})
        b = run_job({"generation": 1, "mode": "preview", "graph": marked, "node": "n1"})
        self.assertEqual(a["rows"], b["rows"])
        self.assertEqual(a["n"], b["n"])

    def test_a_cycle_is_refused(self):
        document = graph_of(
            [node("n1", "head", {"n": 1}), node("n2", "head", {"n": 1})],
            [edge("n1", "n2"), edge("n2", "n1")],
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIn("cycle", reply["error"]["message"])

    def test_sqlite_preview_uses_limit(self):
        db = self.tmp / "t.sqlite"
        if db.exists():
            db.unlink()
        with sqlite3.connect(db) as con:
            con.execute("create table orders (amount real, region text)")
            con.executemany("insert into orders values (?, ?)", [(i, "N") for i in range(30)])
        document = graph_of([node("n1", "read_sql", {"path": str(db), "table": "orders"})])
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply["error"], reply.get("error"))
        self.assertEqual(reply["n"], 30)
        self.assertEqual(reply["columns"], ["amount", "region"])

    def test_cache_skips_an_unchanged_ancestor(self):
        csv_path = self.tmp / "cache.csv"
        csv_path.write_text("amount\n1\n2\n3\n", encoding="utf-8")
        document = graph_of(
            [node("n1", "read_csv", {"path": str(csv_path)}), node("n2", "head", {"n": 1})],
            [edge("n1", "n2")],
        )
        before = stats["computed"]
        run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n2"})
        mid = stats["computed"]
        run_job({"generation": 2, "mode": "preview", "graph": document, "node": "n2"})
        after = stats["computed"]
        self.assertGreater(mid, before)
        self.assertEqual(after, mid)

    def test_frames_round_trip_on_the_stdio_protocol(self):
        buf = io.BytesIO()
        write_frame(buf, {"generation": 7, "mode": "preview"})
        buf.seek(0)
        self.assertEqual(read_frame(buf), {"generation": 7, "mode": "preview"})

    def test_null_cells_are_json_null(self):
        csv_path = self.tmp / "nulls.csv"
        csv_path.write_text("amount,region\n1,\n,N\n", encoding="utf-8")
        document = graph_of([node("n1", "read_csv", {"path": str(csv_path)})])
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply["rows"][0][1])
        self.assertIsNone(reply["rows"][1][0])

    def test_python_named_input_receives_the_frame(self):
        csv_path = self.tmp / "named.csv"
        csv_path.write_text("amount\n1\n2\n3\n", encoding="utf-8")
        code = "def run(frame):\n    return frame.head(1)\n"
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node(
                    "n2",
                    "python",
                    {"code": code},
                    inputs=[{"name": "frame", "accepts": ["dataframe"], "multi": False}],
                ),
            ],
            [edge("n1", "n2", dst_port="frame")],
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n2"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 1)
        self.assertEqual(reply["rows"][0][0], 1)


    def test_csv_skiprows_drops_the_preamble(self):
        csv_path = self.tmp / "preamble.csv"
        csv_path.write_text("export\nnotes\nregion,qty\nN,2\nS,3\n", encoding="utf-8")
        document = graph_of(
            [
                node(
                    "n1",
                    "read_csv",
                    {"path": str(csv_path), "skip_rows": 2, "header": "header row", "header_row": 0},
                )
            ]
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"], ["region", "qty"])
        self.assertEqual(reply["rows"][0][0], "N")
        self.assertEqual(reply["rows"][0][1], 2)
        self.assertEqual(reply["n"], 2)

    def test_csv_header_row_is_not_the_first_line(self):
        csv_path = self.tmp / "header_row.csv"
        csv_path.write_text("title\nregion,qty\nN,2\n", encoding="utf-8")
        document = graph_of(
            [
                node(
                    "n1",
                    "read_csv",
                    {"path": str(csv_path), "skip_rows": 0, "header": "header row", "header_row": 1},
                )
            ]
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"], ["region", "qty"])
        self.assertEqual(reply["rows"][0][0], "N")
        self.assertNotIn("title", reply["columns"])

    def test_sqlite_skip_rows_offsets_the_table(self):
        db = self.tmp / "skip.sqlite"
        if db.exists():
            db.unlink()
        with sqlite3.connect(db) as con:
            con.execute("create table orders (amount real)")
            con.executemany("insert into orders values (?)", [(0,), (1,), (2,)])
        document = graph_of(
            [node("n1", "read_sql", {"path": str(db), "table": "orders", "skip_rows": 1})]
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["rows"][0][0], 1)
        self.assertEqual(reply["n"], 2)

    def test_excel_header_row_skips_the_title(self):
        from openpyxl import Workbook

        path = self.tmp / "sales.xlsx"
        book = Workbook()
        sheet = book.active
        sheet.append(["junk"])
        sheet.append(["region", "qty"])
        sheet.append(["N", 2])
        book.save(path)
        document = graph_of(
            [
                node(
                    "n1",
                    "read_excel",
                    {
                        "path": str(path),
                        "skip_rows": 0,
                        "header": "header row",
                        "header_row": 1,
                    },
                )
            ]
        )
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"], ["region", "qty"])
        self.assertEqual(reply["rows"][0][0], "N")

    def test_pivot_sums_values_by_index_and_column(self):
        csv_path = self.tmp / "pivot.csv"
        csv_path.write_text("region,product,qty\nN,Pen,2\nN,Pen,1\nS,Book,4\n", encoding="utf-8")
        document = graph_of(
            [
                node("n1", "read_csv", {"path": str(csv_path)}),
                node(
                    "n2",
                    "pivot",
                    {"index": "region", "columns": "product", "values": "qty", "agg": "sum"},
                ),
                node("n3", "preview_table"),
                node("n4", "chart", {"mark": "line", "x": "region", "y": "Pen"}),
            ],
            [edge("n1", "n2"), edge("n2", "n3"), edge("n2", "n4")],
        )
        pivot = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n2"})
        self.assertIsNone(pivot.get("error"), pivot.get("error"))
        self.assertIn("region", pivot["columns"])
        self.assertIn("Pen", pivot["columns"])
        self.assertIn("Book", pivot["columns"])
        pen = pivot["columns"].index("Pen")
        north = next(row for row in pivot["rows"] if row[pivot["columns"].index("region")] == "N")
        self.assertEqual(north[pen], 3)
        table = run_job({"generation": 2, "mode": "preview", "graph": document, "node": "n3"})
        self.assertEqual(table["rows"], pivot["rows"])
        chart = run_job({"generation": 3, "mode": "preview", "graph": document, "node": "n4"})
        self.assertEqual(chart["columns"], pivot["columns"])
        self.assertIsNone(chart.get("error"), chart.get("error"))


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)
        self.csv = self.tmp / "people.csv"
        self.csv.write_text(
            "name,city,qty,when,note\nAda,North,10,2024-01-15, hello\nBea,South,4,2024-03-02,Hello\nCal,North,,2024-06-20,  hi\nDee,East,7,2024-08-01,Hi there\n",
            encoding="utf-8",
        )

    def job(self, nodes, edges, node_id):
        reply = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(nodes, edges),
                "node": node_id,
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        return reply

    def reader(self, node_id="n1"):
        return node(node_id, "read_csv", {"path": str(self.csv)})

    def test_str_lower_and_strip_on_a_column(self):
        lowered = self.job(
            [self.reader(), node("n2", "str", {"method": "lower", "column": "name"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(lowered["rows"][0][lowered["columns"].index("name")], "ada")
        stripped = self.job(
            [self.reader(), node("n2", "str", {"method": "strip", "column": "note"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(stripped["rows"][0][stripped["columns"].index("note")], "hello")

    def test_str_contains_split_and_extract(self):
        contains = self.job(
            [self.reader(), node("n2", "str", {"method": "contains", "column": "note", "pat": "Hi"})],
            [edge("n1", "n2")],
            "n2",
        )
        flags = [row[contains["columns"].index("note")] for row in contains["rows"]]
        self.assertEqual(flags, [False, False, False, True])
        split = self.job(
            [
                self.reader(),
                node("n2", "str", {"method": "split", "column": "note", "pat": " "}),
                node("n3", "explode", {"column": "note"}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertGreater(split["n"], 4)
        extracted = self.job(
            [
                node("s", "column", {"name": "name"}),
                self.reader(),
                node("n2", "str", {"method": "extract", "pat": "([A-Z])"}),
            ],
            [edge("n1", "s"), edge("s", "n2")],
            "n2",
        )
        self.assertIn("0", extracted["columns"])
        self.assertEqual(extracted["rows"][0][0], "A")

    def test_datetime_accessors_and_to_numeric(self):
        dated = self.job(
            [
                self.reader(),
                node("n2", "to_datetime", {"column": "when"}),
                node("n3", "dt", {"accessor": "year", "column": "when"}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertEqual(dated["rows"][0][dated["columns"].index("when")], 2024)
        numeric = self.job(
            [
                node("n1", "read_csv", {"path": str(self.csv)}),
                node("n2", "column", {"name": "qty"}),
                node("n3", "to_numeric", {}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertEqual(numeric["rows"][0][0], 10)

    def test_shift_isin_between_loc_and_iloc(self):
        shifted = self.job(
            [self.reader(), node("n2", "shift", {"n": 1})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIsNone(shifted["rows"][0][shifted["columns"].index("qty")])
        kept = self.job(
            [self.reader(), node("n2", "isin", {"column": "city", "values": "North,East"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(kept["n"], 3)
        span = self.job(
            [
                self.reader(),
                node("n2", "between", {"column": "qty", "lower": "5", "upper": "10", "inclusive": "both"}),
            ],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(span["n"], 2)
        located = self.job(
            [self.reader(), node("n2", "loc", {"rows": "0:1"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(located["n"], 2)
        positioned = self.job(
            [self.reader(), node("n2", "iloc", {"rows": "1:3"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(positioned["rows"][0][positioned["columns"].index("name")], "Bea")
        self.assertEqual(positioned["n"], 2)

    def test_ffill_clip_and_where(self):
        filled = self.job(
            [self.reader(), node("n2", "ffill", {}), node("n3", "clip", {"lower": "5", "upper": "9"})],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        qty = filled["columns"].index("qty")
        self.assertEqual([row[qty] for row in filled["rows"]], [9, 5, 5, 7])
        masked = self.job(
            [self.reader(), node("n2", "where", {"mode": "mask", "cond": "qty > 8", "other": "0"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(masked["rows"][0][masked["columns"].index("qty")], 0)

    def test_stack_unstack_transpose_and_crosstab(self):
        stacked = self.job(
            [self.reader(), node("n2", "stack", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(stacked["n"], 20)
        wide = self.job(
            [self.reader(), node("n2", "stack", {}), node("n3", "unstack", {})],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertGreaterEqual(len(wide["columns"]), 2)
        flipped = self.job(
            [self.reader(), node("n2", "transpose", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(flipped["n"], 5)
        counted = self.job(
            [self.reader(), node("n2", "crosstab", {"index": "city", "columns": "name"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("city", counted["columns"])
        self.assertIn("Ada", counted["columns"])

    def test_rolling_diff_groupby_and_reducers(self):
        rolled = self.job(
            [self.reader(), node("n2", "rolling", {"window": 2, "agg": "mean", "column": "qty"})],
            [edge("n1", "n2")],
            "n2",
        )
        qty = rolled["columns"].index("qty")
        self.assertEqual(rolled["rows"][0][qty], 10)
        self.assertEqual(rolled["rows"][1][qty], 7)
        changed = self.job(
            [self.reader(), node("n2", "diff", {"periods": 1})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(changed["rows"][1][changed["columns"].index("qty")], -6)
        grouped = self.job(
            [self.reader(), node("n2", "groupby", {"by": "city", "agg": "sum", "aggs": "qty:sum, qty:mean"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("city", grouped["columns"])
        self.assertTrue(any("sum" in column for column in grouped["columns"]))
        unique = self.job(
            [self.reader(), node("n2", "nunique", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(unique["rows"][unique["index"].index("city")][0], 3)

    def test_writers_require_a_path_and_write_the_frame(self):
        missing = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(
                    [self.reader(), node("n2", "to_csv", {"path": ""})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIn("no path", missing["error"]["message"])
        csv_path = self.tmp / "out.csv"
        written = self.job(
            [self.reader(), node("n2", "to_csv", {"path": str(csv_path), "index": "no"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(written["n"], 4)
        self.assertIn("name,city,qty,when,note", csv_path.read_text(encoding="utf-8").splitlines()[0])
        excel = self.tmp / "out.xlsx"
        self.job(
            [self.reader(), node("n2", "to_excel", {"path": str(excel), "sheet": "People", "index": "no"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertTrue(excel.is_file())
        blob = self.tmp / "out.json"
        self.job(
            [self.reader(), node("n2", "to_json", {"path": str(blob), "orient": "records"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("Ada", blob.read_text(encoding="utf-8"))
        parquet = self.tmp / "out.parquet"
        self.job(
            [self.reader(), node("n2", "to_parquet", {"path": str(parquet), "index": "no"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertTrue(parquet.is_file())
        db = self.tmp / "out.sqlite"
        self.job(
            [
                self.reader(),
                node("n2", "to_sql", {"path": str(db), "table": "people", "if_exists": "replace", "index": "no"}),
            ],
            [edge("n1", "n2")],
            "n2",
        )
        with sqlite3.connect(db) as con:
            count = con.execute("select count(*) from people").fetchone()[0]
        self.assertEqual(count, 4)

    def test_pct_change_cumsum_bfill_interpolate_and_median(self):
        changed = self.job(
            [self.reader(), node("n2", "pct_change", {"periods": 1})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIsNone(changed["rows"][0][changed["columns"].index("qty")])
        total = self.job(
            [self.reader(), node("n2", "cumsum", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(total["rows"][1][total["columns"].index("qty")], 14)
        filled = self.job(
            [self.reader(), node("n2", "bfill", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(filled["rows"][2][filled["columns"].index("qty")], 7)
        smooth = self.job(
            [self.reader(), node("n2", "interpolate", {"method": "linear"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(smooth["rows"][2][smooth["columns"].index("qty")], 5.5)
        middle = self.job(
            [self.reader(), node("n2", "median", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(middle["rows"][middle["index"].index("qty")][0], 7)

    def test_join_and_merge_asof(self):
        right = self.tmp / "managers.csv"
        right.write_text("city,manager\nNorth,Ada\nSouth,Bea\nEast,Cal\n", encoding="utf-8")
        joined = self.job(
            [
                self.reader(),
                node("r", "read_csv", {"path": str(right)}),
                node("n2", "join", {"on": "city", "how": "left"}),
            ],
            [edge("n1", "n2", dst_port="left"), edge("r", "n2", dst_port="right")],
            "n2",
        )
        self.assertIn("manager", joined["columns"])
        self.assertEqual(joined["n"], 4)
        quotes = self.tmp / "quotes.csv"
        quotes.write_text("time,price\n1,10\n4,12\n", encoding="utf-8")
        trades = self.tmp / "trades.csv"
        trades.write_text("time,qty\n1,1\n3,2\n5,3\n", encoding="utf-8")
        merged = self.job(
            [
                node("l", "read_csv", {"path": str(trades)}),
                node("r", "read_csv", {"path": str(quotes)}),
                node("n2", "merge_asof", {"on": "time", "direction": "backward"}),
            ],
            [edge("l", "n2", dst_port="left"), edge("r", "n2", dst_port="right")],
            "n2",
        )
        price = merged["columns"].index("price")
        self.assertEqual(merged["rows"][1][price], 10)
        self.assertEqual(merged["rows"][2][price], 12)


if __name__ == "__main__":
    unittest.main()
