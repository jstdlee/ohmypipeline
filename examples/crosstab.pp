{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "sales.csv",
      "position": [48, 80],
      "params": {
        "path": "sales.csv",
        "skip_rows": 1,
        "header": "header row",
        "header_row": 0
      }
    },
    {
      "id": "n2",
      "type": "crosstab",
      "label": "Region by product",
      "position": [360, 80],
      "params": { "index": "region", "columns": "product", "agg": "count" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
