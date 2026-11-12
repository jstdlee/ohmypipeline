{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "bins.csv",
      "position": [48, 80],
      "params": { "path": "bins.csv" }
    },
    {
      "id": "n2",
      "type": "calculate",
      "label": "Double qty",
      "position": [340, 80],
      "params": { "expr": "double = qty * 2" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
