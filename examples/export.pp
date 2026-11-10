{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "people.csv",
      "position": [48, 80],
      "params": { "path": "people.csv" }
    },
    {
      "id": "n2",
      "type": "to_csv",
      "label": "Write CSV",
      "position": [360, 80],
      "params": { "path": "people-export.csv", "index": "no" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
