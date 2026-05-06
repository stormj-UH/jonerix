# jpkg — jonerix package manager

**Version 2.2.1** — Rust from-scratch port of the C jpkg 1.1.5.

jpkg is the system package manager for jonerix.  It handles package creation,
installation, removal, dependency resolution, Ed25519 signature verification,
and license-gate enforcement for a merged-usr (`/usr → /`) flat filesystem.

## Architecture

```
src/
├── lib.rs           Crate root — #![forbid(unsafe_code)], #![deny(warnings)]
├── types.rs         Newtypes: Sha256Hash, InstallMode, OrphanMode
├── util.rs          SHA-256, version comparison, license gate, layout audit
├── recipe.rs        recipe.toml + .jpkg metadata + INDEX TOML parsing
├── archive.rs       .jpkg wire format: read, write, extract (with path-traversal guards)
├── sign.rs          Ed25519 keypair / sign / verify / PublicKeySet
├── canon.rs         Canonical-bytes construction for deterministic signing
├── db.rs            Installed-package database (fcntl-locked, C-format-compatible)
├── fetch.rs         Synchronous HTTPS (ureq + rustls, webpki-roots, no system CAs)
├── repo.rs          INDEX management, mirror config, signature policy
├── deps.rs          Dependency resolution (topological sort, cycle detection)
├── cmd/             Subcommand implementations
│   ├── install.rs   Network install with signature verification
│   ├── local_install.rs   Local .jpkg file install (signature-exempt)
│   ├── build.rs     Recipe build → .jpkg archive
│   ├── remove.rs    Package removal with orphan detection
│   ├── upgrade.rs   In-place upgrade
│   ├── update.rs    INDEX refresh
│   ├── search.rs    Package search
│   ├── info.rs      Package info query
│   ├── sign.rs      Sign a .jpkg in place
│   ├── verify.rs    Verify a .jpkg signature
│   ├── resign.rs    Bulk re-sign packages
│   ├── keygen.rs    Ed25519 keypair generation
│   ├── license_audit.rs   Transitive license compliance audit
│   └── build_world.rs     Full-tree build orchestration
└── bin/
    ├── jpkg.rs      Network subcommand dispatcher
    └── jpkg-local.rs   Local-only subcommand dispatcher
```

## Wire Format

```
[MAGIC      8B]  "JPKG\x00\x01\x00\x00"
[HDR_LEN    4B]  u32 little-endian
[METADATA   var] UTF-8 TOML (hdr_len bytes, NOT NUL-terminated)
[PAYLOAD    var] zstd-compressed tar archive (to EOF)
```

Byte-compatible with C jpkg 1.1.5.  Existing installed-package databases
(`/var/db/jpkg/installed/`) are read without migration.

## Security

### Unsafe Code Audit

**Zero `unsafe` blocks.**  The crate root declares `#![forbid(unsafe_code)]`.
All dependencies that use `unsafe` internally (ed25519-dalek, zstd, tar, nix)
are well-audited, widely-deployed crates from the permissive-license ecosystem.

### Security Hardening (2.2.1)

| Finding | Severity | Mitigation |
|---------|----------|------------|
| Path traversal via crafted tar entries (`../`) | Critical | `validate_entry_path` rejects `..` that escapes dest |
| Symlink escape (create symlink then write through it) | High | `validate_symlink_target` uses depth arithmetic to reject escapes |
| Integer overflow in `hdr_len` on 32-bit | Medium | `checked_add` with explicit overflow guard |
| Predictable temp file (symlink attack on `.tmp`) | Medium | Uses `tempfile` crate with `O_EXCL` for atomic writes |
| TOCTOU between verify and extract | Medium | Signature verified on in-memory bytes, not re-read from disk |
| Non-UTF-8 metadata panic | Low | `metadata_str()` returns `Result`; `metadata()` panics documented |

### Signature Verification

- **Default policy: Require** — invalid or missing signatures are hard errors
- **Canonical bytes**: `"jpkg-canon\0" + CANON_VERSION(1B) + payload_sha256(32B) + metadata_toml`
- **Key format**: 32-byte raw Ed25519 public keys in `/etc/jpkg/keys/*.pub`
- **jpkg-local exemption**: local installs skip signature checks (trusted source)

### TLS

- Minimum TLS 1.2 (ring provider, rustls)
- Compiled-in Mozilla CA bundle (webpki-roots) — no system CA dependency
- 30-second connect + read timeouts
- Up to 10 redirects

## Building

```sh
# Inside the jonerix build container:
cargo build --release --frozen --target x86_64-unknown-linux-musl \
    --bin jpkg --bin jpkg-local
```

Produces two statically-linked musl binaries:
- `/bin/jpkg` — network subcommands
- `/bin/jpkg-local` — local-only subcommands
- `/bin/jpkg-conform` — version pinning (mksh script)

## Testing

```sh
cargo test          # 215 unit tests + 1 doc-test
```

Set `JPKG_FETCH_TESTS=1` to enable network-dependent tests.

## License

MIT — Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
doing business as LAVA GOAT SOFTWARE.

## Dependencies

All runtime dependencies are permissive-licensed (MIT, Apache-2.0, BSD, ISC).
Zero GPL code at any level.  The `jpkg license-audit` subcommand enforces
this transitively at build time.
