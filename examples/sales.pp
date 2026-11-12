{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "sales.csv",
      "position": [48, 64],
      "params": {
        "path": "sales.csv",
        "skip_rows": 1,
        "header": "header row",
        "header_row": 0
      }
    },
    {
      "id": "n2",
      "type": "pivot",
      "label": "Qty by region",
      "position": [360, 64],
      "params": {
        "index": "region",
        "columns": "product",
        "values": "qty",
        "agg": "sum"
      }
    },
    {
      "id": "n3",
      "type": "preview_table",
      "label": "Preview table",
      "position": [360, 320]
    },
    {
      "id": "n4",
      "type": "chart",
      "label": "Pen by region",
      "position": [760, 32],
      "params": { "mark": "line", "x": "region", "y": "Pen", "series": "" }
    },
    {
      "id": "n5",
      "type": "chart",
      "label": "Book by region",
      "position": [760, 320],
      "params": { "mark": "pie", "x": "region", "y": "Book", "series": "" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n4", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n5", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
