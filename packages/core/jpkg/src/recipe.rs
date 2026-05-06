// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

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
/// Merge duplicate top-level table headers in a TOML document so the strict
/// `toml` crate accepts metadata files written by C jpkg 1.x — which used a
/// hand-rolled parser that silently merged duplicate `[package]` (and other)
/// blocks across separate writes (audit § 6: "Hook body escaping" notes the
/// same parser laxity).  Newer .jpkg metadata files commonly carry two
/// `[package]` blocks (one for `name`/`version`/`license`/etc., a second for
/// `replaces`/`conflicts`), and the strict parser errors on the second header
/// with `TOML parse error at line N, column 1`.
///
/// Algorithm: walk the document line-by-line, group body lines under their
/// most-recent table header, then emit each unique header exactly once with
/// all collected body lines concatenated.  Array-of-tables (`[[X]]`) and
/// dotted sub-table headers (`[a.b]`) keep their identity (the dedup key is
/// the literal content between the brackets).  Lines before the first header
/// (comments, blank lines) stay at the top in their original order.
pub(crate) fn merge_duplicate_sections(input: &str) -> Cow<'_, str> {
    // Quick early-out: if there's at most one `[`-prefixed line, nothing to merge.
    let n_headers = input
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with('[') && !t.starts_with("[[")
        })
        .count();
    if n_headers <= 1 {
        return Cow::Borrowed(input);
    }

    // Two-pass: collect bodies grouped by header key, preserving header order
    // of first occurrence.  Array-of-tables `[[X]]` headers are NOT merged
    // (they are intentionally repeatable per TOML spec) — they're emitted
    // verbatim in document order along with their bodies.
    let mut order: Vec<String> = Vec::new();
    let mut bodies: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut prefix: Vec<&str> = Vec::new();
    let mut current: Option<String> = None;
    // For [[X]]: we treat each occurrence as a unique slot — generate a
    // synthetic key so it's preserved in order without merging.
    let mut aot_counter: usize = 0;
    let mut needs_merge = false;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Track whether we're currently inside a multi-line basic string (`"""`)
    // or multi-line literal string (`'''`).  Header-like lines inside these
    // are SHELL content, not TOML headers.  We update the state on the line
    // BEFORE classifying — so a line that opens a multi-line string is
    // treated as a header iff it starts with `[` outside a string AT THE
    // START of the line (Worker B's sanitizer uses byte-state for the same
    // reason; here a simpler line-level toggle suffices because TOML's
    // multi-line delimiters are themselves on dedicated lines in the
    // metadata files we encounter — and the recipes that use `"""<body>"""`
    // open the delimiter on a `key = """` line that doesn't start with `[`).
    let mut in_basic_multi = false;
    let mut in_literal_multi = false;
    for line in input.lines() {
        // Toggle multi-line string state from THIS line first if it has an
        // opening delimiter, so subsequent in-body lines see the new state.
        // We don't bother with bytewise counting — odd-count toggles handle
        // both open and close.
        if !in_literal_multi {
            let n = line.matches("\"\"\"").count();
            if n % 2 == 1 {
                in_basic_multi = !in_basic_multi;
            }
        }
        if !in_basic_multi {
            let n = line.matches("'''").count();
            if n % 2 == 1 {
                in_literal_multi = !in_literal_multi;
            }
        }
        // Inside a multi-line string => treat as body, never as a header.
        let in_string = in_basic_multi || in_literal_multi;

        let trimmed = line.trim_start();
        if !in_string && trimmed.starts_with("[[") {
            if let Some(end) = trimmed[2..].find("]]") {
                let inner = &trimmed[2..2 + end];
                let key = format!("__aot__{}__{}", aot_counter, inner);
                aot_counter += 1;
                order.push(key.clone());
                bodies.entry(key.clone()).or_default().push(line);
                current = Some(key);
                continue;
            }
        } else if !in_string {
            if let Some(rest) = trimmed.strip_prefix('[') {
                if let Some(end) = rest.find(']') {
                    let inner = rest[..end].trim().to_string();
                    if !seen.insert(inner.clone()) {
                        needs_merge = true;
                    } else {
                        order.push(inner.clone());
                    }
                    bodies.entry(inner.clone()).or_default().push(line);
                    current = Some(inner);
                    continue;
                }
            }
        }
        // Body line (or comment/blank, or in-string content).
        match &current {
            Some(key) => {
                bodies.get_mut(key).unwrap().push(line);
            }
            None => prefix.push(line),
        }
    }

    if !needs_merge {
        return Cow::Borrowed(input);
    }

    // Reassemble: prefix, then each unique section (in first-seen order)
    // with its header line + all body lines from every occurrence.
    let mut out = String::with_capacity(input.len());
    for line in prefix {
        out.push_str(line);
        out.push('\n');
    }
    for key in &order {
        if let Some(lines) = bodies.get(key) {
            // Skip empty groups (defensive)
            if lines.is_empty() {
                continue;
            }
            // The first body line under each unique header is the header itself.
            // Subsequent occurrences only contribute their NON-header body lines —
            // skip the duplicate header lines on merge.
            let mut emitted_header = false;
            for line in lines {
                let t = line.trim_start();
                let is_header = t.starts_with('[');
                if is_header {
                    if !emitted_header {
                        out.push_str(line);
                        out.push('\n');
                        emitted_header = true;
                    }
                    // duplicate header — skip
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
    }
    Cow::Owned(out)
}

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
                // Triple-quoted basic strings ("""...""") in jonerix recipes
                // are nearly always shell or python heredocs that the C jpkg's
                // lenient parser treated as literal (it ignored backslash
                // escape processing).  Recipe authors wrote `python3 -c '"\n"'`
                // expecting the literal two-char `\n` sequence to land in the
                // python source; the strict toml crate processes it as a real
                // newline and breaks the heredoc with `unterminated string
                // literal`.  Reproduced 2026-04-27 bootstrap CI: sudo, runc,
                // containerd, nerdctl, tzdata, strace all failed at this point.
                //
                // Fix: inside `"""..."""` ONLY, double EVERY backslash so the
                // toml crate yields the literal two-char sequence the recipe
                // author actually wrote.  Source `\X` → sanitised `\\X` →
                // toml parses `\\X` → `\X` literal (2 chars).  Source `\\X` →
                // sanitised `\\\\X` → toml parses `\\\\X` → `\\X` literal (3
                // chars).  This was reproduced 2026-04-27 by strace (which
                // uses `\\<newline>` for sed multi-line `a\` line continuation
                // through a shell `"..."` quoted argument); without doubling
                // every backslash including the `\\` pairs, the parsed string
                // ended up as `\<newline>` and mksh's `"..."` line-continuation
                // collapsed all 4 lines into one, producing a malformed
                // dirent64.c.
                // Single-line `"..."` strings keep spec-conformant escape
                // processing (handled by the SanitizeState::BasicSingle arm).
                if b == b'\\' && i + 1 < n {
                    let nxt = bytes[i + 1];
                    if nxt == b'\\' {
                        // Doubled backslash in source: emit 4 so toml parses
                        // back to the literal `\\` (2 chars) the C jpkg
                        // produced.  Don't fold this into the `\X` branch —
                        // we must consume both source bytes here so the next
                        // iteration starts after the `\\` pair, otherwise the
                        // second `\` would be re-processed against whatever
                        // follows it (e.g. `\\n` would mis-read as `\\` then
                        // `\n`).
                        out.extend_from_slice(b"\\\\\\\\");
                        changed = true;
                        i += 2;
                    } else {
                        // `\X` where X != `\`: double the backslash so the
                        // toml crate produces the literal 2-char sequence.
                        out.extend_from_slice(b"\\\\");
                        out.push(nxt);
                        changed = true;
                        i += 2;
                    }
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

// ─── License gate ────────────────────────────────────────────────────────────

/// Returns `true` if `license` is (or resolves to) a permissive license.
///
/// Mirrors the C `license_is_permissive` logic, extended to handle:
/// - Exact case-insensitive match against the table.
/// - SPDX OR: permissive if **any** component is permissive.
/// - SPDX AND: permissive only if **all** components are permissive.
/// - Parenthesised sub-expressions, e.g. `"(MIT OR GPL-2.0) AND Apache-2.0"`.
///
/// Delegates to [`crate::util::license_is_permissive`] so the two copies
/// stay in sync.
pub fn license_is_permissive(license: &str) -> bool {
    crate::util::license_is_permissive(license)
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
        let merged = merge_duplicate_sections(s);
        let sanitized = sanitize_legacy_escapes(merged.as_ref());
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

// ─── Signature ───────────────────────────────────────────────────────────────

/// `[signature]` section — per-package Ed25519 signature embedded in metadata.
///
/// The `algorithm` field must be `"ed25519"` in jpkg 2.x.
/// The `sig` field holds the base64-encoded raw 64-byte signature over the
/// canonical bytes produced by `canon::canonical_bytes`.
///
/// This section is optional (`skip_serializing_if = "Option::is_none"`) so
/// that older recipes without a `[signature]` block round-trip cleanly, and so
/// that the canonical-bytes serialisation can strip it before signing.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Signature {
    /// Signing algorithm identifier.  Must be `"ed25519"` in jpkg 2.x.
    pub algorithm: String,
    /// Human-readable key identifier, e.g. `"jonerix-2026"`.
    pub key_id: String,
    /// Base64-encoded raw 64-byte Ed25519 signature.
    pub sig: String,
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
    /// Per-package Ed25519 signature.  Absent on unsigned packages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl Metadata {
    /// Parse metadata from a TOML string (embedded in a `.jpkg` or read from disk).
    pub fn from_str(s: &str) -> Result<Self, RecipeError> {
        let merged = merge_duplicate_sections(s);
        let sanitized = sanitize_legacy_escapes(merged.as_ref());
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
            // Signature is not set by the build step; Worker B will handle signing.
            signature: None,
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
    /// The INDEX is a single-level TOML document where each top-level key is
    /// either:
    ///   * `pkgname-arch` — a package entry with full IndexEntry fields, or
    ///   * a metadata section like `[meta]` (gen-index.sh emits one carrying
    ///     a timestamp).  Any top-level key whose value is NOT shaped like an
    ///     IndexEntry is silently skipped — the C jpkg's parser ignores
    ///     non-conformant sections the same way (repo.c:171-182 strips the
    ///     `-arch` suffix and bails out cleanly when the suffix isn't there).
    ///
    /// Implementation: deserialise into `BTreeMap<String, toml::Value>` first
    /// (always succeeds for any valid TOML), then per-key try to lift each
    /// value into an `IndexEntry`.  Failures are dropped, not propagated.
    pub fn parse(text: &str) -> Result<Self, RecipeError> {
        let merged = merge_duplicate_sections(text);
        let sanitized = sanitize_legacy_escapes(merged.as_ref());
        let raw: BTreeMap<String, toml::Value> = toml::from_str(&sanitized)?;
        let mut entries: BTreeMap<String, IndexEntry> = BTreeMap::new();
        for (key, val) in raw {
            // Package entries carry an `-arch` suffix; metadata tables (e.g.
            // `[meta]`) do not.  Skip anything that doesn't match the shape.
            if !key.contains('-') {
                continue;
            }
            // Try to deserialise as IndexEntry.  Skip on shape mismatch.
            match val.try_into::<IndexEntry>() {
                Ok(entry) => {
                    entries.insert(key, entry);
                }
                Err(_) => continue,
            }
        }
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
        // jpkg's own recipe sits next to this crate (manifest_dir/recipe.toml
        // under the new packages/jpkg/ layout post-2.0 promotion).  Try the
        // sibling location first; fall back to the historical
        // packages/jpkg/recipe.toml path so the test still finds it if the
        // crate is rearranged again later.
        let recipe_path = {
            let here = manifest_dir().join("recipe.toml");
            if here.exists() {
                here
            } else {
                manifest_dir().join("../jpkg/recipe.toml")
            }
        };
        if !recipe_path.exists() {
            eprintln!("skipping: {:?} not found", recipe_path);
            return;
        }

        let original = std::fs::read_to_string(&recipe_path).expect("read recipe.toml");
        let recipe = Recipe::from_str(&original).expect("parse recipe");

        // Stable invariants — the recipe IS for jpkg, declares musl as a
        // runtime dep.  The version, replaces shape, and license are
        // intentionally not asserted here so the test survives version
        // bumps and the MIT→0BSD relicense (2026-04-30); the license
        // value is only checked to be non-empty.
        assert_eq!(recipe.package.name.as_deref(), Some("jpkg"));
        assert!(recipe.package.license.as_deref().map_or(false, |l| !l.is_empty()),
            "license must be set");
        // mksh joined musl as a runtime dep in 2.0.1-r2 because
        // jpkg-conform's #!/bin/mksh shebang requires it; the test
        // accepts either ["musl"] (older recipes) or any superset.
        assert!(recipe.depends.runtime.iter().any(|d| d == "musl"),
            "runtime deps must include musl");
        assert!(recipe.package.version.as_deref().map_or(false, |v| !v.is_empty()));

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
        // Walk up from manifest_dir() looking for a parent named `packages`.
        // The crate has lived at packages/jpkg-rs/, packages/jpkg/, and now
        // packages/core/jpkg/ — auto-discovery makes the test path-stable.
        let mut pkgs_dir: Option<std::path::PathBuf> = None;
        for ancestor in manifest_dir().ancestors() {
            if ancestor.file_name().map(|n| n == "packages").unwrap_or(false) {
                pkgs_dir = Some(ancestor.to_path_buf());
                break;
            }
        }
        let pkgs_dir = match pkgs_dir {
            Some(p) => p,
            None => {
                eprintln!(
                    "skipping: no `packages` ancestor of {:?}",
                    manifest_dir()
                );
                return;
            }
        };
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

    /// Regression for the publish-images iteration that broke on the live
    /// rolling-release INDEX: `gen-index.sh` emits a `[meta]` table with
    /// timestamp at the top of the document.  The original parse impl
    /// deserialised straight into `BTreeMap<String, IndexEntry>`, which
    /// rejected `[meta]` because it had no `version`/`license`/`sha256`
    /// fields.  The new impl skips non-package sections silently — match
    /// the C jpkg's repo.c behaviour of strip-`-arch`-and-bail.
    #[test]
    fn index_parse_skips_meta_section() {
        let live_shape = r#"[meta]
timestamp = "2026-04-26T23:54:05Z"

[anvil-x86_64]
version = "0.2.1-r1"
license = "MIT"
description = "ext2/3/4 userland in pure Rust"
arch = "x86_64"
sha256 = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
size = 12345
depends = ["musl"]
build-depends = ["rust"]

[brash-aarch64]
version = "1.0.0"
license = "MIT"
description = "Bash-compatible Rust shell"
arch = "aarch64"
sha256 = "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe"
size = 67890
depends = ["musl"]
build-depends = ["rust"]
"#;
        let idx = Index::parse(live_shape).expect("parse INDEX with [meta]");
        assert_eq!(idx.entries.len(), 2, "[meta] must be skipped");
        assert!(idx.get("anvil", "x86_64").is_some());
        assert!(idx.get("brash", "aarch64").is_some());
        // [meta] must NOT have leaked through as a package
        assert!(!idx.entries.contains_key("meta"));
    }

    // ── Signature roundtrip tests ────────────────────────────────────────────

    /// Parse TOML containing `[signature]`, serialise it back, re-parse:
    /// fields must survive the round-trip intact.
    #[test]
    fn metadata_signature_roundtrip_with_sig() {
        let toml_str = r#"
[package]
name = "testsig"
version = "1.0.0"
license = "MIT"
arch = "x86_64"

[files]
sha256 = "aaaa"
size = 1

[signature]
algorithm = "ed25519"
key_id = "jonerix-2026"
sig = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
"#;
        let meta = Metadata::from_str(toml_str).expect("parse metadata with signature");
        let sig = meta.signature.as_ref().expect("signature should be present");
        assert_eq!(sig.algorithm, "ed25519");
        assert_eq!(sig.key_id, "jonerix-2026");
        assert_eq!(sig.sig, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");

        // Re-serialise and re-parse — fields must match.
        let serialised = meta.to_string().expect("serialise");
        assert!(serialised.contains("[signature]"), "serialised form must contain [signature]");
        let meta2 = Metadata::from_str(&serialised).expect("re-parse");
        assert_eq!(meta2.signature, meta.signature);
    }

    /// Parse TOML WITHOUT `[signature]`: signature is None; serialised form
    /// must NOT emit a `[signature]` section at all.
    #[test]
    fn metadata_signature_roundtrip_without_sig() {
        let toml_str = r#"
[package]
name = "nosig"
version = "1.0.0"
license = "MIT"
arch = "x86_64"

[files]
sha256 = "bbbb"
size = 2
"#;
        let meta = Metadata::from_str(toml_str).expect("parse metadata without signature");
        assert!(meta.signature.is_none(), "signature should be None when absent from TOML");

        let serialised = meta.to_string().expect("serialise");
        assert!(
            !serialised.contains("[signature]"),
            "serialised form must omit [signature] when None; got:\n{serialised}"
        );

        let meta2 = Metadata::from_str(&serialised).expect("re-parse");
        assert!(meta2.signature.is_none(), "re-parsed signature should still be None");
    }

    /// And section keys without an `-arch` suffix (e.g. a stray legacy
    /// `[debug]` block) must also be skipped, not error out the parse.
    #[test]
    fn index_parse_skips_keys_without_dash_suffix() {
        let with_debug = r#"[debug]
trace = true

[uutils-x86_64]
version = "0.1.0"
license = "MIT"
description = "GNU coreutils in Rust"
arch = "x86_64"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
size = 1
depends = ["musl"]
build-depends = ["rust"]
"#;
        let idx = Index::parse(with_debug).expect("parse INDEX with stray section");
        assert_eq!(idx.entries.len(), 1);
        assert!(idx.get("uutils", "x86_64").is_some());
    }
}
