//! nsenter — enter the namespaces of another process. Clean-room
//! implementation using open(2) on /proc/PID/ns/* files followed by
//! setns(2). No util-linux source consulted.
//!
//! Reference:
//!   - nsenter(1) man page (behavioural spec)
//!   - setns(2) man page
//!   - namespaces(7)
//!   - linux/sched.h CLONE_NEW* UAPI constants
//!
//! Subset supported:
//!   -t, --target PID          target process (required)
//!   -m, --mount[=FILE]        enter mount namespace
//!   -u, --uts[=FILE]          enter UTS namespace
//!   -i, --ipc[=FILE]          enter IPC namespace
//!   -n, --net[=FILE]          enter network namespace
//!   -p, --pid[=FILE]          enter PID namespace (affects children)
//!   -U, --user[=FILE]         enter user namespace
//!   -C, --cgroup[=FILE]       enter cgroup namespace
//!   -a, --all                 enter every namespace available
//!   -F, --no-fork             do not fork before exec
//!   --                        end of options; rest is COMMAND [ARGS]
//!
//! If no command is given, the user's $SHELL (or /bin/sh) is exec'd.

use jxutil::syscall::*;
use std::ffi::CString;
use std::os::raw::c_char;
use std::process::ExitCode;

#[derive(Default)]
struct NsWant {
    mount: Option<Option<String>>,
    uts:   Option<Option<String>>,
    ipc:   Option<Option<String>>,
    net:   Option<Option<String>>,
    pid:   Option<Option<String>>,
    user:  Option<Option<String>>,
    cgroup:Option<Option<String>>,
    time:  Option<Option<String>>,
}

fn open_path(p: &str) -> std::io::Result<i32> {
    let c = CString::new(p).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL byte in path")
    })?;
    // SAFETY:
    //   - `c.as_ptr()` is non-null and NUL-terminated (CString guarantees this).
    //   - `c` remains alive (in scope) until after `open` returns, so the
    //     pointer is valid for the entire duration of the syscall.
    //   - Flags: O_RDONLY (0) | O_CLOEXEC (0x80000). O_CLOEXEC ensures the fd
    //     does not leak into processes launched after fork+exec.
    //   - mode=0 is ignored for non-creating opens.
    let fd = unsafe { open(c.as_ptr(), 0 | 0x80000, 0) };
    if fd < 0 { Err(std::io::Error::last_os_error()) } else { Ok(fd) }
}

fn enter_ns(path: &str, nstype: i32) -> std::io::Result<()> {
    let fd = open_path(path)?;
    // SAFETY:
    //   - `fd` is >= 0 (checked by open_path) and refers to a namespace inode
    //     under /proc/PID/ns/ (or an explicit file).
    //   - `nstype` is one of the CLONE_NEW* constants, which the kernel
    //     validates against the actual ns type of the fd. On mismatch
    //     the kernel returns EINVAL, not UB.
    //   - After setns the fd is no longer needed — we close it immediately.
    let rc = unsafe { setns(fd, nstype) };
    let err = if rc < 0 { Some(std::io::Error::last_os_error()) } else { None };
    // SAFETY:
    //   - `fd` is >= 0 and was not previously closed. This is the unique close
    //     point — the fd is never stored elsewhere, so no double-close risk.
    unsafe { close(fd); }
    if let Some(e) = err { Err(e) } else { Ok(()) }
}

fn resolve(target: Option<i32>, file_opt: &Option<String>, name: &str) -> Option<String> {
    if let Some(Some(f)) = Some(file_opt).filter(|_| file_opt.is_some()) {
        return Some(f.clone());
    }
    target.map(|pid| format!("/proc/{}/ns/{}", pid, name))
}

