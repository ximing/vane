import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import { Link } from 'react-router-dom';
import './Sidecar.css';

const INSTALL_RELEASE = `# macOS (Apple Silicon / Intel) and Linux x86_64
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-cli.sh | sh
# installs to ~/.local/bin/vane  (override with PREFIX=/usr/local)
export PATH="$HOME/.local/bin:$PATH"
vane --version`;

const INSTALL_SOURCE = `git clone https://github.com/ximing/vane.git
cd vane
cargo install --path crates/vane --locked --force
# or, without cloning:
cargo install --git https://github.com/ximing/vane.git --locked --bin vane`;

const PREREQ = `# default embedder (skip if you choose openai_compat in vane init)
ollama pull nomic-embed-text`;

const INIT = `vane init
# 1. Embedding provider: ollama (default) or openai_compat
# 2. Model / Base URL
# 3. API key (openai_compat only; empty uses OPENAI_API_KEY / VANE_EMBED_API_KEY)
# 4. Vector dimension (empty to probe from the API)
# 5. First project root (can skip)
# 6. Exclude globs (defaults include .git, node_modules, target, *.log, .env, …)
# 7. Enable image types? (default no)  /  Install user service? (default yes)
# Embed probe fails closed. TTY can confirm; scripts:
#   VANE_ALLOW_EMBED_FAIL=1 vane init

vane add ~/notes          # if you skipped the first root
vane start                # if the user service is not installed
vane status               # TTY dashboard (JSON if piped)`;

const QUERY = `# Search the current project (cwd must sit inside a registered root)
vane query "how does auth work" --top-k 8

# One registered root, or every root fused with RRF
vane query "release checklist" --root ~/notes
vane query "release checklist" --all

# Filter by extractor name (not file suffix)
vane query "logo" --type image

# Bare vane query on a TTY prompts for the text; empty cancels (exit 0)
vane query

# --verbose prints each hit's internal id (default hidden)
vane query "how does auth work" --verbose

# Empty hits still exit 0 and print a why line in a TTY`;

const READ = `# Read the n-th hit (1-based) of the last TTY query's chunk text
vane read 1
vane read 2

# Print the whole source file instead of the chunk (ignores cache staleness)
vane read 2 --file

# Non-TTY output is plain chunk text (no meta line), pipe-friendly
vane read 1 | less`;

const MCP = `{
  "mcpServers": {
    "vane": {
      "command": "vane",
      "args": ["mcp"]
    }
  }
}`;

const MCP_INSTALL = `vane mcp install --dry-run              # print what would be written
vane mcp install                        # Claude, Cursor, existing Codex
vane mcp install --client claude        # claude | cursor | codex

# --client claude also installs the vane agent skill to ~/.claude/skills/vane/
# (default-all installs it only if ~/.claude already exists; it never creates ~/.claude)`;

const DIAGNOSE = `vane status                 # TTY dashboard (JSON if piped)
vane doctor                 # config, socket, daemon, embedder, roots, disk
vane issues                 # skipped files in the current root
vane issues --all
vane logs                   # last 50 redacted daemon lines
vane logs --follow --lines 200
vane inspect                # resolved embed / chunk / exclude / types
vane inspect --global
vane inspect --root ~/notes
vane df                     # $VANE_HOME, CAS, per-project dbs
vane gc --dry-run           # count unreferenced cache; does not delete
vane gc --all --dry-run`;

const WATCH = `# Foreground-watch a root for index changes (client-side polling, no daemon IPC)
vane watch                  # current root
vane watch --root ~/notes
vane watch --all            # every registered root
vane watch --interval-ms 500   # 100..=60000, default 1000

# Non-TTY prints one JSON object per line: {"event":"updated","path":"…","root":"…","at":…}
vane watch --root ~/notes | jq .`;

const MODEL = `vane model --model nomic-embed-text --yes
# --yes skips the rebuild prompt (required when not a TTY)`;

const PROJECT_TOML = `# <root>/.vane.toml  — checked into the repo, never put api_key here
[embed]
provider = "openai_compat"
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"

[chunk]
max_chars = 800

exclude = ["**/generated/**"]
include = ["**/*.{md,rst}"]`;

