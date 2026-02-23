---
name: vane
description: Search and read locally indexed documents through the Vane sidecar MCP tools. Use when the user wants local RAG, document search, or to read indexed files without walking the filesystem.
---

# Vane

Use the `vane` MCP server. Do **not** walk the project tree, parse `.gitignore`, or invent a second search protocol.

The sidecar daemon must already be running (`vane start` or the user service). `vane mcp` is only a stdio bridge to `run/vane.sock`.

## Workflow

1. Call **`list_roots`** to see registered roots, `project_id`, model/dim, and live file counts.
2. Call **`search`** with `query` (optional `root`, `type` = extractor name `text`/`image`, `top_k` default 8 max 50). Default scope is every registered project.
3. Call **`read`** with a hit `id` (one chunk) or `path` (all chunks, ascending). Pass `root` when the path exists in more than one project.

`read` of an image returns MCP image content when the file is ≤ 4 MiB; larger files return the absolute path and MIME type only (no base64).

## Client config

```json
{
  "mcpServers": {
    "vane": {
      "command": "vane",
      "args": ["mcp"]
    }
  }
}
```
