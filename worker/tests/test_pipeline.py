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


if __name__ == "__main__":
    unittest.main()
