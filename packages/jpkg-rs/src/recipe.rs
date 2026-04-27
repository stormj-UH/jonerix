//! Recipe / metadata TOML layer — Rust port of jpkg/src/toml.c + pkg.c (parse→struct).
//!
//! Three public types:
//! - [`Recipe`]     — input to `jpkg build` (`recipe.toml`)
//! - [`Metadata`]   — embedded inside a `.jpkg` archive / installed DB entry
//! - [`Index`]      — public package index (one `[pkgname-arch]` section per package)
//!
//! Uses the `toml` + `serde` crates; no hand-rolled parser.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

// ─── Legacy-escape sanitiser ─────────────────────────────────────────────────
//
// The C jpkg's hand-rolled TOML parser is lenient about backslash escapes
// inside basic strings: `\;` (in `find -exec ... \;`), `\.` (in regex), `\(`,
// `\$`, etc. all pass through as literal two-byte sequences.  The standard
// `toml` crate is correct to reject these per TOML spec, but the existing
// recipe corpus (36+ recipes) relies on the lenient behaviour, with several
// recipes containing comments that document the parser's laxity (e.g.
// `packages/extra/nloxide/recipe.toml:220-221`,
// `packages/core/anvil/recipe.toml:108-111`).
//
// Sanitiser: walk the input once, identify positions that are inside a
// basic-string literal (`"…"` or `"""…"""`), and double any backslash whose
// next char is NOT a valid TOML escape (`b/f/n/r/t/"/\\/u/U/<newline>`).
// Doubling `\X` to `\\X` makes the standard parser see a literal `\` followed
// by the original character — exactly what the C parser produced in practice.
// Literal strings (`'…'`, `'''…'''`) are left alone because escapes don't
// process there.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SanitizeState {
    Outside,
    Comment,        // `#` to end-of-line, only when reached from Outside
    BasicSingle,    // "…"
    BasicMulti,     // """…"""
    LiteralSingle,  // '…'
    LiteralMulti,   // '''…'''
}

/// True iff `b` is a valid escape character after `\` in a TOML basic string.
/// `u` and `U` start `\uXXXX` / `\UXXXXXXXX` sequences; `\<newline>` is a
/// line-continuation in multi-line basic strings.
#[inline]
fn is_valid_basic_escape(b: u8) -> bool {
    matches!(b, b'b' | b'f' | b'n' | b'r' | b't' | b'"' | b'\\' | b'u' | b'U' | b'\n' | b'\r')
}

