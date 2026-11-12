{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "people.csv",
      "position": [48, 72],
      "params": { "path": "people.csv" }
    },
    {
      "id": "n2",
      "type": "str",
      "label": "Lowercase names",
      "position": [340, 72],
      "params": { "method": "lower", "column": "name" }
    },
    {
      "id": "n3",
      "type": "to_datetime",
      "label": "Parse dates",
      "position": [640, 72],
      "params": { "column": "when" }
    },
    {
      "id": "n4",
      "type": "dt",
      "label": "Year",
      "position": [940, 72],
      "params": { "accessor": "year", "column": "when" }
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" },
    { "from": "n3", "from_port": "out", "to": "n4", "to_port": "in" }
  ],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
