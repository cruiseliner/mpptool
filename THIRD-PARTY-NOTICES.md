# Third-party notices

mpptool's own source code is dual-licensed under MIT OR Apache-2.0. Release binaries also contain
code from the following libraries, which are under their own licenses:

| Library | License | Used for |
|---|---|---|
| [mpxj-rs](https://crates.io/crates/mpxj-rs) — Rust port of [MPXJ](https://github.com/joniles/mpxj) | LGPL-2.1-or-later | Reading MPP/MPX/MSPDI files (default engine) |
| [mpxrust](https://crates.io/crates/mpxrust) | LGPL-2.1-or-later | Alternative MPP14 reader (`mpxrust` cargo feature, on by default) |
| clap, serde, serde_json, csv, chrono, libc and their dependencies | MIT and/or Apache-2.0 | CLI, serialization, dates |

## LGPL obligations for binaries

Rust links libraries statically. The LGPL lets you distribute such a binary as long as recipients can
replace the LGPL library and relink. Because mpptool's complete source code is public, you can do this:

1. Clone this repository.
2. Point `Cargo.toml` at a modified copy of `mpxj-rs`/`mpxrust`, for example with a `[patch.crates-io]` entry.
3. Run `cargo build --release`.

The LGPL library sources are available from crates.io and from their repositories linked above. The full
LGPL-2.1 text is at <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html>.

To build a binary without mpxrust, run `cargo build --release --no-default-features`. mpxj-rs is still
required.
