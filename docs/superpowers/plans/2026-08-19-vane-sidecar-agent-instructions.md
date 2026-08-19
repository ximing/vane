# Vane sidecar agent standing instructions

You are a **worker**. The controller does not implement. Follow TDD. Do not change `vane-core` public API, persistence, or Won't-have.

## Documents (read these, do not invent)

- Spec: `docs/superpowers/specs/2026-08-19-vane-sidecar-design.md`
- Plan: `docs/superpowers/plans/2026-08-19-vane-sidecar.md`
- This file

Implement **only** the assigned plan task. File map and signatures in the plan are binding.

## TDD (required)

1. Write the failing test first (exact cases from the plan task).
2. Run it. Confirm it fails for the expected reason (missing type/module/assertion). Record RED.
3. Write the minimum implementation.
4. Run the same test. Confirm GREEN.
5. Run `cargo test -p vane` and `cargo clippy -p vane --all-targets -- -D warnings` before you finish.
6. `cargo fmt` on files you touched.

If you skip RED, the task is not done.

## Host isolation (required — do not pollute the machine)

- **Never** read or write `~/.vane`, `$HOME/.vane`, or the user's real config/data dirs.
- Every test sets `VANE_HOME` (or `resolve_home` / `--home`) to a **unique temp directory** you create and delete (`std::env::temp_dir()` + unique name, or a `Drop` guard).
- Daemon / socket / pid / log tests must live under that temp home.
- Do not install launchd/systemd on the host in tests. Service tests write plists under the temp home or skip the real `launchctl` when not in a harness — prefer pure file-level install/uninstall against a fake `HOME`/`XDG` if you must.
- Do not call real Ollama or OpenAI. Use `MockEmbedder` or a local `TcpListener` fake.
- Do not `git add` unrelated files, secrets, or `target/`.

## Dependencies

- No `tokio`, `openssl`, `regex`, `globset`, `native-tls`.
- Glob: in-crate `glob_match` only.
- HTTP: `ureq` with `default-features = false`, features `json` + `tls`.
- Do not add `crates/vane` to wasm32 CI jobs.

## Git

- Commit only that task's files when the plan step says commit.
- Message style from the plan.
- Do not push. Do not merge to `main`.

## Report contract (final message)

- **Status:** `DONE` | `DONE_WITH_CONCERNS` | `BLOCKED` | `NEEDS_CONTEXT`
- Commits (short SHA + subject)
- Tests: RED command/output reason; GREEN command/result
- Isolation: confirm no `~/.vane` writes
- Concerns, if any

Reviewers: set `needs_fix=true` if spec gap, missing TDD evidence, host isolation leak, or Important/Critical quality issue. Do not re-run the implementer's tests. Read the diff and tests.
