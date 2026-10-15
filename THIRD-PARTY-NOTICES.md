# Third-party notices

mpptool's own source code is dual-licensed under MIT OR Apache-2.0. Release binaries also contain
code from the following libraries, which are under their own licenses:

| Library | License | Used for |
|---|---|---|
| [mpxj-rs](https://crates.io/crates/mpxj-rs) — Rust port of [MPXJ](https://github.com/joniles/mpxj) | LGPL-2.1-or-later | Reading MPP/MPX/MSPDI files |
| clap, serde, serde_json, csv, chrono, libc, roxmltree, encoding_rs and their dependencies | MIT and/or Apache-2.0 (encoding_rs: (Apache-2.0 OR MIT) AND BSD-3-Clause) | CLI, serialization, dates, P6 XML parsing, XER text decoding |
| rust_xlsxwriter, zip, flate2, zopfli, zlib-rs and their dependencies | MIT and/or Apache-2.0 (zlib-rs: Zlib) | Writing `.xlsx` workbooks (`to-xlsx`) |

## LGPL obligations for binaries

Rust links libraries statically. The LGPL lets you distribute such a binary as long as recipients can
replace the LGPL library and relink. Because mpptool's complete source code is public, you can do this:

1. Clone this repository.
2. Point `Cargo.toml` at a modified copy of `mpxj-rs`, for example with a `[patch.crates-io]` entry.
3. Run `cargo build --release`.

The LGPL library sources are available from crates.io and from their repositories linked above. The full
LGPL-2.1 text is at <https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html>.

