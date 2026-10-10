// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Config files (2.2.11).
//!
//! # The rule
//!
//! A **config file** is a regular file a package ships under `etc/`, except
//! under `etc/init.d/`, `etc/cron.d/` and `etc/ssl/certs/`, and except
//! `etc/ssl/cert.pem`.  OpenRC treats every file in init.d as a service and
//! snooze-crond sources every file in cron.d, so those files are code: a
//! side-by-side `<file>.jpkg-new` there would run too, and package fixes to
//! them must land.  etc/ssl/certs/ and cert.pem (LibreSSL's default CA
//! file, a link into it once ca-certificates is installed) are the trust
//! store: Go's crypto/x509 (and Node with --use-system-ca) loads every file
//! in certs/, dotfiles included, so an offer there would be trusted; and CA
//! removals must keep reaching every client, also on hosts whose image left
//! its own bundle or link there, so they stay package data, replaced on
//! every update as in 2.2.10.  The other reader
//! directories jonerix ships into filter by name (profile.d `*.sh`,
//! sysctl.d `*.conf`, sudoers.d skips names with a dot, conf.d is read by
//! service name, fonts conf.d `[0-9]*.conf`, local.d `*.start`/`*.stop`).
//! The exception is etc/skel/: `useradd -m` copies everything in it, so
//! after an edit and a package update a new user's home also gets the
//! `<file>.jpkg-new` until the admin deletes it.  That is kept deliberately:
//! losing an admin's skel edits is worse than one stray file.  A package
//! must not ship defaults into any other directory whose reader loads every
//! entry.
//!
//! The class is a pure function of the manifest path, worked out on the
//! installing host.  Nothing about it is stored in the archive metadata (so
//! canonical bytes and signatures are unchanged -- see the golden tests in
//! `canon.rs`) or in the installed `files` manifest (so a 2.2.10 jpkg reads
//! the same database after a downgrade).  The manifest keeps recording what
//! the package SHIPPED, also for a file that was kept; that is the merge
//! base for the next upgrade.
//!
//! # The invariant
//!
//! jpkg overwrites or deletes a non-directory object at a config path only
//! when it is **pristine**: missing, a regular file whose sha256 jpkg
//! recorded for that path (this package's previous manifest or another
//! installed package's), or a symlink whose target jpkg recorded.  Anything
//! else -- a file with other content, an unrecorded symlink, a directory
//! jpkg did not record, a FIFO or a device -- is the admin's: it is never
//! opened, overwritten or deleted.  The one thing jpkg may do to it is
//! rename it to `<path>.jpkg-save`, when a package changes the KIND of
//! object at that path (a file becoming a symlink or a directory) or puts
//! a directory where it cannot be followed.  A probe error before anything
//! is written aborts the install; after writing has started it counts as
//! changed: what cannot be checked is kept.
//!
//! Two limits, both as in 2.2.10: a symlinked parent directory is followed
//! (an admin who moves /etc/foo elsewhere and links it gets the package's
//! files written, and pristine ones removed, through the link; under
//! `--root` an absolute link resolves on the host), and a directory a
//! package ships is merged into an existing directory and written through
//! an admin's symlink that leads to a directory -- except, under `--root`,
//! an absolute one, which would lead out of the root and is saved instead.
//! A package's own previous link where it now ships a directory is removed,
//! and the files below it are installed as new -- after checking that the
//! directory the link led to holds only that package's own pristine files
//! (else the upgrade is refused before any write, naming the rest; links
//! are resolved inside the root and nested links below are walked too).
//! Another package's link that leads to a directory is followed, never
//! removed; under `--root`, where an absolute link (or one climbing out of
//! the root) cannot be followed, the install is refused instead.  One jpkg
//! recorded that leads nowhere or to a file is removed.  Directories are only
//! ever removed when empty, except the old directory of a package that
//! turns it into a symlink, which upgrade-clean removes only after checking
//! that everything in it is the package's and pristine.
//!
//! # What that means
//!
//! * install, upgrade, reinstall: a pristine file is replaced.  A changed
//!   one is kept; if the package's copy differs from every recorded one, it
//!   is written next to it as `<file>.jpkg-new` and a warning names it.
//! * `<file>.jpkg-new` is written only over nothing or over package content
//!   (an earlier offer).  If the admin is working in it, it is left alone
//!   and the new packaged version is not written; once the admin clears
//!   the slot, the next version that changes the file offers its copy.
//! * an upgrade that stops shipping the file, or `jpkg remove`: a pristine
//!   file is deleted; a changed one stays where it is, now owned by no
//!   package, with a warning.  (Where the upgrade ships a symlink or a
//!   directory there instead, see below; where a co-owner keeps the path,
//!   it stays.)  An untouched offer goes in every one of these cases, and so
//!   does an interrupted install's scratch file.
//! * a symlink the package ships where it (or another package) shipped a
//!   symlink before: an admin's change there -- another target, or a file
//!   or directory in its place -- is kept, like a changed config file, and
//!   the package's link is not written.
//! * a package changing the kind of object at a config path (a config file
//!   becoming a symlink or a directory, its own or another package's): a
//!   changed object there is moved to `<file>.jpkg-save` first, never over
//!   an existing one; a pristine file is replaced (see [`displace`]).
//! * a package turning a directory it recorded into a config file: the file
//!   lands once upgrade-clean has emptied the directory; if anything is
//!   left in it, the directory is kept like any changed object.
//! * files and links at config paths are written last, after everything
//!   else, each under `<file>.jpkg-tmp` and renamed into place; every
//!   decision is taken again just before that, which keeps an edit made
//!   earlier in the upgrade.  The writes themselves are not guarded: an
//!   edit made while they run can be lost.
//! * a failure before that last step leaves the copies the database still
//!   records, and none is ever half-written.  A failure during it, or
//!   before the database is updated, can leave some of the new version's
//!   copies, which jpkg did not record: the next different version keeps
//!   them as changed and offers its own copy (reinstalling the same
//!   version repairs them).
//! * verify: a changed config file (or a changed symlink at a config path)
//!   is reported, not counted as a failure.  A missing one is a failure.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::db::FileEntry;
use crate::util::sha256_file;

