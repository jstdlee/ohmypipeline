{
  "format": "ohmypipeline",
  "version": 2,
  "nodes": [
    {
      "id": "n1",
      "type": "read_csv",
      "label": "orders",
      "position": [48, 64],
      "params": { "path": "orders.csv" }
    },
    {
      "id": "n2",
      "type": "if_switch",
      "label": "If switch",
      "position": [288, 64],
      "params": { "condition": "${mode} == \"full\"" }
    },
    {
      "id": "n3",
      "type": "head",
      "label": "Head",
      "position": [528, 16],
      "params": { "n": 100 }
    },
    {
      "id": "n4",
      "type": "head",
      "label": "Head",
      "position": [528, 160],
      "params": { "n": 5 }
    },
    {
      "id": "n5",
      "type": "end_if",
      "label": "End if",
      "position": [768, 64],
      "params": {}
    }
  ],
  "edges": [
    { "from": "n1", "from_port": "out", "to": "n2", "to_port": "in" },
    { "from": "n2", "from_port": "true", "to": "n3", "to_port": "in" },
    { "from": "n2", "from_port": "false", "to": "n4", "to_port": "in" },
    { "from": "n3", "from_port": "out", "to": "n5", "to_port": "true" },
    { "from": "n4", "from_port": "out", "to": "n5", "to_port": "false" }
  ],
  "variables": [
    { "name": "mode", "kind": "string", "value": "preview" }
  ],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