/// Pre-process a TOML document so that the standard `toml` crate accepts
/// non-conformant escape sequences that C jpkg's hand-rolled parser used to
/// pass through.  Returns `Cow::Borrowed` when the input is already strict-
/// conformant (no doubling needed); otherwise returns an owned, doubled copy.
pub(crate) fn sanitize_legacy_escapes(input: &str) -> Cow<'_, str> {
    // Quick early-out: no backslashes => no work to do.
    if !input.contains('\\') {
        return Cow::Borrowed(input);
    }

    let bytes = input.as_bytes();
    let n = bytes.len();
    let mut out: Vec<u8> = Vec::with_capacity(n + 16);
    let mut state = SanitizeState::Outside;
    let mut i = 0usize;
    let mut changed = false;

    while i < n {
        let b = bytes[i];

        match state {
            SanitizeState::Outside => {
                match b {
                    b'#' => {
                        out.push(b);
                        state = SanitizeState::Comment;
                        i += 1;
                    }
                    b'"' => {
                        // Look ahead for `"""` to distinguish multi-line basic.
                        if i + 2 < n && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                            out.extend_from_slice(b"\"\"\"");
                            state = SanitizeState::BasicMulti;
                            i += 3;
                        } else {
                            out.push(b'"');
                            state = SanitizeState::BasicSingle;
                            i += 1;
                        }
                    }
                    b'\'' => {
                        if i + 2 < n && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                            out.extend_from_slice(b"'''");
                            state = SanitizeState::LiteralMulti;
                            i += 3;
                        } else {
                            out.push(b'\'');
                            state = SanitizeState::LiteralSingle;
                            i += 1;
                        }
                    }
                    _ => {
                        out.push(b);
                        i += 1;
                    }
                }
            }
            SanitizeState::Comment => {
                out.push(b);
                if b == b'\n' {
                    state = SanitizeState::Outside;
                }
                i += 1;
            }
            SanitizeState::BasicSingle => {
                if b == b'\\' && i + 1 < n {
                    let nxt = bytes[i + 1];
                    if is_valid_basic_escape(nxt) {
                        out.push(b'\\');
                        out.push(nxt);
                    } else {
                        out.extend_from_slice(b"\\\\");
                        out.push(nxt);
                        changed = true;
                    }
                    i += 2;
                } else if b == b'"' {
                    out.push(b'"');
                    state = SanitizeState::Outside;
                    i += 1;
                } else if b == b'\n' {
                    // Single-line basic strings can't contain newlines per spec;
                    // pass through and let the toml crate error appropriately.
                    out.push(b);
                    state = SanitizeState::Outside;
                    i += 1;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
            SanitizeState::BasicMulti => {
                if b == b'\\' && i + 1 < n {
                    let nxt = bytes[i + 1];
                    if is_valid_basic_escape(nxt) {
                        out.push(b'\\');
                        out.push(nxt);
                    } else {
                        out.extend_from_slice(b"\\\\");
                        out.push(nxt);
                        changed = true;
                    }
                    i += 2;
                } else if b == b'"' && i + 2 < n && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                    // TOML allows up to two trailing double-quotes inside the
                    // body of a multi-line basic string before the closing
                    // `"""`.  We're being conservative and treating any `"""`
                    // as the closer; this matches what every real recipe does.
                    out.extend_from_slice(b"\"\"\"");
                    state = SanitizeState::Outside;
                    i += 3;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
            SanitizeState::LiteralSingle => {
                if b == b'\'' {
                    state = SanitizeState::Outside;
                }
                out.push(b);
                i += 1;
            }
            SanitizeState::LiteralMulti => {
                if b == b'\'' && i + 2 < n && bytes[i + 1] == b'\'' && bytes[i + 2] == b'\'' {
                    out.extend_from_slice(b"'''");
                    state = SanitizeState::Outside;
                    i += 3;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
        }
    }

    if changed {
        // Sanitiser only doubles existing valid UTF-8 bytes, never breaks codepoints.
        Cow::Owned(String::from_utf8(out).expect("sanitiser preserves UTF-8"))
    } else {
        Cow::Borrowed(input)
    }
}

// ─── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RecipeError {
    Toml(toml::de::Error),
    Ser(toml::ser::Error),
    Io(std::io::Error),
    /// A required field was absent.
    Missing(&'static str),
    /// License string failed the permissive-only gate.
    BadLicense(String),
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecipeError::Toml(e) => write!(f, "TOML parse error: {e}"),
            RecipeError::Ser(e) => write!(f, "TOML serialise error: {e}"),
            RecipeError::Io(e) => write!(f, "I/O error: {e}"),
            RecipeError::Missing(field) => write!(f, "required field missing: {field}"),
            RecipeError::BadLicense(lic) => {
                write!(f, "non-permissive license rejected: {lic}")
            }
        }
    }
}

impl std::error::Error for RecipeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RecipeError::Toml(e) => Some(e),
            RecipeError::Ser(e) => Some(e),
            RecipeError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<toml::de::Error> for RecipeError {
    fn from(e: toml::de::Error) -> Self {
        RecipeError::Toml(e)
    }
}

impl From<toml::ser::Error> for RecipeError {
    fn from(e: toml::ser::Error) -> Self {
        RecipeError::Ser(e)
    }
}

impl From<std::io::Error> for RecipeError {
    fn from(e: std::io::Error) -> Self {
        RecipeError::Io(e)
    }
}