/// Paths under `etc/` that are not config (see the module doc): a trailing
/// `/` names a directory whose every file is executed, scheduled or
/// trusted; anything else is one exact path.
const NOT_CONFIG: &[&str] = &["etc/init.d/", "etc/cron.d/", "etc/ssl/certs/", "etc/ssl/cert.pem"];

/// Suffix of the package's copy when a changed config file is kept.
pub const NEW_SUFFIX: &str = ".jpkg-new";
/// Suffix of the temporary name a config file (or config-path link) is
/// written under before it is renamed into place.  jpkg's own scratch.
pub const TMP_SUFFIX: &str = ".jpkg-tmp";
/// Suffix of what is moved aside where a package places a symlink or a
/// directory at a config path (see [`displace`]).
pub const SAVE_SUFFIX: &str = ".jpkg-save";

/// True for a manifest path (relative, no leading slash) that is a config
/// path.  Callers must also check the entry is a regular file.
pub fn is_config_path(path: &str) -> bool {
    path.starts_with("etc/")
        && !NOT_CONFIG
            .iter()
            .any(|d| if d.ends_with('/') { path.starts_with(d) } else { path == *d })
}

/// True when `e` is a config file: a regular file at a config path.
pub fn is_config(e: &FileEntry) -> bool {
    !e.is_dir && e.symlink_target.is_none() && is_config_path(&e.path)
}

/// What is at a config path on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnDisk {
    Missing,
    /// A regular file with this sha256.
    File(String),
    /// A symlink with this target (never followed).
    Link(String),
    /// A directory (never descended into here).
    Dir,
    /// A FIFO, socket or device (never opened).
    Other,
}

/// Look at `rootfs/path` without following a symlink and without opening
/// anything but a regular file.
pub fn on_disk(rootfs: &Path, path: &str) -> io::Result<OnDisk> {
    let abs = rootfs.join(path);
    let m = match abs.symlink_metadata() {
        // ENOTDIR: a parent is a file, so this path cannot exist (yet) --
        // e.g. a package turning etc/x.conf into a directory.
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {
            return Ok(OnDisk::Missing)
        }
        Err(e) => return Err(e),
        Ok(m) => m,
    };
    let ft = m.file_type();
    if ft.is_file() {
        Ok(OnDisk::File(sha256_file(&abs)?))
    } else if ft.is_symlink() {
        Ok(OnDisk::Link(fs::read_link(&abs)?.to_string_lossy().into_owned()))
    } else if ft.is_dir() {
        Ok(OnDisk::Dir)
    } else {
        Ok(OnDisk::Other)
    }
}

