// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

use std::fmt;

// ─── Sha256Hash ──────────────────────────────────────────────────────────────

/// A validated 64-character lower-case hex SHA-256 digest.
///
/// Construct via [`Sha256Hash::try_from`] (validates length + char set) or
/// [`Sha256Hash::from_bytes`] (converts a `[u8; 32]` raw hash).  Deref to
/// `&str` for use wherever a plain string is expected.
///
/// The sentinel all-zeros hash used in symlink manifest entries is available
/// as [`Sha256Hash::SYMLINK_SENTINEL`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Sha256Hash(String);

impl Sha256Hash {
    /// The all-zeros sentinel used in the `files` manifest for symlinks.
    pub const SYMLINK_SENTINEL_STR: &'static str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    /// Return an all-zeros sentinel (symlink marker).
    pub fn symlink_sentinel() -> Self {
        Self(Self::SYMLINK_SENTINEL_STR.to_owned())
    }

    /// Return `true` if this hash is the all-zeros symlink sentinel.
    pub fn is_symlink_sentinel(&self) -> bool {
        self.0 == Self::SYMLINK_SENTINEL_STR
    }

    /// Construct from 32 raw bytes (output of `sha2::Sha256::finalize()`).
    pub fn from_bytes(raw: &[u8; 32]) -> Self {
        Self(hex::encode(raw))
    }

    /// Construct from a hex string, returning `None` if the input is not
    /// exactly 64 lower-case hex characters.
    pub fn try_from_str(s: &str) -> Option<Self> {
        if s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Some(Self(s.to_owned()))
        } else {
            None
        }
    }

    /// Borrow the inner hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the inner `String`.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Hash({:?})", self.0)
    }
}

impl std::ops::Deref for Sha256Hash {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Sha256Hash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<Sha256Hash> for String {
    fn from(h: Sha256Hash) -> String {
        h.0
    }
}

// ─── InstallMode ─────────────────────────────────────────────────────────────

/// Controls whether packages that are already installed should be reinstalled.
///
/// Replaces `force: bool` in [`crate::deps::resolve_install`] and
/// [`crate::cmd::install::install_packages`].
///
/// ```
/// use jpkg::types::InstallMode;
///
/// fn install(mode: InstallMode) {
///     if mode.is_force() { /* ... */ }
/// }
///
/// install(InstallMode::Normal);
/// install(InstallMode::Force);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum InstallMode {
    /// Skip packages that are already installed at the same version (default).
    #[default]
    Normal,
    /// Reinstall even if the package is already present at the same version.
    Force,
}

impl InstallMode {
    /// Return `true` if this is [`InstallMode::Force`].
    #[inline]
    pub fn is_force(self) -> bool {
        self == InstallMode::Force
    }
}

// ─── OrphanMode ──────────────────────────────────────────────────────────────

/// Controls whether orphaned dependencies are included in a removal plan.
///
/// Replaces `orphans: bool` in [`crate::deps::resolve_remove`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OrphanMode {
    /// Remove only the explicitly named packages (default).
    #[default]
    KeepOrphans,
    /// Also remove runtime dependencies that become unused after the removal.
    PruneOrphans,
}

impl OrphanMode {
    /// Return `true` if this is [`OrphanMode::PruneOrphans`].
    #[inline]
    pub fn is_prune(self) -> bool {
        self == OrphanMode::PruneOrphans
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sha256Hash ────────────────────────────────────────────────────────────

    #[test]
    fn sha256_hash_from_valid_hex() {
        let s = "a".repeat(64);
        let h = Sha256Hash::try_from_str(&s).expect("valid 64-char hex");
        assert_eq!(h.as_str(), s.as_str());
        assert_eq!(h.to_string(), s);
    }

    #[test]
    fn sha256_hash_rejects_wrong_length() {
        assert!(Sha256Hash::try_from_str("abc").is_none());
        assert!(Sha256Hash::try_from_str(&"a".repeat(63)).is_none());
        assert!(Sha256Hash::try_from_str(&"a".repeat(65)).is_none());
    }

    #[test]
    fn sha256_hash_rejects_uppercase() {
        // We require lower-case hex (SHA-256 convention used throughout jpkg).
        let upper = "A".repeat(64);
        assert!(Sha256Hash::try_from_str(&upper).is_none());
    }

    #[test]
    fn sha256_hash_from_bytes_roundtrip() {
        let raw = [0xdeu8; 32];
        let h = Sha256Hash::from_bytes(&raw);
        assert_eq!(h.as_str().len(), 64);
        assert!(h.as_str().starts_with("de"));
    }

    #[test]
    fn sha256_hash_symlink_sentinel() {
        let s = Sha256Hash::symlink_sentinel();
        assert!(s.is_symlink_sentinel());
        assert_eq!(s.as_str().len(), 64);
        assert!(s.as_str().chars().all(|c| c == '0'));
    }

    #[test]
    fn sha256_hash_deref_as_str() {
        let s = "b".repeat(64);
        let h = Sha256Hash::try_from_str(&s).unwrap();
        // Deref allows &h to be used as &str.
        let slice: &str = &h;
        assert_eq!(slice, s.as_str());
    }

    // ── InstallMode ───────────────────────────────────────────────────────────

    #[test]
    fn install_mode_is_force() {
        assert!(!InstallMode::Normal.is_force());
        assert!(InstallMode::Force.is_force());
        assert!(!InstallMode::default().is_force());
    }

    // ── OrphanMode ────────────────────────────────────────────────────────────

    #[test]
    fn orphan_mode_is_prune() {
        assert!(!OrphanMode::KeepOrphans.is_prune());
        assert!(OrphanMode::PruneOrphans.is_prune());
        assert!(!OrphanMode::default().is_prune());
    }
}