const SKILL_INSTALL = `# Claude / Codex / Cursor / Grok — same SKILL.md, one copy per runtime
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-skill.sh | sh

# or by hand:
#   mkdir -p ~/.claude/skills/vane ~/.agents/skills/vane ~/.cursor/skills/vane ~/.grok/skills/vane
#   curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/skills/vane/SKILL.md \\
#     | tee ~/.claude/skills/vane/SKILL.md \\
#          ~/.agents/skills/vane/SKILL.md \\
#          ~/.cursor/skills/vane/SKILL.md \\
#          ~/.grok/skills/vane/SKILL.md >/dev/null`;

const SKILL_PLUGINS = `# Claude Code (this repo is a plugin marketplace)
/plugin marketplace add ximing/vane
/plugin install vane@vane

# Codex
codex plugin marketplace add ximing/vane
codex plugin add vane@vane

# Kimi Code
/plugins install https://github.com/ximing/vane
# then /new so the plugin loads`;

export default function Sidecar() {
  return (
    <DocsLayout>
      <article className="sc-page">
        <h1>Local sidecar CLI</h1>
        <p className="sc-lede">
          The <code>vane</code> binary is an optional native product on top of
          the retrieval library: one daemon watches folders you register,
          chunks and embeds them, and serves hybrid search to humans via CLI
          and to agents via MCP. The library itself still does not run models
          — this sidecar calls Ollama or an OpenAI-compatible API for you.
        </p>

        <Callout type="note" title="macOS and Linux">
          First version uses a Unix socket and a launchd / systemd --user
          service. There is no Windows service. Tests and the daemon never
          write your real <code>~/.vane</code> unless you run the CLI
          yourself.
        </Callout>

        <h2 id="install">Install</h2>
        <p>
          Tagged GitHub Releases attach prebuilt binaries for Linux x86_64,
          macOS arm64, and macOS x86_64. The install script picks the right
          one. Add <code>~/.local/bin</code> to <code>PATH</code> if{' '}
          <code>vane</code> is not found afterwards.
        </p>
        <CodeBlock lang="bash" title="install from GitHub Release" code={INSTALL_RELEASE} />
        <p>
          On an unsupported arch, build from source (needs a Rust toolchain):
        </p>
        <CodeBlock lang="bash" title="install from source" code={INSTALL_SOURCE} />
        <p>
          The default embedder is a local Ollama model. Pull it once, or choose{' '}
          <code>openai_compat</code> in the init wizard and set{' '}
          <code>OPENAI_API_KEY</code> / <code>VANE_EMBED_API_KEY</code>.
        </p>
        <CodeBlock lang="bash" title="Ollama (default embedder)" code={PREREQ} />

        <h2 id="init">Initialize</h2>
        <p>
          Home directory resolution is <code>--home</code> &gt;{' '}
          <code>VANE_HOME</code> &gt; <code>~/.vane</code>.{' '}
          <code>vane init</code> writes <code>~/.vane/config/config.toml</code>{' '}
          and can install a user service that starts <code>vane daemon</code>{' '}
          at login.
        </p>
        <CodeBlock lang="bash" title="vane init" code={INIT} />
        <p>
          Point embeddings at a running Ollama (
          <code>nomic-embed-text</code> by default) or an OpenAI-compatible
          endpoint. API keys belong in the global config or in{' '}
          <code>OPENAI_API_KEY</code> / <code>VANE_EMBED_API_KEY</code> — a
          key in a project <code>.vane.toml</code> is rejected. The wizard
          probes the embedder and <strong>fails closed</strong> if the probe
          fails. In a terminal you can confirm "Continue anyway?"; in scripts
          set <code>VANE_ALLOW_EMBED_FAIL=1</code> to write config anyway.
          Search stays BM25-only (<code>degraded</code>) until the provider is
          up.
        </p>

        <h2 id="search">Search from the CLI</h2>
        <p>
          The daemon must already be running. <code>vane query</code> defaults
          to the registered root that contains your current directory; pass{' '}
          <code>--all</code> to fuse hits across every project with RRF.
        </p>
        <CodeBlock lang="bash" title="vane query" code={QUERY} />
        <p>
          On a TTY the first line is a scope header —{' '}
          <code>searching ~/notes · 12 live files · hybrid</code> (or{' '}
          <code>BM25 (degraded: embedder unreachable)</code> when the embedder
          is down) — so you always know which directory you searched and whether
          vectors were used. Single-root hits omit the per-line root (the header
          already says it); <code>--all</code> repeats it for disambiguation.
        </p>
        <p>
          If the embedding provider is down, search falls back to BM25 and
          marks hits <code>degraded</code>. Already-indexed vectors stay on
          disk. Empty hits still succeed: in a terminal the CLI prints a{' '}
          <strong>why</strong> line (not initialized, cwd not a registered
          root, still indexing, embedder down, excluded path, wrong root, empty
          index, or no matching chunks). Piped stdout stays JSON; the reason
          goes to stderr.
        </p>

        <h2 id="read">Read a hit without copying a path</h2>
        <p>
          After a TTY <code>vane query</code>, the hits are cached at{' '}
          <code>~/.vane/run/last_query.json</code>. <code>vane read &lt;n&gt;</code>{' '}
          prints the chunk text of the n-th hit (1-based), so you can go from
          search to reading without leaving the terminal. Only TTY queries write
          the cache — a piped <code>vane query | jq</code> does not clobber it.
        </p>
        <CodeBlock lang="bash" title="vane read" code={READ} />
        <p>
          If the file changed since the query, <code>read &lt;n&gt;</code>{' '}
          reports <em>stale</em>; <code>--file</code> reads straight from disk
          and is unaffected by staleness. <code>read</code> is for human
          verification — agent scripts should use the stateless MCP{' '}
          <code>read</code> tool instead, which does not depend on the cache.
        </p>

        <h2 id="diagnose">Diagnose and maintain</h2>
        <p>
          In a terminal, <code>vane status</code> is a dashboard: daemon
          up/down, dirty queue, disk, and each root&apos;s live files, model,
          and skip count. Piped stdout is JSON. When search looks empty or
          stale, start with <code>vane doctor</code>, then skipped files, logs,
          and resolved policy.
        </p>
        <CodeBlock lang="bash" title="doctor, issues, logs, inspect, df, gc" code={DIAGNOSE} />
        <p>
          The TTY dashboard speaks human: <code>watching</code> when idle,{' '}
          <code>indexing 34/120</code> while a reconcile runs,{' '}
          <code>indexed 3 min ago</code> (relative time, not Unix seconds), and{' '}
          <code>12 skipped — run vane issues</code> when a root has skips. Under{' '}
          <code>LANG=zh_CN.UTF-8</code> the wizard prompts, doctor report, empty
          result reasons, and status lines render in Chinese; JSON and piped
          output always stay English so agents are unaffected.
        </p>
        <p>
          <code>vane issues</code> lists files skipped as too large, invalid
          UTF-8, or embed / extractor errors. <code>vane logs</code> prints
          redacted daemon lines (<code>--lines</code> defaults to 50;{' '}
          <code>--follow</code> tails new ones). <code>vane inspect</code>{' '}
          shows the resolved embed / chunk / exclude / types policy for the
          current project, <code>--root</code>, or <code>--global</code>{' '}
          defaults. <code>vane gc --dry-run</code> counts unreferenced CAS
          without deleting anything.
        </p>
        <p>
          Changing the embedding model re-embeds live files. Confirm in a TTY,
          or pass <code>--yes</code> (<code>-y</code>) when stdin is not a
          terminal — required in scripts.
        </p>
        <CodeBlock lang="bash" title="vane model --yes" code={MODEL} />

        <h2 id="watch-cli">Watch a root live (vane watch)</h2>
        <p>
          <code>vane watch</code> is a foreground observer: it polls the
          live set and dirty queue client-side (no daemon IPC, no notify) and
          prints a line per change — <code>added</code>, <code>updated</code>,
          <code>removed</code> (live-set membership / content-key changes),{' '}
          <code>queued</code> (new dirty entries). Ctrl-C stops it. If the
          daemon is down it prints a one-time hint and keeps watching local
          state.
        </p>
        <CodeBlock lang="bash" title="vane watch" code={WATCH} />

        <h2 id="mcp">MCP and agent skill</h2>
        <p>
          <code>vane mcp</code> is a stdio JSON-RPC 2.0 bridge to{' '}
          <code>~/.vane/run/vane.sock</code>. It does not embed the index
          engine. Merge <code>mcpServers.vane</code> into Claude / Cursor /
          Codex configs under <code>$HOME</code> (default: all known clients),
          or add this by hand:
        </p>
        <CodeBlock lang="bash" title="vane mcp install" code={MCP_INSTALL} />
        <CodeBlock lang="json" title="mcpServers" code={MCP} />
        <p>Three tools:</p>
        <table className="sc-table">
          <thead>
            <tr>
              <th>Tool</th>
              <th>What it does</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>list_roots</code>
              </td>
              <td>
                Registered roots, <code>project_id</code>, model / dim, live
                file counts, rebuild progress
              </td>
            </tr>
            <tr>
              <td>
                <code>search</code>
              </td>
              <td>
                Hybrid search. Optional <code>root</code>,{' '}
                <code>type</code> (<code>text</code> / <code>image</code>),{' '}
                <code>top_k</code> (default 8, max 50). Default scope is every
                registered project.
              </td>
            </tr>
            <tr>
              <td>
                <code>read</code>
              </td>
              <td>
                One chunk by <code>id</code>, or every chunk of a{' '}
                <code>path</code> (ascending). Images ≤ 4 MiB return as MCP
                image content.
              </td>
            </tr>
          </tbody>
        </table>
        <h3 id="agent-skills">Agent skills</h3>
        <p>
          The same <code>SKILL.md</code> works in Claude Code, Codex, Cursor,
          Grok, and Kimi. It tells the agent to install or start the CLI if
          needed, then call <code>list_roots</code> → <code>search</code> →{' '}
          <code>read</code> instead of walking the filesystem. Canonical path:{' '}
          <code>skills/vane/SKILL.md</code>.
        </p>
        <CodeBlock lang="bash" title="install skill into local agent runtimes" code={SKILL_INSTALL} />
        <CodeBlock lang="text" title="plugin install (Claude / Codex / Kimi)" code={SKILL_PLUGINS} />
        <Callout type="warning" title="Local single-user trust">
          Anyone who can open the Unix socket (mode 0600, same uid) can
          search and read every indexed file. There is no token and no ACL in
          v1.
        </Callout>

        <h2 id="config">Project vs global config</h2>
        <p>
          Global defaults live in <code>~/.vane/config/config.toml</code>.
          Per-root policy belongs in <code>&lt;root&gt;/.vane.toml</code>:
        </p>
        <CodeBlock lang="toml" title=".vane.toml" code={PROJECT_TOML} />
        <ul className="sc-list">
          <li>
            <code>exclude</code> is a <strong>union</strong> of global and
            project globs. A project cannot turn off <code>node_modules</code>.
          </li>
          <li>
            <code>[[types]]</code> / <code>include</code> <strong>replace</strong>{' '}
            the global type table. Narrowing a project to markdown drops
            images unless you add them back.
          </li>
          <li>
            <code>[embed]</code> and <code>[chunk]</code> overlay field by
            field. Changing the model or dim rebuilds that project&apos;s
            index; extract cache is reused when the chunker did not change.
          </li>
        </ul>
        <p>
          Day-to-day: <code>vane include add</code>,{' '}
          <code>vane exclude add</code>, <code>vane model --yes</code>. Pass{' '}
          <code>--global</code> to edit defaults instead of the current
          project. Preview resolved policy with <code>vane inspect</code>.
        </p>

        <h2 id="watch-cas">Watch, git checkouts, and cache</h2>
        <p>
          One process, one OS file watcher. Excluded directories (
          <code>node_modules</code>, <code>.git</code>, <code>target</code>, …)
          are never registered. A git checkout is just file add / delete /
          rename: the same bytes hash to the same extract and embed cache
          keys, so switching branches does not re-embed unchanged files.
          Unreferenced cache entries stay until <code>vane gc</code> or the
          TTL (default 365 days). Preview with <code>vane gc --dry-run</code>.
          GC never deletes your source files — only data under{' '}
          <code>$VANE_HOME/rag</code>. <code>vane df</code> shows home, CAS,
          and per-project db sizes.
        </p>

        <h2 id="commands">Command map</h2>
        <table className="sc-table">
          <thead>
            <tr>
              <th>Command</th>
              <th>Purpose</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>vane init</code>
              </td>
              <td>Wizard: config, first root, excludes, optional user service</td>
            </tr>
            <tr>
              <td>
                <code>vane add</code> / <code>vane rm</code>
              </td>
              <td>
                Register or unregister a folder. <code>rm</code> keeps the
                project db until <code>gc</code>
              </td>
            </tr>
            <tr>
              <td>
                <code>vane start</code> / <code>stop</code> / <code>daemon</code>
              </td>
              <td>Start/stop; <code>daemon</code> is the foreground process</td>
            </tr>
            <tr>
              <td>
                <code>vane status</code>
              </td>
              <td>
                TTY dashboard (daemon, roots, live files, skips). JSON if piped
              </td>
            </tr>
            <tr>
              <td>
                <code>vane doctor</code>
              </td>
              <td>Diagnose home, daemon, embedder, roots, and disk</td>
            </tr>
            <tr>
              <td>
                <code>vane query</code>
              </td>
              <td>
                CLI search of the current project, <code>--root</code>, or{' '}
                <code>--all</code>. <code>--verbose</code> shows hit ids. Empty
                hits print a why line
              </td>
            </tr>
            <tr>
              <td>
                <code>vane read &lt;n&gt;</code>
              </td>
              <td>
                Print the n-th hit of the last TTY query.{' '}
                <code>--file</code> reads the source file (ignores staleness)
              </td>
            </tr>
            <tr>
              <td>
                <code>vane watch</code>
              </td>
              <td>
                Foreground poll of live-set / dirty changes.{' '}
                <code>--root</code>, <code>--all</code>,{' '}
                <code>--interval-ms</code> (100–60000)
              </td>
            </tr>
            <tr>
              <td>
                <code>vane issues</code>
              </td>
              <td>
                Skipped files. <code>--root</code> or <code>--all</code>
              </td>
            </tr>
            <tr>
              <td>
                <code>vane logs</code>
              </td>
              <td>
                Redacted daemon logs. <code>--follow</code>,{' '}
                <code>--lines</code> (default 50)
              </td>
            </tr>
            <tr>
              <td>
                <code>vane inspect</code>
              </td>
              <td>
                Resolved embed / chunk / exclude / types.{' '}
                <code>--root</code> or <code>--global</code>
              </td>
            </tr>
            <tr>
              <td>
                <code>vane mcp</code>
              </td>
              <td>
                stdio MCP bridge. <code>vane mcp install [--dry-run]
                [--client claude|cursor|codex]</code>
              </td>
            </tr>
            <tr>
              <td>
                <code>vane model</code>
              </td>
              <td>
                Change embed provider / model / dim and rebuild.{' '}
                <code>--yes</code> skips the confirm (required if not a TTY)
              </td>
            </tr>
            <tr>
              <td>
                <code>vane df</code>
              </td>
              <td>
                Disk usage for <code>$VANE_HOME</code>, CAS, and per-project dbs
              </td>
            </tr>
            <tr>
              <td>
                <code>vane gc</code>
              </td>
              <td>
                Compact and drop unreferenced CAS. <code>--dry-run</code> counts
                only. <code>--all</code> for every project
              </td>
            </tr>
            <tr>
              <td>
                <code>vane service uninstall</code>
              </td>
              <td>Remove the user service; keeps config and index data</td>
            </tr>
          </tbody>
        </table>

        <h2 id="library">When to use the library instead</h2>
        <p>
          Embed Vane inside Node, Go, or the browser when you already have
          vectors and want an in-process index. See{' '}
          <Link to="/quickstart">Quick Start</Link> and the{' '}
          <Link to="/guides/hybrid-search">Hybrid Search</Link> guide. The
          sidecar is the out-of-the-box path for local folders and agents.
        </p>
      </article>
    </DocsLayout>
  );
}