// ─── License gate (inlined from util.c:518-554) ──────────────────────────────

/// Permissive SPDX identifiers accepted by jonerix (mirrors `util.c`).
static PERMISSIVE_LICENSES: &[&str] = &[
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Apache-2.0",
    "0BSD",
    "CC0",
    "CC0-1.0",
    "Unlicense",
    "curl",
    "MirOS",
    "OpenSSL",
    "SSLeay",
    "zlib",
    "Zlib",
    "public domain",
    "Public-Domain",
    "BSD-2-Clause-Patent",
    "PSF-2.0",
    "BSL-1.0",
    "Artistic-2.0",
    "Ruby",
    "MPL-2.0",
    "Info-ZIP",
    "bzip2-1.0.6",
];

/// Returns `true` if `license` is (or resolves to) a permissive license.
///
/// Mirrors the C `license_is_permissive` logic exactly:
/// - Exact case-insensitive match against the table.
/// - SPDX OR: permissive if **any** component is permissive.
/// - SPDX AND: permissive only if **all** components are permissive.
pub fn license_is_permissive(license: &str) -> bool {
    // Exact match (case-insensitive).
    if PERMISSIVE_LICENSES
        .iter()
        .any(|&l| l.eq_ignore_ascii_case(license))
    {
        return true;
    }

    // SPDX OR: "MIT OR Apache-2.0" → permissive if any component is.
    if let Some(pos) = license.find(" OR ") {
        let left = &license[..pos];
        let right = &license[pos + 4..];
        return license_is_permissive(left) || license_is_permissive(right);
    }

    // SPDX AND: "MIT AND GPL-2.0" → permissive only if all components are.
    if let Some(pos) = license.find(" AND ") {
        let left = &license[..pos];
        let right = &license[pos + 5..];
        return license_is_permissive(left) && license_is_permissive(right);
    }

    false
}

// ─── Sub-structs shared between Recipe and Metadata ──────────────────────────

/// `[package]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageSection {
    /// Required.
    pub name: Option<String>,
    /// Required.
    pub version: Option<String>,
    /// Required (gate-checked by [`Recipe::validate`]).
    pub license: Option<String>,
    pub description: Option<String>,
    pub arch: Option<String>,
    /// Packages whose conflicting files this one silently takes ownership of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    /// Packages that cannot coexist with this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

/// `[source]` section — present only in `recipe.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceSection {
    /// Absent / "local" = no fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// SHA-256 hex of the downloaded tarball, verified post-fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// `[build]` section — present only in `recipe.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildSection {
    /// `autoconf`, `cmake`, `custom`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configure: Option<String>,
    #[serde(rename = "build", skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
}

/// `[depends]` section.
///
/// Note: `replaces` and `conflicts` also appear here in some historic recipes;
/// the C parser accepts both `package.replaces` and `depends.replaces`.  We
/// keep them in `PackageSection` (canonical) and ignore them here to avoid
/// duplication — downstream code should always look at `PackageSection`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependsSection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build: Vec<String>,
}

/// `[hooks]` section — shell commands run around install/remove.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_install: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_install: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_remove: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_remove: Option<String>,
}

/// `[files]` section — present only in `metadata.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesSection {
    /// SHA-256 hex of the zstd payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Byte length of the zstd payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

// ─── Recipe ──────────────────────────────────────────────────────────────────

/// Parsed `recipe.toml` — the input to `jpkg build`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(default)]
    pub package: PackageSection,
    #[serde(default)]
    pub source: SourceSection,
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub depends: DependsSection,
    #[serde(default, skip_serializing_if = "hooks_section_is_empty")]
    pub hooks: HooksSection,
}

fn hooks_section_is_empty(h: &HooksSection) -> bool {
    h.pre_install.is_none()
        && h.post_install.is_none()
        && h.pre_remove.is_none()
        && h.post_remove.is_none()
}

