# AGENTS.md — guide for AI coding agents

This file tells automated agents (Codex, Claude Code, Cursor, Copilot, Grok, …) how to work on this repository.

## What this is
`mpptool` is a Rust CLI and library that reads Microsoft Project files (`.mpp`, `.mpx`, MSPDI `.xml`) into an engine-neutral model (`src/model.rs`). It writes Microsoft Project XML (`src/mspdi_writer.rs`) and can also schedule new plans from CSV (`src/from_csv.rs`).

## Commands (run all before declaring a task done)
```bash
cargo build
cargo fmt --all                                              # rustfmt.toml: max_width=140
cargo clippy --all-targets --all-features -- -D warnings    # must be clean
cargo test                                                   # unit + tests/cli.rs
```
For reader or writer changes, also run the real-file corpus. It needs network access the first time.
```bash
scripts/fetch-corpus.sh
cargo build --release && scripts/test-corpus.sh   # expect: read_failures=3 (password-protected) xml_not_wellformed=0 roundtrip_mismatch=0
MPXJ_DATA=$PWD/corpus/mpxj/junit/data cargo test --test cli
```

## Rules
- **Never commit** `corpus/`, `target/`, `dist/`, or any third-party `.mpp` file. MPXJ's samples are LGPL and are fetched at test time.
- Do not add dependencies with licenses incompatible with MIT/Apache-2.0 for our code. The LGPL readers are the only exception, and they are documented in THIRD-PARTY-NOTICES.md.
- Library code must not panic on bad input. Return `Err(String)` with a message that names the file.
- Keep MSPDI element order identical to what Microsoft Project writes. Cost values in MSPDI are in hundredths. `LinkLag` is in tenths of a minute. Durations are `PT#H#M#S`.
- To add a field: model → reader(s) → writer → test (fixture or unit). Update the README's "What carries over" table.
- Keep CLI output stable. Scripts parse `--json` and `--csv`, so add new fields instead of renaming existing ones.
- Update `CHANGELOG.md` for user-visible changes.

## Useful facts
- `mpxj-rs` is a method-by-method port of Java MPXJ. Its API names mirror Java (`get_unique_id`, `get_predecessors`), and its source is in `~/.cargo/registry/src/*/mpxj-rs-*/src`.
- Task UID 0 is the project summary task. Rows with no name and no dates are "null tasks" (blank rows) and are written as `<IsNull>1</IsNull>`.
- Visible task IDs (`id`) can change when rows are reordered. UIDs are stable, and MSPDI links use UIDs.
- Good first tasks are listed under "Roadmap" in README.md.
