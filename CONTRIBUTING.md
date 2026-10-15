# Contributing to mpptool

Thanks for helping! Bug reports with a sample file are the most valuable contribution. Remove any confidential data from a file before attaching it, because issue attachments are public.

## Development setup

```bash
git clone https://github.com/cruiseliner/mpptool && cd mpptool
cargo build
cargo test
```

The minimum Rust version is 1.88. Before you open a PR, run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

### Testing against real files

Real `.mpp` samples come from MPXJ's LGPL test suite. They are **downloaded on demand and never committed**:

```bash
scripts/fetch-corpus.sh                                    # sparse clone into ./corpus (git-ignored)
MPXJ_DATA=$PWD/corpus/mpxj/junit/data cargo test --test cli
cargo build --release && scripts/test-corpus.sh            # all ~600 files: read, convert, round-trip
```

A change to a reader or writer must keep `scripts/test-corpus.sh` and `scripts/test-corpus-p6.sh` at `xml_not_wellformed=0 roundtrip_mismatch=0`.

## Code layout

| Path | Purpose |
|---|---|
| `src/model.rs` | Engine-neutral `Plan` model (tasks, resources, assignments, calendars) |
| `src/read_mpxj.rs` | Reader using `mpxj-rs` (MPP/MPX/MSPDI) → `Plan` |
| `src/read_mpxrust.rs` | Alternative MPP14 reader using `mpxrust` (feature `mpxrust`) |
| `src/read_p6.rs` | Primavera P6 XER / P6 XML reader → `Plan` |
| `src/mspdi_writer.rs` | `Plan` → Microsoft Project XML |
| `src/from_csv.rs` | CSV parser and scheduler (forward/backward pass, critical path) |
| `src/main.rs` | CLI (clap) and table/CSV/JSON output |
| `tests/cli.rs` | End-to-end tests of the binary |
| `tests/fixtures/` | Small fixtures we wrote ourselves, so they are freely redistributable |

## Guidelines

- Add each new field in three places: the model, the reader(s), and the writer. Add a test that round-trips it, either in `tests/fixtures/` or as a unit test.
- MSPDI element order matters to Microsoft Project. Follow the order Project itself writes (see comments in `mspdi_writer.rs`).
- Only commit test files that you created yourself or that are clearly redistributable under a compatible license.
- Keep error messages user-facing: start them with the file name, say what went wrong, and avoid panics.
- By contributing, you agree that your contribution is dual-licensed under MIT OR Apache-2.0.