/// Everything jpkg recorded for one path before the current operation:
/// the regular-file hashes and the symlink targets.
#[derive(Debug, Default, Clone)]
pub struct Recorded<'a> {
    pub shas: Vec<&'a str>,
    pub links: Vec<&'a str>,
    /// Some manifest records the path as a directory.
    pub dir: bool,
}

impl<'a> Recorded<'a> {
    /// Record one manifest entry for the path.
    pub fn add(&mut self, sha256: &'a str, symlink_target: Option<&'a str>, is_dir: bool) {
        if is_dir {
            self.dir = true;
            return;
        }
        match symlink_target {
            Some(t) => self.links.push(t),
            None if !sha256.is_empty() => self.shas.push(sha256),
            None => {}
        }
    }

    pub fn entry(e: &'a FileEntry) -> Self {
        let mut r = Recorded::default();
        r.add(&e.sha256, e.symlink_target.as_deref(), e.is_dir);
        r
    }
}

/// True when jpkg may write, rename or delete over `disk`.
pub fn is_pristine(disk: &OnDisk, rec: &Recorded<'_>) -> bool {
    match disk {
        OnDisk::Missing => true,
        OnDisk::File(h) => rec.shas.contains(&h.as_str()),
        OnDisk::Link(t) => rec.links.contains(&t.as_str()),
        // A directory is decided by the caller once its owned contents are
        // gone (see `is_empty_dir`); here it is never pristine.
        OnDisk::Dir | OnDisk::Other => false,
    }
}

/// What install does with one config file the new package ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Write the package's copy (the normal install path).
    Install,
    /// Leave the admin's object alone; the package's copy is one jpkg
    /// already recorded, so there is nothing new to offer.
    Keep,
    /// Leave the admin's object alone and write the package's copy next to
    /// it as `<file>.jpkg-new`.
    KeepAndNew,
}

/// Decide for one config file.  `new_sha` is the package's copy; `rec` is
/// everything jpkg recorded for the path before this install.
pub fn action(disk: &OnDisk, new_sha: &str, rec: &Recorded<'_>) -> Action {
    if matches!(disk, OnDisk::File(d) if d == new_sha) || is_pristine(disk, rec) {
        Action::Install
    } else if rec.shas.contains(&new_sha) {
        Action::Keep
    } else {
        Action::KeepAndNew
    }
}

/// What happens to the object at a config path where the new package places
/// a symlink or a directory (`n`, a non-config entry at a config path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Displace {
    /// Nothing to do, or nothing different from 2.2.10: install_files
    /// replaces jpkg's own object, merges into a directory, follows a
    /// symlink that leads to a directory where a directory goes, and
    /// upgrade-clean handles a directory jpkg recorded becoming a symlink.
    Leave,
    /// The admin changed what the package ships here without the package
    /// changing its kind (a retargeted link, or a file or directory in
    /// place of a link), or put something where nothing was recorded: keep
    /// it, and do not write the package's link.
    Keep,
    /// The package's own previous link, jpkg's file, or another package's
    /// recorded link that does not lead to a directory (dead, or to a
    /// file), where a directory goes: remove it first
    /// (config files below it are then decided as if nothing were there).
    Remove,
    /// Anything else not pristine: move it to `<path>.jpkg-save` first.
    Save,
}

/// Decide [`Displace`] for the object `disk` at `n.path`.  `link_to_dir`:
/// `disk` is a symlink that leads (followed, as install_files would follow
/// it) to a directory inside the root being installed into.  `own_link`:
/// `disk` is exactly the symlink this package's previous version shipped
/// there (another owner's link that leads to a directory is followed, never
/// removed).
pub fn displace(
    disk: &OnDisk,
    n: &FileEntry,
    rec: &Recorded<'_>,
    link_to_dir: bool,
    own_link: bool,
) -> Displace {
    let nothing_recorded = rec.shas.is_empty() && rec.links.is_empty() && !rec.dir;
    match (disk, n.symlink_target.as_deref()) {
        (OnDisk::Missing, _) => Displace::Leave,
        // The package places a symlink.
        (OnDisk::Link(t), Some(new)) if t == new => Displace::Leave,
        (OnDisk::Dir, Some(_)) if rec.dir => Displace::Leave,
        (d, Some(_)) if is_pristine(d, rec) => Displace::Leave,
        (_, Some(_)) if !rec.links.is_empty() || nothing_recorded => Displace::Keep,
        (_, Some(_)) => Displace::Save,
        // The package places a directory.  Its own previous link there is
        // removed (its layout changes; the caller checks what is below it),
        // and so is jpkg's file there; another link that leads to a
        // directory is followed, as it always was.
        (OnDisk::Dir, None) => Displace::Leave,
        (OnDisk::Link(_), None) if own_link => Displace::Remove,
        (OnDisk::File(_), None) if is_pristine(disk, rec) => Displace::Remove,
        (OnDisk::Link(_), None) if link_to_dir => Displace::Leave,
        (OnDisk::Link(_), None) if is_pristine(disk, rec) => Displace::Remove,
        (_, None) => Displace::Save,
    }
}

