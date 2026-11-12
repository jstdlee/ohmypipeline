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
      "type": "ffill",
      "label": "Fill forward",
      "position": [360, 80],
      "params": {}
    },
    {
      "id": "n3",
      "type": "clip",
      "label": "Clip qty",
      "position": [680, 80],
      "params": { "lower": "5", "upper": "9" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
