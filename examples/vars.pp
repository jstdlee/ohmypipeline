{
  "format": "ohmypipeline",
  "version": 2,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "orders",
      "position": [48, 64],
      "params": { "path": "${data}" }
    },
    {
      "id": "n2",
      "type": "head",
      "label": "Head",
      "position": [320, 64],
      "params": { "n": "${rows}" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "variables": [
    { "name": "data", "kind": "path", "value": "orders.csv" },
    { "name": "rows", "kind": "int", "value": 3 }
  ],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
