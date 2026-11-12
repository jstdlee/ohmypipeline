{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "orders.csv",
      "position": [48, 64],
      "params": { "path": "orders.csv" }
    },
    {
      "id": "n2",
      "type": "head",
      "label": "Head",
      "position": [320, 64],
      "params": { "n": 5 }
    },
    {
      "id": "n3",
      "type": "change_headers",
      "label": "Change headers",
      "position": [592, 64],
      "params": {
        "mapping": [
          { "from": "amount", "to": "revenue" },
          { "from": "region", "to": "" }
        ]
      }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