fn take_inline(a: &str, long: &str, short: Option<&str>) -> (bool, Option<String>) {
    // --ns or --ns=FILE or -n or -n=FILE
    if a == long || Some(a) == short { return (true, None); }
    if let Some(rest) = a.strip_prefix(&format!("{}=", long)) {
        return (true, Some(rest.to_string()));
    }
    if let Some(s) = short {
        if let Some(rest) = a.strip_prefix(&format!("{}=", s)) {
            return (true, Some(rest.to_string()));
        }
    }
    (false, None)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut target: Option<i32> = None;
    let mut want = NsWant::default();
    let mut all = false;
    let mut no_fork = false;
    let mut cmd: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].clone();
        if a == "--" { cmd.extend_from_slice(&args[i+1..]); break; }
        if a == "-h" || a == "--help" {
            println!("Usage: nsenter [OPTIONS] [COMMAND [ARGS...]]");
            println!("Run a program with namespaces of another process.");
            println!("  -t, --target PID         target process");
            println!("  -a, --all                enter all namespaces");
            println!("  -m, --mount[=FILE]       mount ns");
            println!("  -u, --uts[=FILE]         UTS ns");
            println!("  -i, --ipc[=FILE]         IPC ns");
            println!("  -n, --net[=FILE]         network ns");
            println!("  -p, --pid[=FILE]         PID ns");
            println!("  -U, --user[=FILE]        user ns");
            println!("  -C, --cgroup[=FILE]      cgroup ns");
            println!("  -F, --no-fork            do not fork before exec");
            return ExitCode::SUCCESS;
        }
        if a == "-t" || a == "--target" {
            i += 1;
            target = args.get(i).and_then(|s| s.parse().ok());
            if target.is_none() { eprintln!("nsenter: bad pid"); return ExitCode::FAILURE; }
            i += 1; continue;
        }
        if let Some(rest) = a.strip_prefix("--target=") {
            target = rest.parse().ok();
            i += 1; continue;
        }
        if a == "-a" || a == "--all" { all = true; i += 1; continue; }
        if a == "-F" || a == "--no-fork" { no_fork = true; i += 1; continue; }

        let checks: [(&str, Option<&str>, &dyn Fn(&mut NsWant, Option<String>)); 8] = [
            ("--mount",  Some("-m"), &|w, f| w.mount  = Some(f)),
            ("--uts",    Some("-u"), &|w, f| w.uts    = Some(f)),
            ("--ipc",    Some("-i"), &|w, f| w.ipc    = Some(f)),
            ("--net",    Some("-n"), &|w, f| w.net    = Some(f)),
            ("--pid",    Some("-p"), &|w, f| w.pid    = Some(f)),
            ("--user",   Some("-U"), &|w, f| w.user   = Some(f)),
            ("--cgroup", Some("-C"), &|w, f| w.cgroup = Some(f)),
            ("--time",   None,       &|w, f| w.time   = Some(f)),
        ];
        let mut consumed = false;
        for (long, short, set) in checks.iter() {
            let (hit, inline) = take_inline(&a, long, *short);
            if hit {
                set(&mut want, inline);
                consumed = true; break;
            }
        }
        if consumed { i += 1; continue; }

        // Not an option; rest is the command.
        cmd.extend_from_slice(&args[i..]);
        break;
    }

    // B11 fix: check if ANY namespace was requested (not just explicit files).
    if target.is_none() && !any_ns_requested(&want) {
        eprintln!("nsenter: --target or at least one --NS=FILE is required");
        return ExitCode::FAILURE;
    }

    // If --all is set, pull every ns file the target exposes.
    if all {
        if want.mount.is_none()  { want.mount  = Some(None); }
        if want.uts.is_none()    { want.uts    = Some(None); }
        if want.ipc.is_none()    { want.ipc    = Some(None); }
        if want.net.is_none()    { want.net    = Some(None); }
        if want.pid.is_none()    { want.pid    = Some(None); }
        if want.user.is_none()   { want.user   = Some(None); }
        if want.cgroup.is_none() { want.cgroup = Some(None); }
        // B13 fix: --all should include time namespace too.
        if want.time.is_none()   { want.time   = Some(None); }
    }

    // Kernel ordering note: user NS must be entered first if present,
    // otherwise subsequent setns calls may be denied by LSM checks.
    let order: [(&Option<Option<String>>, &str, i32); 8] = [
        (&want.user,   "user",   CLONE_NEWUSER),
        (&want.cgroup, "cgroup", CLONE_NEWCGROUP),
        (&want.ipc,    "ipc",    CLONE_NEWIPC),
        (&want.uts,    "uts",    CLONE_NEWUTS),
        (&want.net,    "net",    CLONE_NEWNET),
        (&want.pid,    "pid",    CLONE_NEWPID),
        (&want.mount,  "mnt",    CLONE_NEWNS),
        (&want.time,   "time",   CLONE_NEWTIME),
    ];
    for (slot, name, nstype) in order.iter() {
        if let Some(file_opt) = slot.as_ref() {
            let path = resolve(target, file_opt, name);
            if let Some(p) = path {
                if let Err(e) = enter_ns(&p, *nstype) {
                    // Missing time ns is OK on old kernels.
                    if *name == "time" && e.raw_os_error() == Some(2) { continue; }
                    eprintln!("nsenter: setns {}: {}", p, e);
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    // Fork is required for PID ns entry to actually affect the exec'd
    // process (setns on PID ns only takes effect in children).
    let needs_fork = want.pid.is_some() && !no_fork;
    if needs_fork {
        // SAFETY:
        //   - fork() has no arguments and is always safe to call.
        //   - Returns: child PID (>0) in the parent, 0 in the child, -1 on error.
        //   - Between fork() and exec_command() in the child, we only call
        //     async-signal-safe functions (eprintln uses write(2) which is safe;
        //     CString construction allocates but is acceptable since we're the
        //     only thread in the child post-fork).
        let pid = unsafe { fork() };
        if pid < 0 {
            eprintln!("nsenter: fork: {}", std::io::Error::last_os_error());
            return ExitCode::FAILURE;
        }
        if pid > 0 {
            let mut status: i32 = 0;
            // SAFETY:
            //   - `pid` is > 0 (a valid child PID we just forked).
            //   - `&mut status` is a non-null, aligned, writable *mut i32.
            //   - options=0 means block until the child exits or is signalled.
            let wret = unsafe { waitpid(pid, &mut status, 0) };
            // B12 fix: properly decode wait status for signal exits.
            if wret < 0 {
                eprintln!("nsenter: waitpid: {}", std::io::Error::last_os_error());
                return ExitCode::from(1);
            }
            let exit_code: u8 = if (status & 0x7f) == 0 {
                // WIFEXITED: child called exit() or returned from main.
                ((status >> 8) & 0xff) as u8
            } else {
                // WIFSIGNALED: child killed by signal. Shell convention: 128+signum.
                let sig = (status & 0x7f) as u8;
                128u8.saturating_add(sig)
            };
            return ExitCode::from(exit_code);
        }
    }

    exec_command(&cmd)
}

/// B11 fix: check if ANY namespace was requested (Some(None) or Some(Some(file))).
fn any_ns_requested(w: &NsWant) -> bool {
    w.mount.is_some() || w.uts.is_some() || w.ipc.is_some()
        || w.net.is_some() || w.pid.is_some() || w.user.is_some()
        || w.cgroup.is_some() || w.time.is_some()
}

fn exec_command(cmd: &[String]) -> ExitCode {
    let (prog, argv) = if cmd.is_empty() {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (sh.clone(), vec![sh])
    } else {
        (cmd[0].clone(), cmd.to_vec())
    };
    let cprog = CString::new(prog.as_str()).unwrap();
    let cargs: Vec<CString> = argv.iter().map(|s| CString::new(s.as_str()).unwrap()).collect();
    let mut ptrs: Vec<*const c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    // SAFETY:
    //   - `cprog.as_ptr()` is non-null and NUL-terminated (CString invariant).
    //   - `ptrs` is a NULL-terminated array of non-null, NUL-terminated C
    //     string pointers.  The pointed-to `CString` values (`cargs`) remain
    //     alive for the entire duration of the call because they are still in
    //     scope (the Vec is not dropped until after execvp returns on error).
    //   - `cprog` and `cargs` all outlive the `unsafe` block.
    //   - On success, execvp replaces the address space and never returns.
    //   - On error (file not found, permission denied, etc.) execvp returns -1
    //     and we report via last_os_error().
    unsafe { execvp(cprog.as_ptr(), ptrs.as_ptr()); }
    eprintln!("nsenter: exec {}: {}", prog, std::io::Error::last_os_error());
    ExitCode::from(127)
}
