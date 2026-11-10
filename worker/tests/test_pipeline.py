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


class ScaleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)
        self.csv = self.tmp / "scale.csv"
        self.csv.write_text("qty,x,y\n1,1,2\n2,2,4\n3,3,6\n", encoding="utf-8")

    def test_standard_scale_labels_a_preview_fit_and_transform_reuses_it(self):
        read = node("n1", "read_csv", {"path": str(self.csv)})
        fitted = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(
                    [read, node("n2", "standard_scale", {"columns": "qty", "action": "fit_transform"})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(fitted.get("error"), fitted.get("error"))
        self.assertEqual(fitted["fitted"]["source"], "preview")
        self.assertIn("preview", fitted["fitted"]["summary"])
        self.assertAlmostEqual(fitted["rows"][1][fitted["columns"].index("qty")], 0.0)
        changed = self.tmp / "scale2.csv"
        changed.write_text("qty\n1\n2\n3\n10\n", encoding="utf-8")
        transformed = run_job(
            {
                "generation": 2,
                "mode": "full",
                "graph": graph_of(
                    [
                        node("n1", "read_csv", {"path": str(changed)}),
                        node(
                            "n2",
                            "standard_scale",
                            {"columns": "qty", "action": "transform"},
                            fitted=fitted["fitted"],
                        ),
                    ],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(transformed.get("error"), transformed.get("error"))
        self.assertNotIn("fitted", transformed)
        bare = run_job(
            {
                "generation": 3,
                "mode": "preview",
                "graph": graph_of(
                    [node("n1", "read_csv", {"path": str(self.csv)}), node("n2", "minmax_scale", {"action": "transform"})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
            }
        )
        self.assertIn("no fitted parameters", bare["error"]["message"])

    def test_ols_writes_fitted_values_and_coefficients(self):
        read = node("n1", "read_csv", {"path": str(self.csv)})
        reply = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(
                    [read, node("n2", "ols", {"y": "y", "x": "x", "residuals": "yes", "action": "fit_transform"})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertAlmostEqual(reply["fitted"]["coefficients"]["x"], 2.0, places=4)
        self.assertIn("y_fitted", reply["columns"])
        self.assertIn("y_residual", reply["columns"])
        self.assertAlmostEqual(reply["rows"][0][reply["columns"].index("y_residual")], 0.0, places=4)

    def test_calculate_assigns_columns_and_rejects_other_syntax(self):
        read = node("n1", "read_csv", {"path": str(self.csv)})
        reply = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(
                    [read, node("n2", "calculate", {"expr": "double = qty * 2\nsum = qty + x"})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["rows"][0][reply["columns"].index("double")], 2)
        bad = run_job(
            {
                "generation": 2,
                "mode": "preview",
                "graph": graph_of(
                    [read, node("n2", "calculate", {"expr": "bad = qty + missing"})],
                    [edge("n1", "n2")],
                ),
                "node": "n2",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIn("unknown column missing", bad["error"]["message"])


class MoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)

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

    def rejected(self, nodes, edges, node_id):
        return run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of(nodes, edges),
                "node": node_id,
                "base_dir": str(self.tmp),
            }
        )

    def test_resample_asfreq_expanding_ewm_and_dates(self):
        path = self.tmp / "times.csv"
        path.write_text("when,qty\n2024-01-01,1\n2024-01-02,3\n2024-01-08,4\n", encoding="utf-8")
        read = node("n1", "read_csv", {"path": str(path)})
        weekly = self.job(
            [read, node("n2", "resample", {"column": "when", "freq": "W", "agg": "sum", "fill": "none"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertGreaterEqual(weekly["n"], 2)
        self.assertIn("qty", weekly["columns"])
        dense = self.job(
            [read, node("n2", "asfreq", {"column": "when", "freq": "D", "fill": "ffill"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertGreater(dense["n"], 3)
        expanded = self.job(
            [read, node("n2", "expanding", {"window": 1, "agg": "sum", "column": "qty"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(expanded["rows"][2][expanded["columns"].index("qty")], 8)
        smoothed = self.job(
            [read, node("n2", "ewm", {"span": 2, "agg": "mean", "column": "qty"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(smoothed["n"], 3)
        dates = self.job([node("n1", "date_range", {"start": "2024-01-01", "periods": 3, "freq": "D"})], [], "n1")
        self.assertEqual(dates["n"], 3)
        self.assertTrue(str(dates["rows"][0][0]).startswith("2024-01-01"))
        missing = self.rejected(
            [read, node("n2", "resample", {"column": "when", "freq": "", "agg": "mean"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("frequency", missing["error"]["message"])

    def test_time_zones_and_timedelta(self):
        path = self.tmp / "zones.csv"
        path.write_text("when,qty\n2024-01-01 00:00,1\n2024-01-01 02:00,2\n", encoding="utf-8")
        read = node("n1", "read_csv", {"path": str(path)})
        local = self.job(
            [read, node("n2", "tz_localize", {"column": "when", "tz": "UTC"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("+00:00", local["rows"][0][local["columns"].index("when")])
        converted = self.job(
            [
                read,
                node("n2", "tz_localize", {"column": "when", "tz": "UTC"}),
                node("n3", "tz_convert", {"column": "when", "tz": "US/Eastern"}),
            ],
            [edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertIn("-05:00", converted["rows"][0][converted["columns"].index("when")])
        naive = self.rejected(
            [read, node("n2", "tz_convert", {"column": "when", "tz": "UTC"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("timezone-aware", naive["error"]["message"])
        spans = self.job(
            [read, node("n2", "to_timedelta", {"column": "qty", "unit": "h"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("0 days 01:00:00", spans["rows"][0][spans["columns"].index("qty")])

    def test_cut_dummies_factorize_arith_apply_and_map(self):
        path = self.tmp / "bins.csv"
        path.write_text("name,qty,flag\nAda,1,a\nBea,6,b\nCal,9,a\n", encoding="utf-8")
        read = node("n1", "read_csv", {"path": str(path)})
        bands = self.job(
            [read, node("n2", "cut", {"column": "qty", "bins": "0,5,10"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("5.0", bands["rows"][0][bands["columns"].index("qty")])
        tiles = self.job(
            [read, node("n2", "qcut", {"column": "qty", "q": 2})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(tiles["n"], 3)
        dummies = self.job(
            [read, node("n2", "get_dummies", {"column": "flag"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIn("flag_a", dummies["columns"])
        codes = self.job(
            [read, node("n2", "factorize", {"column": "flag"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(codes["rows"][0][codes["columns"].index("flag")], 0)
        added = self.job(
            [read, node("n2", "arith", {"op": "mul", "value": "2"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(added["rows"][0][added["columns"].index("qty")], 2)
        summed = self.job(
            [read, node("n2", "apply", {"fn": "sum", "axis": "0"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(summed["rows"][summed["index"].index("qty")][0], 16)
        mapped = self.job(
            [
                read,
                node("n2", "map", {"fn": "code", "code": "def run(value):\n    return value\n"}),
            ],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(mapped["rows"][0][mapped["columns"].index("name")], "Ada")
        empty = self.rejected([read, node("n2", "cut", {"column": "qty", "bins": ""})], [edge("n1", "n2")], "n2")
        self.assertIn("bins", empty["error"]["message"])

    def test_rank_corr_unique_reindex_sort_wide_and_combine(self):
        path = self.tmp / "pairs.csv"
        path.write_text("name,x,y\nAda,1,3\nBea,2,2\nCal,3,1\n", encoding="utf-8")
        read = node("n1", "read_csv", {"path": str(path)})
        ranked = self.job(
            [read, node("n2", "rank", {"column": "x", "method": "average"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(ranked["rows"][0][ranked["columns"].index("x")], 1)
        related = self.job([read, node("n2", "corr", {})], [edge("n1", "n2")], "n2")
        self.assertIn("x", related["columns"])
        values = self.job(
            [read, node("n2", "unique", {"column": "name"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(values["n"], 3)
        ordered = self.job(
            [read, node("n2", "sort_index", {"order": "descending", "axis": "0"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(ordered["rows"][0][ordered["columns"].index("name")], "Cal")
        relabeled = self.job(
            [read, node("n2", "reindex", {"columns": "name,y"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(relabeled["columns"], ["name", "y"])
        wide = self.tmp / "wide_long.csv"
        wide.write_text("id,A1,A2\n1,10,12\n2,8,9\n", encoding="utf-8")
        long = self.job(
            [
                node("w", "read_csv", {"path": str(wide)}),
                node("n2", "wide_to_long", {"stubnames": "A", "i": "id", "j": "year", "sep": ""}),
            ],
            [edge("w", "n2")],
            "n2",
        )
        self.assertEqual(long["n"], 4)
        self.assertIn("A", long["columns"])
        left = node("l", "read_csv", {"path": str(path)})
        other = self.tmp / "holes.csv"
        other.write_text("name,x,y\nAda,,9\nBea,5,\n", encoding="utf-8")
        filled = self.job(
            [left, node("r", "read_csv", {"path": str(other)}), node("n2", "combine_first", {})],
            [edge("l", "n2", dst_port="left"), edge("r", "n2", dst_port="right")],
            "n2",
        )
        self.assertEqual(filled["rows"][0][filled["columns"].index("x")], 1)

    def test_groupby_transform_filter_and_quantile(self):
        path = self.tmp / "groups.csv"
        path.write_text("city,qty\nNorth,10\nNorth,2\nSouth,4\n", encoding="utf-8")
        read = node("n1", "read_csv", {"path": str(path)})
        transformed = self.job(
            [read, node("n2", "groupby", {"by": "city", "how": "transform", "agg": "sum", "column": "qty"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(transformed["rows"][0][transformed["columns"].index("qty")], 12)
        self.assertEqual(transformed["n"], 3)
        kept = self.job(
            [
                read,
                node(
                    "n2",
                    "groupby",
                    {"by": "city", "how": "filter", "agg": "sum", "column": "qty", "filter_op": "gt", "filter_value": "5"},
                ),
            ],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(kept["n"], 2)
        half = self.job(
            [read, node("n2", "quantile", {"q": 0.5})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(half["rows"][half["index"].index("qty")][0], 4)

    def test_readers_cap_preview_and_report_a_missing_file(self):
        table = self.tmp / "table.tsv"
        table.write_text("name\tqty\nAda\t1\n", encoding="utf-8")
        read_table = self.job([node("n1", "read_table", {"path": str(table), "sep": "\t"})], [], "n1")
        self.assertEqual(read_table["columns"], ["name", "qty"])
        fixed = self.tmp / "fixed.txt"
        fixed.write_text("Ada     0010\nBea     0004\n", encoding="utf-8")
        fwf = self.job(
            [node("n1", "read_fwf", {"path": str(fixed), "colspecs": "0:8,8:12", "header": "no header"})],
            [],
            "n1",
        )
        self.assertEqual(fwf["n"], 2)
        page = self.tmp / "page.html"
        page.write_text("<table><tr><th>name</th><th>qty</th></tr><tr><td>Ada</td><td>3</td></tr></table>", encoding="utf-8")
        html = self.job([node("n1", "read_html", {"path": str(page), "which": 0})], [], "n1")
        self.assertEqual(html["rows"][0][0], "Ada")
        xml = self.tmp / "rows.xml"
        xml.write_text("<data><row><name>Ada</name><qty>3</qty></row></data>", encoding="utf-8")
        parsed = self.job([node("n1", "read_xml", {"path": str(xml)})], [], "n1")
        self.assertIn("Ada", parsed["rows"][0])
        blob = self.tmp / "nested.json"
        blob.write_text('[{"name":"Ada","city":{"id":"N"}}]', encoding="utf-8")
        normal = self.job([node("n1", "json_normalize", {"path": str(blob)})], [], "n1")
        self.assertIn("city.id", normal["columns"])
        frame = self.tmp / "ticks.parquet"
        import pandas as pd

        pd.DataFrame({"qty": list(range(12))}).to_parquet(frame, index=False)
        preview = run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": graph_of([node("n1", "read_parquet", {"path": str(frame)})]),
                "node": "n1",
                "base_dir": str(self.tmp),
            }
        )
        self.assertIsNone(preview.get("error"), preview.get("error"))
        self.assertEqual(preview["n"], 12)
        missing = self.rejected([node("n1", "read_parquet", {"path": str(self.tmp / "nope.parquet")})], [], "n1")
        self.assertIn("file not found", missing["error"]["message"])


class VarsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)
        self.csv = self.tmp / "vars_orders.csv"
        self.csv.write_text("amount,region\n12,N\n8,S\n15,N\n3,E\n", encoding="utf-8")

    def document(self, params_n="${rows}", path="${data}", variables=None, extra=None):
        declared = variables if variables is not None else [
            {"name": "data", "kind": "path", "value": "vars_orders.csv"},
            {"name": "rows", "kind": "int", "value": 3},
        ]
        return {
            "format": "ohmypipeline",
            "version": 2,
            "nodes": [
                node("n1", "read_csv", {"path": path}),
                node("n2", "head", {"n": params_n}),
            ],
            "edges": [edge("n1", "n2")],
            "variables": declared,
            **(extra or {}),
        }

    def job(self, document, **extra):
        return run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": document,
                "node": "n2",
                "base_dir": str(self.tmp),
                **extra,
            }
        )

    def test_substitution_uses_the_declared_variables(self):
        reply = self.job(self.document())
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 3)
        self.assertEqual(reply["computed"], ["n1", "n2"])

    def test_an_override_changes_the_value_for_this_run(self):
        reply = self.job(self.document(), overrides={"rows": "2"})
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 2)

    def test_an_override_does_not_return_a_stale_cached_result(self):
        document = self.document()
        before = stats["computed"]
        first = self.job(document)
        self.assertEqual(first["n"], 3)
        overridden = self.job(document, overrides={"rows": "1"})
        self.assertEqual(overridden["n"], 1)
        self.assertGreater(stats["computed"], before, "the override must recompute")

    def test_an_unknown_variable_is_a_node_error(self):
        document = self.document(params_n="${nope}")
        reply = self.job(document)
        self.assertIsNotNone(reply["error"])
        self.assertEqual(reply["error"]["node"], "n2")
        self.assertEqual(reply["error"]["message"], "unknown variable ${nope}")

    def test_a_string_variable_in_a_numeric_param_is_a_type_error(self):
        document = self.document(
            variables=[
                {"name": "data", "kind": "path", "value": "vars_orders.csv"},
                {"name": "rows", "kind": "string", "value": "many"},
            ]
        )
        reply = self.job(document)
        self.assertIsNotNone(reply["error"])
        self.assertEqual(reply["error"]["node"], "n2")
        self.assertEqual(reply["error"]["message"], "variable ${rows} must be an int")

    def test_a_text_param_takes_the_value_as_text(self):
        document = self.document()
        document["nodes"][0]["params"] = {"path": "${data}.csv"}
        document["variables"] = [
            {"name": "data", "kind": "string", "value": "vars_orders"},
            {"name": "rows", "kind": "int", "value": 3},
        ]
        reply = self.job(document)
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 3)

    def test_the_job_graph_is_not_mutated_by_substitution(self):
        import copy

        document = self.document()
        original = copy.deepcopy(document)
        self.job(document, overrides={"rows": "2"})
        self.assertEqual(document, original)

    def test_python_nodes_get_vars_and_an_unsubstituted_code_param(self):
        code = "def run(df):\n    return df.head(vars['rows'])\n"
        document = self.document()
        document["nodes"] = [
            node("n1", "read_csv", {"path": "${data}"}),
            node(
                "n2",
                "python",
                {"code": code},
                inputs=[{"name": "df", "accepts": ["dataframe"]}],
            ),
        ]
        document["edges"] = [edge("n1", "n2", dst_port="df")]
        reply = self.job(document)
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 3)

    def test_a_reset_drops_the_cache_of_the_given_nodes(self):
        document = self.document()
        self.job(document)
        before = stats["computed"]
        self.job(document)
        self.assertEqual(stats["computed"], before, "warm cache serves the second run")
        reset = self.job(document, op="reset", nodes=["n1"])
        self.assertIsNone(reset["error"])
        self.assertEqual(stats["computed"], before, "the reset itself computes nothing")
        self.job(document)
        self.assertGreater(stats["computed"], before, "reset dropped the cache")


class StatsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)

    def run_with_stats(self, nodes, edges, node_id):
        document = graph_of(nodes, edges)
        document["variables"] = document.get("variables", [])
        return run_job(
            {
                "generation": 1,
                "mode": "preview",
                "graph": document,
                "node": node_id,
                "base_dir": str(self.tmp),
                "stats": True,
            }
        )

    def test_stats_cover_the_whole_frame_for_several_dtypes(self):
        csv_path = self.tmp / "stats.csv"
        csv_path.write_text(
            "qty,score,city,when,flag\n1,1.5,N,2024-01-01,true\n2,,S,2024-01-03,false\n3,2.5,N,2024-01-06,true\n",
            encoding="utf-8",
        )
        reply = self.run_with_stats(
            [node("n1", "read_csv", {"path": str(csv_path)})], [], "n1"
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        columns = {item["name"]: item for item in reply["stats"]}
        self.assertEqual([item["name"] for item in reply["stats"]], ["qty", "score", "city", "when", "flag"])
        qty = columns["qty"]
        self.assertEqual(qty["count"], 3)
        self.assertEqual(qty["nulls"], 0)
        self.assertEqual(qty["null_pct"], 0.0)
        self.assertEqual(qty["unique"], 3)
        self.assertEqual(qty["min"], 1)
        self.assertEqual(qty["max"], 3)
        self.assertAlmostEqual(qty["mean"], 2.0)
        self.assertEqual(len(qty["hist"]), 12)
        self.assertEqual(sum(qty["hist"]), 3)
        score = columns["score"]
        self.assertEqual(score["nulls"], 1)
        self.assertEqual(score["null_pct"], 33.3)
        self.assertAlmostEqual(score["mean"], 2.0)
        self.assertEqual(score["top"], [])
        city = columns["city"]
        self.assertEqual(city["min"], "N")
        self.assertEqual(city["max"], "S")
        self.assertEqual(city["hist"], [])
        self.assertEqual(city["top"], [["N", 2], ["S", 1]])
        when = columns["when"]
        self.assertEqual(when["min"], "2024-01-01")
        self.assertEqual(when["max"], "2024-01-06")
        flag = columns["flag"]
        self.assertEqual(flag["min"], False)
        self.assertEqual(flag["max"], True)
        self.assertEqual(flag["hist"], [])

    def test_stats_are_robust_to_all_null_and_empty(self):
        csv_path = self.tmp / "stats_holes.csv"
        csv_path.write_text("empty,mixed\n,\n,\n", encoding="utf-8")
        reply = self.run_with_stats(
            [node("n1", "read_csv", {"path": str(csv_path)})], [], "n1"
        )
        columns = {item["name"]: item for item in reply["stats"]}
        empty = columns["empty"]
        self.assertEqual(empty["count"], 2)
        self.assertEqual(empty["nulls"], 2)
        self.assertEqual(empty["null_pct"], 100.0)
        self.assertIsNone(empty["min"])
        self.assertIsNone(empty["mean"])
        self.assertEqual(empty["hist"], [])
        self.assertEqual(empty["unique"], 0)
        mixed = columns["mixed"]
        self.assertIsNone(mixed["min"])
        self.assertEqual(mixed["hist"], [])

    def test_stats_of_a_series_name_the_column(self):
        csv_path = self.tmp / "one.csv"
        csv_path.write_text("qty\n2\n4\n6\n", encoding="utf-8")
        reply = self.run_with_stats(
            [node("n1", "read_csv", {"path": str(csv_path)}), node("n2", "column", {"name": "qty"})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(len(reply["stats"]), 1)
        stats = reply["stats"][0]
        self.assertEqual(stats["name"], "qty")
        self.assertEqual(stats["count"], 3)
        self.assertAlmostEqual(stats["mean"], 4.0)
        # A series with no name is one column called value.
        unnamed = self.run_with_stats(
            [node("n1", "read_csv", {"path": str(csv_path)}), node("n2", "nunique", {})],
            [edge("n1", "n2")],
            "n2",
        )
        self.assertEqual(unnamed["stats"][0]["name"], "value")

    def test_a_reply_without_stats_has_none(self):
        csv_path = self.tmp / "plain.csv"
        csv_path.write_text("a\n1\n", encoding="utf-8")
        document = graph_of([node("n1", "read_csv", {"path": str(csv_path)})])
        reply = run_job({"generation": 1, "mode": "preview", "graph": document, "node": "n1"})
        self.assertNotIn("stats", reply)
        self.assertEqual(reply["computed"], ["n1"])


class FlowTests(unittest.TestCase):
    """Loops, branches, and the condition language."""

    def setUp(self):
        self.tmp = Path(__file__).resolve().parent / "_tmp"
        self.tmp.mkdir(exist_ok=True)

    def job(self, nodes, edges, node_id, variables=None, **extra):
        document = graph_of(nodes, edges)
        if variables is not None:
            document["variables"] = variables
        return run_job(
            {
                "generation": 1,
                "mode": extra.pop("mode", "preview"),
                "graph": document,
                "node": node_id,
                "base_dir": str(self.tmp),
                **extra,
            }
        )

    def files_loop(self, name, pattern, body_nodes, body_edges, add_column=None):
        """loop_start -> body -> loop_end. The body reads ${loop_value}."""
        end_params = {"pair": name}
        if add_column is not None:
            end_params["add_column"] = add_column
        nodes = [node(name, "loop_start", {"mode": "files", "pattern": pattern})]
        nodes += body_nodes
        nodes.append(node("lend", "loop_end", end_params))
        edges = [edge(name, body_nodes[0]["id"])] + body_edges
        edges.append(edge(body_nodes[-1]["id"], "lend"))
        return nodes, edges

    def test_files_loop_concatenates_and_names_the_loop_column(self):
        for stem, region, rows in (("bulk_a", "N", 2), ("bulk_b", "S", 1), ("bulk_c", "E", 3)):
            path = self.tmp / f"{stem}.csv"
            text = "region,amount\n" + "".join(f"{region},{10 + i}\n" for i in range(rows))
            path.write_text(text, encoding="utf-8")
        reader = node("n2", "read_csv", {"path": "${loop_value}"})
        nodes, edges = self.files_loop("n1", "bulk_*.csv", [reader], [])
        reply = self.job(nodes, edges, "lend")
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"][0], "loop_value")
        self.assertEqual(reply["n"], 6)
        loop_values = [row[0] for row in reply["rows"]]
        self.assertEqual(len(loop_values), 6)
        self.assertTrue(all(Path(str(v)).is_absolute() for v in loop_values))
        regions = {row[reply["columns"].index("region")] for row in reply["rows"]}
        self.assertEqual(regions, {"N", "S", "E"})
        self.assertEqual(reply["inactive"], [])

    def test_groups_loop_iterates_sorted_unique_values(self):
        csv_path = self.tmp / "grouped.csv"
        csv_path.write_text("region,amount\nN,12\nS,8\nN,15\n", encoding="utf-8")
        start = node("n1", "loop_start", {"mode": "groups", "column": "region", "name": "g"})
        head = node("n2", "head", {"n": 1})
        end = node("n3", "loop_end", {"pair": "n1"})
        reply = self.job(
            [
                node("n0", "read_csv", {"path": str(csv_path)}),
                start,
                head,
                end,
            ],
            [edge("n0", "n1"), edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"][0], "g")
        self.assertEqual([row[0] for row in reply["rows"]], ["N", "S"])
        amounts = [row[reply["columns"].index("amount")] for row in reply["rows"]]
        self.assertEqual(amounts, [12, 8])

    def test_values_loop_splits_a_variable(self):
        csv_path = self.tmp / "values_in.csv"
        csv_path.write_text("region,amount\nN,12\n", encoding="utf-8")
        start = node("n1", "loop_start", {"mode": "values", "values": "${regions}", "name": "region_v"})
        head = node("n2", "head", {"n": 1})
        end = node("n3", "loop_end", {"pair": "n1"})
        reply = self.job(
            [
                node("n0", "read_csv", {"path": str(csv_path)}),
                start,
                head,
                end,
            ],
            [edge("n0", "n1"), edge("n1", "n2"), edge("n2", "n3")],
            "n3",
            variables=[{"name": "regions", "kind": "string", "value": "N,S,E"}],
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["columns"][0], "region_v")
        self.assertEqual([row[0] for row in reply["rows"]], ["N", "S", "E"])
        self.assertEqual(reply["n"], 3)

    def test_preview_caps_the_loop_at_three_of_five_iterations(self):
        csv_path = self.tmp / "cap_in.csv"
        csv_path.write_text("region,amount\nN,12\n", encoding="utf-8")
        start = node("n1", "loop_start", {"mode": "values", "values": "1,2,3,4,5", "name": "i"})
        head = node("n2", "head", {"n": 1})
        end = node("n3", "loop_end", {"pair": "n1"})
        reply = self.job(
            [
                node("n0", "read_csv", {"path": str(csv_path)}),
                start,
                head,
                end,
            ],
            [edge("n0", "n1"), edge("n1", "n2"), edge("n2", "n3")],
            "n3",
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["claim"], "preview · first 3 of 5 iterations")
        self.assertEqual(reply["n"], 3)
        self.assertEqual([row[0] for row in reply["rows"]], ["1", "2", "3"])
        full = self.job(
            [
                node("n0", "read_csv", {"path": str(csv_path)}),
                start,
                head,
                end,
            ],
            [edge("n0", "n1"), edge("n1", "n2"), edge("n2", "n3")],
            "n3",
            mode="full",
        )
        self.assertEqual(full["n"], 5)
        self.assertEqual(full["claim"], "full")

    def test_an_iteration_error_names_the_file(self):
        good = self.tmp / "err_a.csv"
        good.write_text("region,amount\nN,12\n", encoding="utf-8")
        bad = self.tmp / "err_b.csv"
        bad.write_text("region\nS,8\n", encoding="utf-8")
        reader = node("n2", "read_csv", {"path": "${loop_value}"})
        column = node("n3", "column", {"name": "amount"})
        nodes, edges = self.files_loop("n1", "err_*.csv", [reader, column], [edge("n2", "n3")])
        reply = self.job(nodes, edges, "lend")
        self.assertIsNotNone(reply["error"])
        self.assertEqual(reply["error"]["node"], "n3")
        self.assertEqual(
            reply["error"]["message"],
            "iteration 2 (err_b.csv): missing column amount",
        )

    def test_if_switch_branches_and_reports_inactive(self):
        csv_path = self.tmp / "branch.csv"
        csv_path.write_text("amount,region\n12,N\n8,S\n15,N\n3,E\n", encoding="utf-8")
        document_nodes = [
            node("n1", "read_csv", {"path": str(csv_path)}),
            node("n2", "if_switch", {"condition": '${mode} == "full"'}),
            node("n3", "head", {"n": 2}),
            node("n4", "head", {"n": 1}),
            node("n5", "end_if", {}),
        ]
        document_edges = [
            edge("n1", "n2"),
            edge("n2", "n3", src_port="true"),
            edge("n2", "n4", src_port="false"),
            edge("n3", "n5", dst_port="true"),
            edge("n4", "n5", dst_port="false"),
        ]
        variables = [{"name": "mode", "kind": "string", "value": "preview"}]
        reply = self.job(document_nodes, document_edges, "n5", variables=variables)
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 1)
        self.assertEqual(reply["inactive"], ["n3"])
        self.assertEqual(reply["claim"], "preview")
        full = self.job(
            document_nodes,
            document_edges,
            "n5",
            variables=variables,
            overrides={"mode": "full"},
        )
        self.assertIsNone(full.get("error"), full.get("error"))
        self.assertEqual(full["n"], 2)
        self.assertEqual(full["inactive"], ["n4"])

    def test_rows_condition_on_an_empty_frame(self):
        csv_path = self.tmp / "empty.csv"
        csv_path.write_text("amount,region\n", encoding="utf-8")
        nodes = [
            node("n1", "read_csv", {"path": str(csv_path)}),
            node("n2", "if_switch", {"condition": "rows > 0"}),
            node("n3", "head", {"n": 1}),
            node("n4", "head", {"n": 1}),
            node("n5", "end_if", {}),
        ]
        edges = [
            edge("n1", "n2"),
            edge("n2", "n3", src_port="true"),
            edge("n2", "n4", src_port="false"),
            edge("n3", "n5", dst_port="true"),
            edge("n4", "n5", dst_port="false"),
        ]
        reply = self.job(nodes, edges, "n5")
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["inactive"], ["n3"])
        self.assertEqual(reply["n"], 0)

    def test_skip_propagates_through_two_nodes(self):
        csv_path = self.tmp / "skip.csv"
        csv_path.write_text("amount,region\n12,N\n8,S\n", encoding="utf-8")
        nodes = [
            node("n1", "read_csv", {"path": str(csv_path)}),
            node("n2", "if_switch", {"condition": "${off}"}),
            node("n3", "head", {"n": 1}),
            node("n4", "head", {"n": 1}),
            node("n5", "head", {"n": 1}),
            node("n6", "end_if", {}),
        ]
        edges = [
            edge("n1", "n2"),
            edge("n2", "n3", src_port="true"),
            edge("n3", "n4"),
            edge("n4", "n6", dst_port="true"),
            edge("n2", "n5", src_port="false"),
            edge("n5", "n6", dst_port="false"),
        ]
        reply = self.job(
            nodes,
            edges,
            "n6",
            variables=[{"name": "off", "kind": "bool", "value": False}],
        )
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["inactive"], ["n3", "n4"])
        self.assertEqual(reply["n"], 1)

    def test_an_if_switch_inside_a_loop_body(self):
        two = self.tmp / "body_a.csv"
        two.write_text("region,amount\nN,12\nN,15\n", encoding="utf-8")
        one = self.tmp / "body_b.csv"
        one.write_text("region,amount\nS,8\n", encoding="utf-8")
        reader = node("n2", "read_csv", {"path": "${loop_value}"})
        switch = node("n3", "if_switch", {"condition": "rows > 1"})
        head = node("n4", "head", {"n": 1})
        nodes, edges = self.files_loop(
            "n1",
            "body_*.csv",
            [reader, switch, head],
            [
                edge("n2", "n3"),
                edge("n3", "n4", src_port="true"),
            ],
        )
        reply = self.job(nodes, edges, "lend")
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["n"], 1)
        self.assertEqual(reply["inactive"], ["n4"])
        loop_values = [row[0] for row in reply["rows"]]
        self.assertTrue(loop_values[0].endswith("body_a.csv"))

    def test_requesting_a_body_node_runs_the_first_iteration(self):
        first = self.tmp / "first_a.csv"
        first.write_text("region,amount\nN,12\n", encoding="utf-8")
        second = self.tmp / "first_b.csv"
        second.write_text("region,amount\nS,8\nS,9\n", encoding="utf-8")
        reader = node("n2", "read_csv", {"path": "${loop_value}"})
        nodes, edges = self.files_loop("n1", "first_*.csv", [reader], [])
        reply = self.job(nodes, edges, "n2")
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["claim"], "first iteration")
        self.assertEqual(reply["n"], 1)
        self.assertEqual(reply["rows"][0][reply["columns"].index("region")], "N")
        start = node("n1", "loop_start", {"mode": "files", "pattern": "first_*.csv"})
        end = node("lend", "loop_end", {"pair": "n1"})
        reply = self.job([start, reader, end], [edge("n1", "n2"), edge("n2", "lend")], "n1")
        self.assertIsNone(reply.get("error"), reply.get("error"))
        self.assertEqual(reply["claim"], "first iteration")
        self.assertEqual(reply["columns"], ["path", "name"])
        self.assertEqual(reply["rows"][0][1], "first_a")

    def test_a_loop_end_without_a_pair_is_an_error(self):
        end = node("n1", "loop_end", {"pair": "missing"})
        reply = self.job([end], [], "n1")
        self.assertIsNotNone(reply["error"])
        self.assertIn("not paired", reply["error"]["message"])

    def test_the_condition_parser_rejects_calls_and_broken_text(self):
        import pandas as pd

        from pipeline.flow import FlowError, evaluate

        with self.assertRaises(FlowError) as caught:
            evaluate("__import__('os')", {}, None)
        self.assertIn("unknown name", str(caught.exception))
        with self.assertRaises(FlowError) as caught:
            evaluate("((rows > 0)", {}, None)
        self.assertIn("unbalanced parenthesis at 0", str(caught.exception))
        with self.assertRaises(FlowError):
            evaluate("rows >", {}, None)
        self.assertTrue(evaluate('${m} == "full"', {"m": "full"}, None))
        self.assertFalse(evaluate("rows > 0", {}, pd.DataFrame()))
        self.assertTrue(evaluate("not false and (1 < 2)", {}, None))


if __name__ == "__main__":
    unittest.main()
