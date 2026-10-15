# AGENTS.md — guide for AI coding agents

This file tells automated agents (Codex, Claude Code, Cursor, Copilot, Grok, …) how to work on this repository.

## What this is
`mpptool` is a Rust CLI and library that reads Microsoft Project files (`.mpp`, `.mpx`, MSPDI `.xml`) and Primavera P6 files (`.xer`, P6 XML; our own reader in `src/read_p6.rs`) into an engine-neutral model (`src/model.rs`). It writes Microsoft Project XML (`src/mspdi_writer.rs`) and can also schedule new plans from CSV (`src/from_csv.rs`).

## Commands (run all before declaring a task done)
```bash
cargo build
cargo fmt --all                                              # rustfmt.toml: max_width=140
cargo clippy --all-targets -- -D warnings    # must be clean
cargo test                                                   # unit + tests/cli.rs
```
For reader or writer changes, also run the real-file corpus. It needs network access the first time.
```bash
scripts/fetch-corpus.sh
cargo build --release && scripts/test-corpus.sh   # expect: read_failures=3 (password-protected) xml_not_wellformed=0 roundtrip_mismatch=0
scripts/test-corpus-p6.sh                         # expect: read_failures=0 xml_not_wellformed=0 roundtrip_mismatch=0
MPXJ_DATA=$PWD/corpus/mpxj/junit/data cargo test --test cli --test p6
scripts/test-corpus-features.sh                    # report commands over every file; expect: feature_failures=0
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
- Primavera: `read_p6.rs` parses XER (`%T/%F/%R` tables, `clndr_data` calendar mini-language) or P6 XML (roxmltree) into `P6Data`, then `to_plan` maps a single project onto `Plan`. P6 ids are remapped to sequential UIDs. P6 durations, lags and float are in hours.
- Report commands live in their own modules (`gantt.rs`, `check.rs`, …): they take a `&Plan` and return a `String` or a serializable struct; `main.rs` only parses arguments and writes files. Never change the output of existing commands; add fields/commands instead. New `Plan`/`TaskRec` fields must not change existing JSON output (`#[serde(skip)]` or not serialized).
- `tests/fixtures/team.csv` builds (via `from-csv`) a plan where Alice is double-booked in the week of 2026-05-04 — use it for resource-load tests.
- `tests/fixtures/status-v2.xml` is a later version of `status.xml` (one task added, one renamed/relinked, dates and % changed) for `compare` tests.
- `tests/fixtures/status.xml` is a hand-written MSPDI plan with a status date, baselines, float, costs, a constraint, SS+lag link and an unassigned task — use it for report tests.
- `board.rs` and `portfolio.rs` reuse `check::assess` and `variance::variance`; RAG thresholds are documented in README — change both together. `portfolio` must never abort on one bad file: record it in `skipped`.
- Tests run in parallel. Give every test its own temp file names.
- The `mpxrust` reader was removed in 0.6.1. `--engine mpxj` stays as a no-op; `--engine mpxrust` must keep returning a clear error. Don't reintroduce the dependency.
- `mpxj-rs` is a method-by-method port of Java MPXJ. Its API names mirror Java (`get_unique_id`, `get_predecessors`), and its source is in `~/.cargo/registry/src/*/mpxj-rs-*/src`.
- Task UID 0 is the project summary task. Rows with no name and no dates are "null tasks" (blank rows) and are written as `<IsNull>1</IsNull>`.
- Visible task IDs (`id`) can change when rows are reordered. UIDs are stable, and MSPDI links use UIDs.
- Good first tasks are listed under "Roadmap" in README.md.
