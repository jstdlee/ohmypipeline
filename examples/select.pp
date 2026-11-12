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
      "type": "isin",
      "label": "North and East",
      "position": [360, 48],
      "params": { "column": "city", "values": "North,East" }
    },
    {
      "id": "n3",
      "type": "loc",
      "label": "First two labels",
      "position": [680, 48],
      "params": { "rows": "0:1" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