/// `dir/name` + `suffix`, e.g. `etc/x.conf` → `etc/x.conf.jpkg-new`.
pub fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The keep-or-delete decision behind upgrade-clean and remove, split out so
/// the error case is testable: a probe error keeps the object.
pub fn keep_probe(probe: io::Result<OnDisk>, rec: &Recorded<'_>) -> bool {
    match probe {
        Ok(d) => !is_pristine(&d, rec),
        Err(_) => true,
    }
}

/// For upgrade-clean and remove: true when the entry `e` of an installed
/// manifest at a config path must stay where it is because what is on disk
/// is not what `e` recorded.  For a directory entry that means anything but
/// a directory (or nothing): an admin's symlink, file or FIFO there is
/// kept.  Paths outside the config class are never kept here.
pub fn keep_on_disk(rootfs: &Path, e: &FileEntry) -> bool {
    if !is_config_path(&e.path) {
        return false;
    }
    let probe = on_disk(rootfs, &e.path);
    if let Err(ref err) = probe {
        log::warn!("jpkg: cannot check /{} ({err}); keeping it", e.path);
    }
    if e.is_dir {
        return !matches!(probe, Ok(OnDisk::Missing | OnDisk::Dir));
    }
    keep_probe(probe, &Recorded::entry(e))
}

