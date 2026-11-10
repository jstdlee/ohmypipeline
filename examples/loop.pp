{
  "format": "ohmypipeline",
  "version": 2,
  "nodes": [
    {
      "id": "n1",
      "type": "loop_start",
      "label": "Loop start",
      "position": [48, 64],
      "params": { "mode": "files", "pattern": "loop/*.csv", "name": "loop_value" }
    },
    {
      "id": "n2",
      "type": "read_csv",
      "label": "Read CSV",
      "position": [288, 64],
      "params": { "path": "${loop_value}" }
    },
    {
      "id": "n3",
      "type": "loop_end",
      "label": "Loop end",
      "position": [528, 64],
      "params": { "pair": "n1", "add_column": true }
    }
  ],
  "edges": [
    { "from": "n2", "from_port": "out", "to": "n3", "to_port": "in" }
  ],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
