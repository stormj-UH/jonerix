//! jpkg — jonerix package manager (Rust port of jpkg 1.1.5).
//!
//! Goal: byte-equivalent CLI behaviour at every CI call site
//! (`scripts/ci-full-bootstrap.sh`, `scripts/build-all.sh`,
//! `.github/workflows/{full-bootstrap,publish-packages}.yml`).
//!
//! Module map (Rust ↔ C, both relative to `packages/`):
//!
//! - `util`    ↔ jpkg/src/util.c   (logging, sha256, paths, version cmp, license gate, layout audit)
//! - `recipe`  ↔ jpkg/src/toml.c   (recipe.toml + .jpkg metadata + INDEX TOML — uses `toml` crate)
//! - `archive` ↔ jpkg/src/pkg.c    (.jpkg format: magic + LE32 hdr_len + TOML + zstd(tar))
//! - `sign`    ↔ jpkg/src/sign.c   (Ed25519 sign / verify / detached-sig / keygen)
//! - `db`      ↔ jpkg/src/db.c     (installed-pkg state at /var/db/jpkg/installed)
//! - `fetch`   ↔ jpkg/src/fetch.c  (HTTPS via ureq+rustls)
//! - `repo`    ↔ jpkg/src/repo.c   (INDEX TOML, mirror config, sig-verified INDEX.zst)
//! - `deps`    ↔ jpkg/src/deps.c   (topological sort, cycle detection, removal-order)
//! - `cmd`     ↔ jpkg/src/cmd_*.c  (one sub-module per subcommand)
//!
//! .jpkg archive layout (preserved from C jpkg, see audit § 2):
//! ```text
//! [MAGIC: 8 bytes] "JPKG\x00\x01\x00\x00"
//! [HEADER_LEN: 4 bytes LE32]
//! [METADATA: header_len bytes of TOML, UTF-8, not NUL-terminated]
//! [PAYLOAD: zstd-compressed tar archive — to end of file]
//! ```

#![allow(dead_code)]

pub mod util;
pub mod recipe;
pub mod archive;
pub mod sign;

pub mod db;
pub mod fetch;
pub mod repo;
pub mod deps;

pub mod cmd;

// Phase-3 CLI sub-module:
// pub mod cmd;

/// `JPKG\x00\x01\x00\x00` — see jpkg/src/pkg.h:17-20.  First 4 bytes are the
/// ASCII tag; next 4 are version major/minor in two LE16s (currently 1, 0).
pub const JPKG_MAGIC: [u8; 8] = *b"JPKG\x00\x01\x00\x00";
