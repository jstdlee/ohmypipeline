{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "times.csv",
      "position": [48, 80],
      "params": { "path": "times.csv" }
    },
    {
      "id": "n2",
      "type": "resample",
      "label": "Weekly sum",
      "position": [340, 80],
      "params": { "column": "when", "freq": "W", "agg": "sum", "fill": "none" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
