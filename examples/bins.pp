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
      "type": "cut",
      "label": "Qty bands",
      "position": [340, 48],
      "params": { "column": "qty", "bins": "0,5,10" }
    },
    {
      "id": "n3",
      "type": "get_dummies",
      "label": "Flag dummies",
      "position": [340, 220],
      "params": { "column": "flag" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n1", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