impl Recipe {
    /// Parse a recipe from a TOML string.
    ///
    /// Runs the input through [`sanitize_legacy_escapes`] first so the legacy
    /// recipe corpus (which depends on the C parser's escape laxity) parses
    /// cleanly under the strict-conformant `toml` crate.
    pub fn from_str(s: &str) -> Result<Self, RecipeError> {
        let sanitized = sanitize_legacy_escapes(s);
        let r: Recipe = toml::from_str(&sanitized)?;
        Ok(r)
    }

    /// Read and parse a recipe from a file path.
    pub fn from_file(path: &Path) -> Result<Self, RecipeError> {
        let s = std::fs::read_to_string(path)?;
        Self::from_str(&s)
    }

    /// Validate required fields and the permissive-license gate.
    ///
    /// - `package.name` must be non-empty.
    /// - `package.version` must be non-empty.
    /// - `package.license`, if present, must pass [`license_is_permissive`].
    ///   (The field itself is optional at the recipe stage — the build command
    ///   enforces it before creating the archive.  We mirror that lenience
    ///   here: absent = no error; present-but-bad = error.)
    pub fn validate(&self) -> Result<(), RecipeError> {
        match self.package.name.as_deref() {
            None | Some("") => return Err(RecipeError::Missing("package.name")),
            Some(s) if s.trim().is_empty() => return Err(RecipeError::Missing("package.name")),
            _ => {}
        }
        match self.package.version.as_deref() {
            None | Some("") => return Err(RecipeError::Missing("package.version")),
            Some(s) if s.trim().is_empty() => return Err(RecipeError::Missing("package.version")),
            _ => {}
        }
        if let Some(lic) = &self.package.license {
            if !license_is_permissive(lic) {
                return Err(RecipeError::BadLicense(lic.clone()));
            }
        }
        Ok(())
    }
}

// ─── Metadata ────────────────────────────────────────────────────────────────

/// Metadata embedded inside a `.jpkg` archive, and also written to
/// `/var/db/jpkg/installed/<name>/metadata.toml`.
///
/// Structurally identical to [`Recipe`] plus a `[files]` section.
/// Does NOT contain a `[source]` or `[build]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub package: PackageSection,
    #[serde(default)]
    pub depends: DependsSection,
    #[serde(default, skip_serializing_if = "hooks_section_is_empty")]
    pub hooks: HooksSection,
    #[serde(default)]
    pub files: FilesSection,
}

impl Metadata {
    /// Parse metadata from a TOML string (embedded in a `.jpkg` or read from disk).
    pub fn from_str(s: &str) -> Result<Self, RecipeError> {
        let sanitized = sanitize_legacy_escapes(s);
        let m: Metadata = toml::from_str(&sanitized)?;
        Ok(m)
    }

    /// Serialize metadata to a TOML string (to embed in a `.jpkg` or write to disk).
    pub fn to_string(&self) -> Result<String, RecipeError> {
        Ok(toml::to_string(self)?)
    }

    /// Build a `Metadata` from a validated `Recipe` plus the payload hash/size.
    ///
    /// Copies `package`, `depends`, and `hooks`; adds `files`.
    /// The `source` and `build` sections are intentionally dropped — they are
    /// not part of the installed-package metadata format.
    pub fn from_recipe(r: &Recipe, payload_sha256: String, payload_size: u64) -> Self {
        Metadata {
            package: r.package.clone(),
            depends: r.depends.clone(),
            hooks: r.hooks.clone(),
            files: FilesSection {
                sha256: Some(payload_sha256),
                size: Some(payload_size),
            },
        }
    }
}

// ─── Index ───────────────────────────────────────────────────────────────────

/// One entry in the public package INDEX.
///
/// The TOML section key is `pkgname-arch`; `arch` is stored explicitly inside
/// the entry.  The C code strips the `-arch` suffix from the key to recover the
/// package name — we reproduce that in [`Index::get`].
///
/// Note: the TOML field is `build-depends` (dash), mapped to the Rust field
/// `build_depends` (underscore) via `#[serde(rename = "build-depends")]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexEntry {
    pub version: String,
    pub license: String,
    pub description: String,
    pub arch: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<String>,
    /// Serialised as `build-depends` in the TOML (SPDX-style dash key).
    #[serde(
        rename = "build-depends",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub build_depends: Vec<String>,
}

