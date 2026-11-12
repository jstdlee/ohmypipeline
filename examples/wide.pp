{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "wide.csv",
      "position": [48, 80],
      "params": { "path": "wide.csv" }
    },
    {
      "id": "n2",
      "type": "wide_to_long",
      "label": "A by year",
      "position": [340, 80],
      "params": { "stubnames": "A", "i": "id", "j": "year", "sep": "" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