/// Delete `<path>.jpkg-new` when it is a regular file holding package
/// content (one of `package_shas`).  Anything else there -- the admin's own
/// edit of it, a symlink, a directory -- is left alone.  Returns whether it
/// was removed.
pub fn drop_stale_new(rootfs: &Path, path: &str, package_shas: &[&str]) -> io::Result<bool> {
    let new = with_suffix(&rootfs.join(path), NEW_SUFFIX);
    match new.symlink_metadata() {
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {
            return Ok(false)
        }
        Err(e) => return Err(e),
        Ok(m) if !m.file_type().is_file() => return Ok(false),
        Ok(_) => {}
    }
    if package_shas.contains(&sha256_file(&new)?.as_str()) {
        fs::remove_file(&new)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// May jpkg write the packaged version to `<path>.jpkg-new`?  Only over
/// nothing, or over a regular file holding package content (one of
/// `package_shas`, e.g. an earlier offer).  An admin's work in progress
/// there, a symlink, a directory or a special file is left alone.
pub fn new_slot_free(rootfs: &Path, path: &str, package_shas: &[&str]) -> io::Result<bool> {
    let slot = format!("{path}{NEW_SUFFIX}");
    Ok(match on_disk(rootfs, &slot)? {
        OnDisk::Missing => true,
        OnDisk::File(h) => package_shas.contains(&h.as_str()),
        OnDisk::Link(_) | OnDisk::Dir | OnDisk::Other => false,
    })
}

/// True when `rootfs/path` is a directory with nothing in it (not followed).
pub fn is_empty_dir(rootfs: &Path, path: &str) -> io::Result<bool> {
    let abs = rootfs.join(path);
    match abs.symlink_metadata() {
        Ok(m) if m.file_type().is_dir() => Ok(fs::read_dir(&abs)?.next().is_none()),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// True when `rel` is something jpkg itself left next to a path the
/// manifest `owned` lists: an untouched offer (`<path>.jpkg-new`, a regular
/// file holding exactly the copy recorded for `<path>`), or the scratch
/// name a config path is written under (`<path>.jpkg-tmp`, a regular file
/// or symlink an interrupted install left).
pub fn is_package_leftover(
    rootfs: &Path,
    rel: &str,
    owned: &std::collections::HashMap<&str, &FileEntry>,
) -> bool {
    if let Some(base) = rel.strip_suffix(NEW_SUFFIX) {
        return match owned.get(base) {
            Some(e) if is_config(e) => {
                matches!(on_disk(rootfs, rel), Ok(OnDisk::File(h)) if h == e.sha256)
            }
            _ => false,
        };
    }
    if let Some(base) = rel.strip_suffix(TMP_SUFFIX) {
        // <path>.jpkg-tmp, or an offer's own scratch, <path>.jpkg-new.jpkg-tmp.
        let base = base.strip_suffix(NEW_SUFFIX).unwrap_or(base);
        return owned.get(base).is_some_and(|e| is_config_path(&e.path))
            && matches!(on_disk(rootfs, rel), Ok(OnDisk::File(_) | OnDisk::Link(_)));
    }
    false
}

/// Remove what an interrupted install left next to `path`: `<path>.jpkg-tmp`
/// and an offer's `<path>.jpkg-new.jpkg-tmp`, when they are a regular file or
/// symlink (jpkg's own scratch).  Returns whether it removed anything.
pub fn drop_scratch(rootfs: &Path, path: &str) -> io::Result<bool> {
    let abs = rootfs.join(path);
    let mut dropped = false;
    for tmp in [with_suffix(&abs, TMP_SUFFIX), with_suffix(&with_suffix(&abs, NEW_SUFFIX), TMP_SUFFIX)] {
        match tmp.symlink_metadata() {
            Ok(m) if m.file_type().is_file() || m.file_type().is_symlink() => {
                fs::remove_file(&tmp)?;
                dropped = true;
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(dropped)
}

/// Resolve `rel` inside `root` the way a chroot into `root` would: symlinks
/// are followed component by component, an absolute target starts again at
/// `root`, and `..` never climbs above it.  Returns the real path (under
/// `root`), or `None` when a component is missing, a link loops (more than
/// 40 hops) or cannot be read.  Never opens anything but directories'
/// metadata and links.
pub fn resolve_in_root(root: &Path, rel: &Path) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::path::Component;
    fn push_rev(pending: &mut Vec<OsString>, p: &Path) {
        for c in p.components().rev() {
            match c {
                Component::Normal(n) => pending.push(n.to_os_string()),
                Component::ParentDir => pending.push(OsString::from("..")),
                _ => {}
            }
        }
    }
    let mut pending: Vec<OsString> = Vec::new();
    push_rev(&mut pending, rel);
    let mut cur = root.to_path_buf();
    let mut hops = 0;
    while let Some(c) = pending.pop() {
        if c == ".." {
            if cur != root {
                cur.pop();
            }
            continue;
        }
        let next = cur.join(&c);
        let m = next.symlink_metadata().ok()?;
        if m.file_type().is_symlink() {
            hops += 1;
            if hops > 40 {
                return None;
            }
            let t = fs::read_link(&next).ok()?;
            if t.is_absolute() {
                cur = root.to_path_buf();
            }
            push_rev(&mut pending, &t);
        } else {
            cur = next;
        }
    }
    Some(cur)
}

/// True when `<path>.jpkg-new` exists as a regular file (never followed).
pub fn has_pending_new(rootfs: &Path, path: &str) -> bool {
    with_suffix(&rootfs.join(path), NEW_SUFFIX)
        .symlink_metadata()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, sha: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            sha256: sha.into(),
            size: 0,
            mode: 0o100644,
            symlink_target: None,
            is_dir: false,
        }
    }

    #[test]
    fn config_rule_is_regular_files_under_etc_minus_init_d_and_cron_d() {
        assert!(is_config(&file("etc/ntpd.conf", "a")));
        assert!(is_config(&file("etc/conf.d/sshd", "a")));
        assert!(is_config(&file("etc/sysctl.d/50-x.conf", "a")));
        assert!(!is_config(&file("etc/init.d/sshd", "a")));
        assert!(!is_config(&file("etc/cron.d/logrotate", "a")));
        assert!(!is_config(&file("etcetera/x", "a")));
        assert!(!is_config(&file("bin/etc/x", "a")));
        assert!(!is_config(&file("share/dhcpcd/dhcpcd.conf", "a")));
        let mut link = file("etc/ssl/cert.pem", "");
        link.symlink_target = Some("certs/ca-certificates.crt".into());
        assert!(!is_config(&link));
        let mut dir = file("etc/unbound", "0");
        dir.is_dir = true;
        assert!(!is_config(&dir));
    }

    #[test]
    fn config_action_table() {
        use Action::*;
        let f = |s: &str| OnDisk::File(s.to_string());
        let l = |s: &str| OnDisk::Link(s.to_string());
        let rec = |shas: &[&'static str], links: &[&'static str]| Recorded {
            shas: shas.to_vec(),
            links: links.to_vec(),
            dir: false,
        };
        // (on disk, new, recorded) → action
        let rows: Vec<(OnDisk, &str, Recorded<'_>, Action)> = vec![
            (OnDisk::Missing, "N", rec(&["O"], &[]), Install), // admin deleted it
            (OnDisk::Missing, "N", rec(&[], &[]), Install),    // first install
            (f("O"), "N", rec(&["O"], &[]), Install),          // unchanged, package changed
            (f("O"), "O", rec(&["O"], &[]), Install),          // unchanged, reinstall
            (f("N"), "N", rec(&["O"], &[]), Install),          // admin already has new
            (f("N"), "N", rec(&[], &[]), Install),             // pre-existing, identical
            (f("D"), "O", rec(&["O"], &[]), Keep),             // changed, package same
            (f("D"), "N", rec(&["O"], &[]), KeepAndNew),       // changed, package changed
            (f("D"), "N", rec(&[], &[]), KeepAndNew),          // pre-existing, differs
            (f("P"), "N", rec(&["O", "P"], &[]), Install),     // previous owner's copy
            // The package's own old symlink at P is pristine (row 7b).
            (l("old.conf"), "N", rec(&[], &["old.conf"]), Install),
            // An unrecorded symlink is the admin's, never written through.
            (l("/data/x"), "O", rec(&["O"], &[]), Keep),
            (l("/data/x"), "N", rec(&["O"], &["other"]), KeepAndNew),
            // A directory, FIFO or device is never pristine.  (A directory
            // jpkg recorded is decided by the caller after upgrade-clean.)
            (OnDisk::Other, "O", rec(&["O"], &[]), Keep),
            (OnDisk::Other, "N", rec(&["O"], &[]), KeepAndNew),
            (OnDisk::Dir, "N", rec(&["O"], &[]), KeepAndNew),
            (OnDisk::Dir, "N", Recorded { dir: true, ..rec(&[], &[]) }, KeepAndNew),
        ];
        for (disk, new, rec, want) in &rows {
            assert_eq!(action(disk, new, rec), *want, "{disk:?} new={new} rec={rec:?}");
        }
        // Other never gets Install, whatever was recorded.
        for new in ["O", "N", ""] {
            assert_ne!(action(&OnDisk::Other, new, &rec(&["O", "N", ""], &["x"])), Install);
        }
    }

    #[test]
    fn probe_error_keeps_the_file() {
        let e = file("etc/x.conf", "O");
        let rec = Recorded::entry(&e);
        let err = io::Error::new(io::ErrorKind::PermissionDenied, "nope");
        assert!(keep_probe(Err(err), &rec), "an unreadable file must never be deleted");
        assert!(!keep_probe(Ok(OnDisk::File("O".into())), &rec));
        assert!(!keep_probe(Ok(OnDisk::Missing), &rec));
        assert!(keep_probe(Ok(OnDisk::File("D".into())), &rec));
        assert!(keep_probe(Ok(OnDisk::Link("O".into())), &rec), "a link is not the file");
    }

    #[test]
    fn displace_table() {
        use Displace::*;
        let f = |s: &str| OnDisk::File(s.to_string());
        let l = |s: &str| OnDisk::Link(s.to_string());
        let rec = |shas: &[&'static str], links: &[&'static str], dir: bool| Recorded {
            shas: shas.to_vec(),
            links: links.to_vec(),
            dir,
        };
        let mut link = file("etc/x.conf", "");
        link.symlink_target = Some("x.d/main".into());
        let mut dir = file("etc/x.conf", "");
        dir.is_dir = true;
        // (on disk, new entry, recorded, link leads to a dir, this package's own old link) → what happens first
        let rows: Vec<(OnDisk, &FileEntry, Recorded<'_>, bool, bool, Displace)> = vec![
            // The package places a symlink.
            (OnDisk::Missing, &link, rec(&[], &[], false), false, false, Leave),
            (f("O"), &link, rec(&["O"], &[], false), false, false, Leave),      // jpkg's file: replaced
            (f("D"), &link, rec(&["O"], &[], false), false, false, Save),       // changed file, kind changes
            (l("x.d/main"), &link, rec(&[], &[], false), false, false, Leave),  // already that link
            (l("old"), &link, rec(&[], &["old"], false), false, false, Leave),  // jpkg's old link
            (l("/data/x"), &link, rec(&[], &["old"], false), false, false, Keep), // the admin retargeted it
            (f("D"), &link, rec(&[], &["old"], false), false, false, Keep),     // a file in place of the link
            (OnDisk::Dir, &link, rec(&[], &["old"], false), false, false, Keep),
            (l("/data/x"), &link, rec(&[], &[], false), false, false, Keep),    // nothing recorded: the admin's
            (f("D"), &link, rec(&[], &[], false), false, false, Keep),
            (l("/data/x"), &link, rec(&["O"], &[], false), false, false, Save), // admin link over a packaged file
            (OnDisk::Dir, &link, rec(&[], &[], true), false, false, Leave),     // upgrade-clean's case
            (OnDisk::Other, &link, rec(&["O"], &[], false), false, false, Save),
            // The package places a directory.
            (OnDisk::Missing, &dir, rec(&[], &[], false), false, false, Leave),
            (OnDisk::Dir, &dir, rec(&[], &[], false), false, false, Leave),     // merged into, as always
            (l("/data/x.d"), &dir, rec(&[], &[], false), true, false, Leave),   // followed, as always
            (l("/data/gone"), &dir, rec(&[], &[], false), false, false, Save),  // cannot be followed
            (l("old"), &dir, rec(&[], &["old"], false), false, false, Remove),  // jpkg's dead link
            (l("old"), &dir, rec(&[], &["old"], false), true, true, Remove),    // its own link to a dir: removed
            (l("old"), &dir, rec(&[], &["old"], false), true, false, Leave),    // another's link to a dir: followed
            (l("old"), &dir, rec(&[], &["old"], false), false, true, Remove),   // its own dead link
            (f("O"), &dir, rec(&["O"], &[], false), false, false, Remove),      // jpkg's file in the way
            (f("D"), &dir, rec(&["O"], &[], false), false, false, Save),
            (OnDisk::Other, &dir, rec(&[], &[], false), false, false, Save),
        ];
        for (disk, n, rec, to_dir, own, want) in &rows {
            assert_eq!(displace(disk, n, rec, *to_dir, *own), *want, "{disk:?} new={n:?} rec={rec:?} to_dir={to_dir} own={own}");
        }
    }

    #[test]
    fn keep_on_disk_keeps_whatever_replaced_a_recorded_directory() {
        let t = tempfile::TempDir::new().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("etc/svc")).unwrap();
        let mut d = file("etc/svc", "");
        d.is_dir = true;
        assert!(!keep_on_disk(r, &d), "still a directory");
        fs::remove_dir(r.join("etc/svc")).unwrap();
        assert!(!keep_on_disk(r, &d), "gone");
        std::os::unix::fs::symlink("../persist/svc", r.join("etc/svc")).unwrap();
        assert!(keep_on_disk(r, &d), "the admin's link");
        fs::remove_file(r.join("etc/svc")).unwrap();
        fs::write(r.join("etc/svc"), b"x").unwrap();
        assert!(keep_on_disk(r, &d), "the admin's file");
    }

    #[test]
    fn trust_store_is_package_data() {
        assert!(!is_config(&file("etc/ssl/certs/ca-certificates.crt", "a")));
        assert!(is_config(&file("etc/ssl/openssl.cnf", "a")));
        assert!(!is_config_path("etc/ssl/cert.pem"), "LibreSSL's default CA file: package data");
        assert!(is_config_path("etc/ssl/cert.pem.local"), "an exact path, not a prefix");
    }

    #[test]
    fn on_disk_sees_dirs_and_treats_a_file_parent_as_missing() {
        let t = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(t.path().join("etc/d")).unwrap();
        fs::write(t.path().join("etc/f"), b"x").unwrap();
        assert_eq!(on_disk(t.path(), "etc/d").unwrap(), OnDisk::Dir);
        assert_eq!(on_disk(t.path(), "etc/f/below").unwrap(), OnDisk::Missing);
        assert!(is_empty_dir(t.path(), "etc/d").unwrap());
        fs::write(t.path().join("etc/d/a"), b"x").unwrap();
        assert!(!is_empty_dir(t.path(), "etc/d").unwrap());
        assert!(!is_empty_dir(t.path(), "etc/f").unwrap());
        assert!(!is_empty_dir(t.path(), "etc/none").unwrap());
    }

    #[test]
    fn new_slot_is_free_only_for_nothing_or_package_content() {
        let t = tempfile::TempDir::new().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("etc")).unwrap();
        let pkg = sha256_file(&{
            let p = r.join("pkg");
            fs::write(&p, b"v1\n").unwrap();
            p
        })
        .unwrap();
        assert!(new_slot_free(r, "etc/a.conf", &[&pkg]).unwrap(), "nothing there");
        fs::write(r.join("etc/a.conf.jpkg-new"), b"v1\n").unwrap();
        assert!(new_slot_free(r, "etc/a.conf", &[&pkg]).unwrap(), "an earlier offer");
        fs::write(r.join("etc/a.conf.jpkg-new"), b"merging\n").unwrap();
        assert!(!new_slot_free(r, "etc/a.conf", &[&pkg]).unwrap(), "the admin's work");
        std::os::unix::fs::symlink(r.join("pkg"), r.join("etc/b.conf.jpkg-new")).unwrap();
        assert!(!new_slot_free(r, "etc/b.conf", &[&pkg]).unwrap(), "a link, even to package content");
        fs::create_dir(r.join("etc/c.conf.jpkg-new")).unwrap();
        assert!(!new_slot_free(r, "etc/c.conf", &[&pkg]).unwrap(), "an empty directory");
    }

    #[test]
    fn keep_on_disk_covers_symlinks_at_config_paths() {
        let t = tempfile::TempDir::new().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("etc/pki")).unwrap();
        let mut link = file("etc/pki/default.pem", "");
        link.symlink_target = Some("certs/ca.crt".into());
        std::os::unix::fs::symlink("certs/ca.crt", r.join("etc/pki/default.pem")).unwrap();
        assert!(!keep_on_disk(r, &link), "jpkg's own link");
        fs::remove_file(r.join("etc/pki/default.pem")).unwrap();
        fs::write(r.join("etc/pki/default.pem"), b"private CA\n").unwrap();
        assert!(keep_on_disk(r, &link), "the admin replaced the link with a file");
        let mut code = file("etc/init.d/svc", "O");
        code.symlink_target = None;
        fs::create_dir_all(r.join("etc/init.d")).unwrap();
        fs::write(r.join("etc/init.d/svc"), b"hacked\n").unwrap();
        assert!(!keep_on_disk(r, &code), "init.d is code");
    }

    #[test]
    fn resolve_in_root_follows_links_like_a_chroot() {
        let t = tempfile::TempDir::new().unwrap();
        let r = t.path().join("root");
        fs::create_dir_all(r.join("share/foo")).unwrap();
        fs::create_dir_all(r.join("etc")).unwrap();
        std::os::unix::fs::symlink("/share/foo", r.join("etc/abs")).unwrap();
        std::os::unix::fs::symlink("../share/foo", r.join("etc/rel")).unwrap();
        std::os::unix::fs::symlink("../../../../../share/foo", r.join("etc/climb")).unwrap();
        std::os::unix::fs::symlink("loop2", r.join("etc/loop1")).unwrap();
        std::os::unix::fs::symlink("loop1", r.join("etc/loop2")).unwrap();
        std::os::unix::fs::symlink("/nowhere", r.join("etc/dead")).unwrap();
        let want = Some(r.join("share/foo"));
        assert_eq!(resolve_in_root(&r, Path::new("etc/abs")), want, "absolute: starts again at the root");
        assert_eq!(resolve_in_root(&r, Path::new("etc/rel")), want);
        assert_eq!(resolve_in_root(&r, Path::new("etc/climb")), want, "'..' stops at the root");
        assert_eq!(resolve_in_root(&r, Path::new("etc/loop1")), None);
        assert_eq!(resolve_in_root(&r, Path::new("etc/dead")), None);
    }

    #[test]
    fn with_suffix_appends_to_file_name() {
        assert_eq!(
            with_suffix(Path::new("/r/etc/x.conf"), NEW_SUFFIX),
            PathBuf::from("/r/etc/x.conf.jpkg-new")
        );
    }
}
