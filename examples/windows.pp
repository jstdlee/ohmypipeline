{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "people.csv",
      "position": [48, 120],
      "params": { "path": "people.csv" }
    },
    {
      "id": "n2",
      "type": "groupby",
      "label": "Qty by city",
      "position": [360, 48],
      "params": { "by": "city", "agg": "sum", "aggs": "qty:sum, qty:mean" }
    },
    {
      "id": "n3",
      "type": "rolling",
      "label": "Rolling mean",
      "position": [360, 240],
      "params": { "window": 2, "agg": "mean", "column": "qty" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n1", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
