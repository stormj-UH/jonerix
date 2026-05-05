//! ionice — get/set I/O scheduling class and priority for a process.
//! Clean-room implementation driving the ioprio_get(2) / ioprio_set(2)
//! Linux syscalls directly. No util-linux source consulted.
//!
//! Reference:
//!   - ioprio_set(2), ioprio_get(2) man pages (behavioural spec)
//!   - linux/ioprio.h UAPI constants (class/data bit packing)
//!   - Documentation/block/ioprio.rst

#![deny(unsafe_op_in_unsafe_fn)]

use jxutil::syscall::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::process::ExitCode;

fn parse_class(s: &str) -> Option<i32> {
    match s {
        "0" | "none" => Some(IOPRIO_CLASS_NONE),
        "1" | "realtime" | "rt" => Some(IOPRIO_CLASS_RT),
        "2" | "best-effort" | "be" => Some(IOPRIO_CLASS_BE),
        "3" | "idle" => Some(IOPRIO_CLASS_IDLE),
        _ => None,
    }
}

fn class_name(c: i32) -> &'static str {
    match c { 0 => "none", 1 => "realtime", 2 => "best-effort", 3 => "idle", _ => "?" }
}

/// Show ioprio for a given PID. Returns true on success, false on error.
fn show_pid(pid: i32) -> bool {
    let v = ioprio_get(IOPRIO_WHO_PROCESS, pid);
    if v < 0 {
        eprintln!("ionice: ioprio_get: {}", std::io::Error::last_os_error());
        return false;
    }
    let v = v as i32;
    let c = ioprio_class(v);
    let d = ioprio_data(v);
    // B08 fix: util-linux prints just "none" or "idle" (no prio) for those classes.
    match c {
        IOPRIO_CLASS_NONE => println!("none"),
        IOPRIO_CLASS_IDLE => println!("idle"),
        _ => println!("{}: prio {}", class_name(c), d),
    }
    true
}

fn apply_and_exec(class: Option<i32>, level: Option<i32>, argv: &[String]) -> ExitCode {
    let c = class.unwrap_or(IOPRIO_CLASS_BE);
    let d = if c == IOPRIO_CLASS_IDLE { 0 } else { level.unwrap_or(4) };
    let packed = ioprio_pack(c, d);
    if ioprio_set(IOPRIO_WHO_PROCESS, 0, packed) < 0 {
        eprintln!("ionice: ioprio_set: {}", std::io::Error::last_os_error());
        return ExitCode::FAILURE;
    }
    let prog = match argv.first() { Some(s) => s, None => return ExitCode::SUCCESS };
    let cprog = match CString::new(prog.as_str()) {
        Ok(c) => c,
        Err(_) => { eprintln!("ionice: NUL byte in command"); return ExitCode::FAILURE; }
    };
    let cargs: Vec<CString> = match argv.iter()
        .map(|s| CString::new(s.as_str()).map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(_) => { eprintln!("ionice: NUL byte in argument"); return ExitCode::FAILURE; }
    };
    let mut ptrs: Vec<*const c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    // SAFETY:
    //   - cprog.as_ptr() is non-null, NUL-terminated (CString invariant).
    //   - ptrs is NULL-terminated array of valid C string pointers.
    //   - All CStrings remain alive until execvp returns on error.
    unsafe { execvp(cprog.as_ptr(), ptrs.as_ptr()); }
    eprintln!("ionice: cannot exec '{}': {}", prog, std::io::Error::last_os_error());
    ExitCode::from(127)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("Usage:");
        println!("  ionice [-c CLASS] [-n LEVEL] -p PID...");
        println!("  ionice [-c CLASS] [-n LEVEL] COMMAND [ARGS...]");
        println!("Classes: 1 realtime, 2 best-effort, 3 idle, 0 none");
        return ExitCode::SUCCESS;
    }
    if args.is_empty() {
        if !show_pid(0) { return ExitCode::FAILURE; }
        return ExitCode::SUCCESS;
    }

    let mut class: Option<i32> = None;
    let mut level: Option<i32> = None;
    let mut pids:  Vec<i32>    = Vec::new();
    let mut cmd:   Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-c" | "--class" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    class = parse_class(v);
                    if class.is_none() { eprintln!("ionice: bad class '{}'", v); return ExitCode::FAILURE; }
                }
            }
            "-n" | "--classdata" => {
                i += 1;
                level = args.get(i).and_then(|v| v.parse().ok());
                if level.is_none() { eprintln!("ionice: bad level"); return ExitCode::FAILURE; }
            }
            "-p" | "--pid" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<i32>().ok()) { pids.push(v); }
                else { eprintln!("ionice: bad pid"); return ExitCode::FAILURE; }
            }
            "-t" | "--ignore" => {},
            _ => { cmd.extend_from_slice(&args[i..]); break; }
        }
        i += 1;
    }

    if !pids.is_empty() {
        if class.is_none() && level.is_none() {
            let mut rc = ExitCode::SUCCESS;
            for p in &pids { if !show_pid(*p) { rc = ExitCode::FAILURE; } }
            return rc;
        }
        let c = class.unwrap_or(IOPRIO_CLASS_BE);
        let d = if c == IOPRIO_CLASS_IDLE { 0 } else { level.unwrap_or(4) };
        let packed = ioprio_pack(c, d);
        let mut rc = ExitCode::SUCCESS;
        for p in &pids {
            if ioprio_set(IOPRIO_WHO_PROCESS, *p, packed) < 0 {
                eprintln!("ionice: ioprio_set pid {}: {}", p, std::io::Error::last_os_error());
                rc = ExitCode::FAILURE;
            }
        }
        return rc;
    }

    if !cmd.is_empty() {
        return apply_and_exec(class, level, &cmd);
    }

    if !show_pid(0) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
