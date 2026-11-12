{
  "format": "ohmypipeline",
  "version": 1,
  "nodes": [
    {
      "id": "n1",
      "type": "read_html",
      "label": "page.html",
      "position": [48, 64],
      "params": { "path": "page.html", "which": 0 }
    },
    {
      "id": "n2",
      "type": "read_xml",
      "label": "rows.xml",
      "position": [360, 64],
      "params": { "path": "rows.xml" }
    },
    {
      "id": "n3",
      "type": "json_normalize",
      "label": "nested.json",
      "position": [680, 64],
      "params": { "path": "nested.json" }
    }
  ],
  "edges": [],
  "marks": [],
  "camera": { "x": 0, "y": 0, "zoom": 1 }
}
