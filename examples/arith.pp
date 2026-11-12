{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "bins.csv",
      "position": [48, 96],
      "params": { "path": "bins.csv" }
    },
    {
      "id": "n2",
      "type": "arith",
      "label": "Double qty",
      "position": [340, 48],
      "params": { "op": "mul", "value": "2" }
    },
    {
      "id": "n3",
      "type": "apply",
      "label": "Sum columns",
      "position": [340, 240],
      "params": { "fn": "sum", "axis": "0" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n1", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