/// Parsed package INDEX — a flat TOML document whose top-level tables are
/// keyed as `pkgname-arch`.
///
/// Round-trips via [`Index::parse`] / [`Index::to_string`] with deterministic
/// (BTreeMap, alphabetical) ordering.
#[derive(Debug)]
pub struct Index {
    /// Raw map: key is the full `pkgname-arch` string, value is the entry.
    pub entries: BTreeMap<String, IndexEntry>,
}

impl Index {
    /// Parse an INDEX from its TOML text representation.
    ///
    /// The INDEX is a single-level TOML document where each top-level key is a
    /// `pkgname-arch` string containing an inline table of fields.  We
    /// deserialise it as `BTreeMap<String, IndexEntry>`.
    pub fn parse(text: &str) -> Result<Self, RecipeError> {
        let sanitized = sanitize_legacy_escapes(text);
        let entries: BTreeMap<String, IndexEntry> = toml::from_str(&sanitized)?;
        Ok(Index { entries })
    }

    /// Serialise the INDEX back to TOML, with entries in alphabetical order.
    pub fn to_string(&self) -> Result<String, RecipeError> {
        Ok(toml::to_string(&self.entries)?)
    }

    /// Look up a package by name and arch.
    ///
    /// Tries the key `name-arch` directly, which is the canonical format
    /// written by `jpkg build`.
    pub fn get(&self, name: &str, arch: &str) -> Option<&IndexEntry> {
        let key = format!("{name}-{arch}");
        self.entries.get(&key)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    // ── 1. Round-trip the real jpkg recipe.toml ───────────────────────────────

    #[test]
    fn roundtrip_jpkg_recipe_toml() {
        let recipe_path = manifest_dir().join("../jpkg/recipe.toml");
        if !recipe_path.exists() {
            eprintln!("skipping: {:?} not found", recipe_path);
            return;
        }

        let original = std::fs::read_to_string(&recipe_path).expect("read recipe.toml");
        let recipe = Recipe::from_str(&original).expect("parse recipe");

        assert_eq!(recipe.package.name.as_deref(), Some("jpkg"));
        assert_eq!(recipe.package.version.as_deref(), Some("1.1.5"));
        assert_eq!(recipe.package.license.as_deref(), Some("MIT"));
        assert_eq!(recipe.depends.runtime, vec!["musl"]);
        assert!(recipe.package.replaces.contains(&"jpkg-local".to_string()));

        // Re-serialise then re-parse — fields must survive a round-trip.
        let serialised = toml::to_string(&recipe).expect("serialise");
        let recipe2 = Recipe::from_str(&serialised).expect("re-parse");
        assert_eq!(recipe2.package.name, recipe.package.name);
        assert_eq!(recipe2.package.version, recipe.package.version);
        assert_eq!(recipe2.depends.runtime, recipe.depends.runtime);
        assert_eq!(recipe2.package.replaces, recipe.package.replaces);
    }

    // ── 2. Round-trip an installed metadata.toml ──────────────────────────────

    #[test]
    fn roundtrip_metadata_toml() {
        let toml_str = r#"
[package]
name = "libressl"
version = "4.0.0"
license = "ISC"
description = "Free TLS/crypto stack from OpenBSD"
arch = "x86_64"

[depends]
runtime = ["musl"]
build = ["clang", "cmake", "samurai"]

[hooks]
post_install = "ldconfig /lib 2>/dev/null || true"

[files]
sha256 = "abc123def456abc123def456abc123def456abc123def456abc123def456abcd"
size = 1234567
"#;

        let meta = Metadata::from_str(toml_str).expect("parse metadata");
        assert_eq!(meta.package.name.as_deref(), Some("libressl"));
        assert_eq!(meta.files.sha256.as_deref(), Some("abc123def456abc123def456abc123def456abc123def456abc123def456abcd"));
        assert_eq!(meta.files.size, Some(1234567));
        assert_eq!(meta.hooks.post_install.as_deref(), Some("ldconfig /lib 2>/dev/null || true"));

        // Re-serialise and re-parse.
        let serialised = meta.to_string().expect("serialise");
        let meta2 = Metadata::from_str(&serialised).expect("re-parse");
        assert_eq!(meta2.package.name, meta.package.name);
        assert_eq!(meta2.files.sha256, meta.files.sha256);
        assert_eq!(meta2.files.size, meta.files.size);
        assert_eq!(meta2.depends.runtime, meta.depends.runtime);
        assert_eq!(meta2.hooks.post_install, meta.hooks.post_install);
    }

    // ── 3. Index round-trip and get() lookup ─────────────────────────────────

    #[test]
    fn index_roundtrip_and_lookup() {
        let index_toml = r#"
[mksh-x86_64]
version = "R59c-r2"
license = "MirOS"
description = "MirBSD Korn Shell — /bin/sh on jonerix"
arch = "x86_64"
sha256 = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"
size = 98304
depends = ["musl"]

[libressl-aarch64]
version = "4.0.0"
license = "ISC"
description = "Free TLS/crypto stack from OpenBSD"
arch = "aarch64"
sha256 = "1122334455667788990011223344556677889900112233445566778899001122"
size = 2097152
depends = ["musl"]
build-depends = ["clang", "cmake", "samurai"]
"#;

        let index = Index::parse(index_toml).expect("parse INDEX");
        assert_eq!(index.entries.len(), 2);

        // get() by name + arch
        let mksh = index.get("mksh", "x86_64").expect("mksh-x86_64 not found");
        assert_eq!(mksh.version, "R59c-r2");
        assert_eq!(mksh.license, "MirOS");
        assert_eq!(mksh.arch, "x86_64");
        assert!(mksh.build_depends.is_empty());

        let libressl = index.get("libressl", "aarch64").expect("libressl-aarch64 not found");
        assert_eq!(libressl.version, "4.0.0");
        assert_eq!(libressl.build_depends, vec!["clang", "cmake", "samurai"]);

        // to_string() is deterministic (BTreeMap → alphabetical by key).
        let s1 = index.to_string().expect("serialise 1");
        let s2 = index.to_string().expect("serialise 2");
        assert_eq!(s1, s2, "serialisation is not deterministic");

        // Keys appear in alphabetical order: libressl-aarch64 before mksh-x86_64.
        let libressl_pos = s1.find("libressl-aarch64").expect("libressl-aarch64 missing");
        let mksh_pos = s1.find("mksh-x86_64").expect("mksh-x86_64 missing");
        assert!(libressl_pos < mksh_pos, "BTreeMap order not preserved in output");

        // Re-parse the serialised output and check it's equivalent.
        let index2 = Index::parse(&s1).expect("re-parse INDEX");
        assert_eq!(index2.entries.len(), 2);
        let mksh2 = index2.get("mksh", "x86_64").expect("mksh-x86_64 missing after re-parse");
        assert_eq!(mksh2.version, mksh.version);
    }

    // ── 4. Recipe::validate ───────────────────────────────────────────────────

    #[test]
    fn validate_rejects_empty_name() {
        let r = Recipe {
            package: PackageSection {
                name: Some(String::new()),
                version: Some("1.0.0".into()),
                license: Some("MIT".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(r.validate(), Err(RecipeError::Missing("package.name"))));
    }

    #[test]
    fn validate_rejects_missing_name() {
        let r = Recipe {
            package: PackageSection {
                name: None,
                version: Some("1.0.0".into()),
                license: Some("MIT".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(r.validate(), Err(RecipeError::Missing("package.name"))));
    }

    #[test]
    fn validate_rejects_empty_version() {
        let r = Recipe {
            package: PackageSection {
                name: Some("foo".into()),
                version: Some(String::new()),
                license: Some("MIT".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(r.validate(), Err(RecipeError::Missing("package.version"))));
    }

    #[test]
    fn validate_rejects_gpl3_license() {
        let r = Recipe {
            package: PackageSection {
                name: Some("bash".into()),
                version: Some("5.2".into()),
                license: Some("GPL-3.0".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(r.validate(), Err(RecipeError::BadLicense(_))));
    }

    #[test]
    fn validate_accepts_missing_description() {
        let r = Recipe {
            package: PackageSection {
                name: Some("toybox".into()),
                version: Some("0.8.11".into()),
                license: Some("0BSD".into()),
                description: None,
                ..Default::default()
            },
            ..Default::default()
        };
        // description is optional — validate must succeed.
        assert!(r.validate().is_ok());
    }

    #[test]
    fn validate_accepts_permissive_licenses() {
        let permissive = [
            "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC",
            "0BSD", "MirOS", "PSF-2.0", "MIT OR Apache-2.0",
        ];
        for lic in permissive {
            let r = Recipe {
                package: PackageSection {
                    name: Some("pkg".into()),
                    version: Some("1.0".into()),
                    license: Some(lic.into()),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(
                r.validate().is_ok(),
                "expected {lic:?} to be accepted but validate() returned error"
            );
        }
    }

    #[test]
    fn validate_accepts_absent_license() {
        // License is optional at the recipe stage; validation must not error.
        let r = Recipe {
            package: PackageSection {
                name: Some("pkg".into()),
                version: Some("1.0".into()),
                license: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(r.validate().is_ok());
    }

    // ── 5. Parse real core recipe.toml files ─────────────────────────────────
    //
    // Some existing recipes use `\;` inside TOML basic strings (from find -exec
    // usage) — a construct the lenient hand-rolled C parser accepted but strict
    // TOML does not.  We try up to 10 recipes, require at least 3 to parse
    // without error, and skip individual files that fail rather than panicking.
    // This faithfully tests that well-formed recipes are accepted while
    // acknowledging that some tree recipes carry a pre-existing TOML defect.

    #[test]
    fn parse_real_core_recipes() {
        use walkdir::WalkDir;

        // Walk every recipe.toml under packages/core and packages/extra and
        // assert they all parse via Recipe::from_file (which routes through
        // the sanitizer).  No `.take(N)` cap — with the sanitizer in place
        // the entire corpus must parse, otherwise we have a real regression
        // and the new jpkg-rs would refuse to build legacy recipes.
        let pkgs_dir = manifest_dir()
            .join("..")
            .canonicalize()
            .expect("canonicalize packages/ parent");
        if !pkgs_dir.exists() {
            eprintln!("skipping: pkgs dir {:?} not found", pkgs_dir);
            return;
        }

        let mut failures: Vec<(std::path::PathBuf, RecipeError)> = Vec::new();
        let mut parsed = 0usize;
        for entry in WalkDir::new(&pkgs_dir)
            .min_depth(2)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() == "recipe.toml")
        {
            // Skip jpkg-rs's own dummy recipes / placeholder dirs (none yet).
            let path = entry.path();
            // Skip our own crate's package dir (no recipe yet, but defensive).
            if path.to_string_lossy().contains("/jpkg-rs/") {
                continue;
            }
            match Recipe::from_file(path) {
                Ok(recipe) => {
                    assert!(
                        recipe.package.name.as_deref().map(|n| !n.is_empty()).unwrap_or(false),
                        "empty/missing name in {:?}",
                        path
                    );
                    parsed += 1;
                }
                Err(e) => failures.push((path.to_path_buf(), e)),
            }
        }

        assert!(parsed > 0, "no recipe.toml files found under {:?}", pkgs_dir);
        assert!(
            failures.is_empty(),
            "{} recipes failed to parse (out of {} total).  First few:\n{}",
            failures.len(),
            parsed + failures.len(),
            failures
                .iter()
                .take(5)
                .map(|(p, e)| format!("  {:?}: {}", p, e))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Sanitizer round-trip: feed it a basic-string with `\;`, `\(`, `\.`,
    /// `\$`, etc. (all valid TOML basic-string escapes after sanitisation),
    /// and assert the result parses + the post-sanitise text contains the
    /// expected `\\X` doubled forms.
    #[test]
    fn sanitize_legacy_escapes_passes_through_known_corpus_patterns() {
        use super::sanitize_legacy_escapes;
        let recipe = r#"
[package]
name = "x"
version = "1.0"
license = "MIT"
[build]
install = """find $DESTDIR -type f -exec strip {} \;
sed 's/\.so\.3//' /etc/foo.conf
grep -E '\(' /etc/bar"""
"#;
        let sanitised = sanitize_legacy_escapes(recipe);
        // Each non-conformant escape should now appear doubled.
        let s = sanitised.as_ref();
        assert!(s.contains(r"\\;"), "expected \\\\; — got {s}");
        assert!(s.contains(r"\\."), "expected \\\\. — got {s}");
        // The wrapped recipe should round-trip through serde.
        let r = Recipe::from_str(recipe).expect("sanitised recipe should parse");
        assert_eq!(r.package.name.as_deref(), Some("x"));
        assert!(r.build.install.unwrap().contains("strip {} \\;"));
    }

    /// A clean (no backslash) recipe must NOT be modified by the sanitizer
    /// — `Cow::Borrowed` path is the hot-path for compliant TOML.
    #[test]
    fn sanitize_legacy_escapes_borrows_when_no_changes_needed() {
        use super::sanitize_legacy_escapes;
        let clean = r#"
[package]
name = "y"
version = "1.0"
license = "MIT"
"#;
        let result = sanitize_legacy_escapes(clean);
        match result {
            std::borrow::Cow::Borrowed(s) => assert_eq!(s, clean),
            std::borrow::Cow::Owned(_) => panic!("sanitiser should borrow when no changes needed"),
        }
    }

    /// Literal strings (`'…'` / `'''…'''`) must be left untouched even when
    /// they contain backslashes — escape processing doesn't run inside them.
    #[test]
    fn sanitize_legacy_escapes_leaves_literal_strings_alone() {
        use super::sanitize_legacy_escapes;
        let input = "key = '''path\\with\\;backslashes'''\n";
        let out = sanitize_legacy_escapes(input);
        assert!(matches!(out, std::borrow::Cow::Borrowed(_)),
                "literal strings shouldn't trigger doubling");
    }

    // ── Extra: from_recipe constructor ───────────────────────────────────────

    #[test]
    fn from_recipe_builds_metadata() {
        let r = Recipe {
            package: PackageSection {
                name: Some("zstd".into()),
                version: Some("1.5.6".into()),
                license: Some("BSD-3-Clause".into()),
                description: Some("Fast real-time compression".into()),
                arch: Some("x86_64".into()),
                ..Default::default()
            },
            depends: DependsSection {
                runtime: vec!["musl".into()],
                build: vec!["cmake".into()],
            },
            hooks: HooksSection {
                post_install: Some("ldconfig /lib 2>/dev/null || true".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string();
        let meta = Metadata::from_recipe(&r, sha.clone(), 512_000);

        assert_eq!(meta.package.name, r.package.name);
        assert_eq!(meta.package.version, r.package.version);
        assert_eq!(meta.depends.runtime, r.depends.runtime);
        assert_eq!(meta.hooks.post_install, r.hooks.post_install);
        assert_eq!(meta.files.sha256.as_deref(), Some(sha.as_str()));
        assert_eq!(meta.files.size, Some(512_000));

        // source / build sections are gone from Metadata — confirm they're absent
        // from the serialised output.
        let toml_out = meta.to_string().expect("serialise");
        assert!(!toml_out.contains("[source]"), "source section must not appear in metadata");
        assert!(!toml_out.contains("[build]"), "build section must not appear in metadata");
    }
}
