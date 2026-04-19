//! /sys helpers. Most util-linux info tools read a mix of /proc (for
//! per-record text blobs) and /sys (for structured topology trees).

use std::fs;
use std::io;
use std::path::Path;

/// Read a one-line value file, trim whitespace.
pub fn read_line(path: impl AsRef<Path>) -> io::Result<String> {
    let s = fs::read_to_string(path)?;
    Ok(s.trim().to_string())
}

/// Try to read a one-line value file; return None on any error.
pub fn try_line(path: impl AsRef<Path>) -> Option<String> {
    read_line(path).ok()
}

/// Enumerate numeric child directories of `parent` matching `prefix`,
/// e.g. `/sys/devices/system/cpu` with prefix "cpu" returns the IDs
/// of every "cpu0", "cpu1", ... subdir.
pub fn enum_numeric(parent: &str, prefix: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let dir = match fs::read_dir(parent) { Ok(d) => d, Err(_) => return out };
    for e in dir.flatten() {
        let name = match e.file_name().into_string() { Ok(s) => s, Err(_) => continue };
        if let Some(rest) = name.strip_prefix(prefix) {
            if let Ok(n) = rest.parse::<u32>() { out.push(n); }
        }
    }
    out.sort();
    out
}

/// Parse a Linux cpumask-list, e.g. "0-3,7,9-11" into a Vec<u32>.
/// Used for topology masks (core_siblings_list, thread_siblings_list).
pub fn parse_cpulist(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.trim().split(',') {
        if part.is_empty() { continue; }
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                for n in a..=b { out.push(n); }
            }
        } else if let Ok(n) = part.parse::<u32>() {
            out.push(n);
        }
    }
    out
}
