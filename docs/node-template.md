# Node template

A template is one node, not a pipeline. Built-in features stay as they are. Export writes a template. Customize stores a copy in the user library. Import checks the format and the version, and refuses a version it does not know.

Files are UTF-8 JSON.

```json
{
  "format": "ohmypipeline-template",
  "version": 1,
  "id": "keep_north",
  "title": "Keep the north rows",
  "group": "Library",
  "base": "python",
  "notes": "Drops the other regions.",
  "ports": [
    { "name": "df", "accepts": ["dataframe"], "multi": false }
  ],
  "params": [
    { "name": "code", "kind": "code", "default": "def run(df):\n    return df\n" }
  ],
  "code": "def run(df):\n    return df\n",
  "help": "Optional inspector text."
}
```

`format` must be `ohmypipeline-template`. `version` must be `1`. `id` and `title` are required. `base` is the built-in feature id when the template started from one. `code` is present only when the user customized behavior. A built-in export stores the feature id and the current parameters. It does not turn the built-in into editable source.

The library lives in `$XDG_DATA_HOME/ohmypipeline/library`, or `~/.local/share/ohmypipeline/library`. The palette lists those files under Library.
