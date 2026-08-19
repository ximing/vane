import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import { Link } from 'react-router-dom';
import './Sidecar.css';

const INSTALL_RELEASE = `# macOS (Apple Silicon / Intel) and Linux x86_64
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-cli.sh | sh
# installs to ~/.local/bin/vane  (override with PREFIX=/usr/local)

vane --version`;

const INSTALL_SOURCE = `git clone https://github.com/ximing/vane.git
cd vane
cargo install --path crates/vane --locked --force
# or, without cloning:
cargo install --git https://github.com/ximing/vane.git --locked --bin vane`;

const INIT = `vane init
# 1. Embedding provider: ollama (default) or openai_compat
# 2. First project root (can skip)
# 3. Exclude globs (defaults include .git, node_modules, target, *.log, .env, …)
# 4. Enable image types? (default no)  /  Install user service? (default yes)

vane add ~/notes          # if you skipped the first root
vane start                # if the user service is not installed
vane status`;

const QUERY = `# Search the current project (cwd must sit inside a registered root)
vane query "how does auth work" --top-k 8

# One registered root, or every root fused with RRF
vane query "release checklist" --root ~/notes
vane query "release checklist" --all

# Filter by extractor name (not file suffix)
vane query "logo" --type image`;

const MCP = `{
  "mcpServers": {
    "vane": {
      "command": "vane",
      "args": ["mcp"]
    }
  }
}`;

const PROJECT_TOML = `# <root>/.vane.toml  — checked into the repo, never put api_key here
[embed]
provider = "openai_compat"
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"

[chunk]
max_chars = 800

exclude = ["**/generated/**"]
include = ["**/*.{md,rst}"]`;

const SKILL_HINT = `Copy crates/vane/SKILL.md into the agent's skill directory, or point
the agent at it. The skill says: call list_roots, then search, then
read. Do not walk the filesystem.`;

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
          one:
        </p>
        <CodeBlock lang="bash" title="install from GitHub Release" code={INSTALL_RELEASE} />
        <p>
          Until a release includes the CLI tarball, or on an unsupported
          arch, build from source (needs a Rust toolchain):
        </p>
        <CodeBlock lang="bash" title="install from source" code={INSTALL_SOURCE} />

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
          key in a project <code>.vane.toml</code> is rejected.
        </p>

        <h2 id="search">Search from the CLI</h2>
        <p>
          The daemon must already be running. <code>vane query</code> defaults
          to the registered root that contains your current directory; pass{' '}
          <code>--all</code> to fuse hits across every project with RRF.
        </p>
        <CodeBlock lang="bash" title="vane query" code={QUERY} />
        <p>
          If the embedding provider is down, search falls back to BM25 and
          marks hits <code>degraded</code>. Already-indexed vectors stay on
          disk.
        </p>

        <h2 id="mcp">MCP and agent skill</h2>
        <p>
          <code>vane mcp</code> is a stdio JSON-RPC 2.0 bridge to{' '}
          <code>~/.vane/run/vane.sock</code>. It does not embed the index
          engine. Add this to Claude Code / Cursor / other MCP clients:
        </p>
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
        <CodeBlock lang="text" title="crates/vane/SKILL.md" code={SKILL_HINT} />
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
          <code>vane exclude add</code>, <code>vane model</code>. Pass{' '}
          <code>--global</code> to edit defaults instead of the current
          project.
        </p>

        <h2 id="watch-cas">Watch, git checkouts, and cache</h2>
        <p>
          One process, one OS file watcher. Excluded directories (
          <code>node_modules</code>, <code>.git</code>, <code>target</code>, …)
          are never registered. A git checkout is just file add / delete /
          rename: the same bytes hash to the same extract and embed cache
          keys, so switching branches does not re-embed unchanged files.
          Unreferenced cache entries stay until <code>vane gc</code> or the
          TTL (default 365 days). GC never deletes your source files — only
          data under <code>$VANE_HOME/rag</code>.
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
                <code>vane query</code>
              </td>
              <td>CLI search of the current project, <code>--root</code>, or <code>--all</code></td>
            </tr>
            <tr>
              <td>
                <code>vane mcp</code>
              </td>
              <td>stdio MCP bridge to the running daemon</td>
            </tr>
            <tr>
              <td>
                <code>vane model</code>
              </td>
              <td>Change embed provider / model / dim and rebuild</td>
            </tr>
            <tr>
              <td>
                <code>vane gc</code>
              </td>
              <td>
                Compact and drop unreferenced CAS. <code>--all</code> for every
                project
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
