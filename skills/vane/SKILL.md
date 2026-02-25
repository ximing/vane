---
name: vane
description: Use when the user wants local RAG, the vane CLI, sidecar MCP search, reading indexed notes or docs without walking the filesystem, or to install or start the Vane daemon. Triggers include vane query, vane mcp, list_roots, search/read of local folders, Ollama embeddings, and 本机文档检索 / 不要扫盘.
version: 0.3.0
sourcePath: crates/vane
repository: git@github.com:ximing/vane.git
---

# Vane sidecar

Local hybrid search over folders the user registered. One daemon, Unix socket, MCP stdio. macOS and Linux only.

**Do not walk the project tree, parse `.gitignore`, or invent a second search protocol.** Prefer MCP tools `list_roots` / `search` / `read`. Fall back to the `vane` CLI only when those tools are not available.

## Install CLI (if `vane` is missing)

```bash
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-cli.sh | sh
export PATH="$HOME/.local/bin:$PATH"
vane --version
```

Unsupported arch: `cargo install --git https://github.com/ximing/vane.git --locked --bin vane`

## Daemon must be running

```bash
vane status          # not initialized → vane init
vane start           # if the user service is not installed
```

`vane init` wizard: embed provider (Ollama default `nomic-embed-text`, or `openai_compat`), first folder, excludes, optional user service (launchd / systemd --user).

Home: `--home` > `VANE_HOME` > `~/.vane`. Never put `api_key` in `<root>/.vane.toml`.

`vane mcp` is only a stdio bridge to `~/.vane/run/vane.sock`. It does not embed the index.

## MCP client config

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

## Workflow

1. **`list_roots`** — registered roots, `project_id`, model/dim, live file counts, rebuild progress.
2. **`search`** — required `query`. Optional `root` (absolute path), `type` (`text` / `image`), `top_k` (default 8, max 50). Default scope is every registered project.
3. **`read`** — hit `id` (one chunk) or `path` (all chunks, ascending). Pass `root` when the same relative path exists in more than one project.

Hits include `id`, `path`, `root`, `title`, `snippet`, `score`, `modality`, `extractor`, `degraded`. `degraded: true` means the embedder was down and the hit is BM25-only.

`read` of an image ≤ 4 MiB returns MCP image content; larger files return the absolute path and MIME type only (no base64).

CLI fallback when MCP tools are absent:

```bash
vane query "how does auth work"
vane query "release" --all
vane query "logo" --type image --top-k 8
```

`vane query` without `--all` / `--root` uses the registered root that contains the current working directory.

## Common mistakes

| Symptom | What to do |
|---|---|
| `vane: command not found` | Install the CLI; add `~/.local/bin` to `PATH` |
| MCP: daemon is not running | `vane start` (or `vane init` first) |
| `list_roots` empty / live files 0 | `vane add <folder>`; wait for first reconcile; `vane status` |
| `search` returns `[]` | Confirm live files > 0. Embedder down still returns BM25 with `degraded` |
| Agent globbed or grepped the repo | Stop. Call `search` then `read` |

`vane gc` never deletes the user's source files — only data under `$VANE_HOME/rag`.
