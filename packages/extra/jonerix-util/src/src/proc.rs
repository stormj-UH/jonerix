//! /proc parsing helpers. Every file under /proc is a textual
//! pseudo-file with a documented (procfs(5)) format, so no kernel
//! headers are needed to interpret them.

use std::fs;
use std::io;

/// Read a whole file as a String, trimming trailing newlines.
pub fn read_trim(path: &str) -> io::Result<String> {
    let s = fs::read_to_string(path)?;
    Ok(s.trim_end().to_string())
}

/// Parse a key/value `/proc/cpuinfo`-style file. Lines look like
/// `key<whitespace>:<whitespace>value`, blank lines separate records.
/// Returns a vec of records, each a vec of (key, value) pairs preserving
/// order within the record.
pub fn parse_cpuinfo(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = Vec::new();
    let mut cur: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
            continue;
        }
        if let Some(colon) = line.find(':') {
            let k = line[..colon].trim().to_string();
            let v = line[colon+1..].trim().to_string();
            cur.push((k, v));
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// Read /proc/meminfo into a Vec of (key, value-with-unit).
pub fn read_meminfo() -> io::Result<Vec<(String, String)>> {
    let s = fs::read_to_string("/proc/meminfo")?;
    Ok(s.lines().filter_map(|l| {
        let c = l.find(':')?;
        Some((l[..c].trim().to_string(), l[c+1..].trim().to_string()))
    }).collect())
}
