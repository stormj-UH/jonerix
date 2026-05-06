//! chsh — change a user's login shell.
//! Clean-room implementation for jonerix:
//!   - run with no args: prints a numbered list of /bin shells and prompts
//!   - `-l`, `--list`: just list the shells
//!   - `-n N`, `--select-number N`: pick the Nth shell from the list
//!   - `-s SHELL` or bare SHELL: pick by path or short name
//!   - jonerix only allows shells that live in /bin
//!   - falls back to /bin/shadow-chsh for the privileged passwd write
//!     when run as a non-root user (we are not setuid)
//! No util-linux source consulted.

use std::collections::BTreeSet;
use std::env;
use std::ffi::CString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

extern "C" {
    fn geteuid() -> u32;
    fn getuid() -> u32;
    fn chown(path: *const std::os::raw::c_char, owner: u32, group: u32) -> i32;
}

const SHADOW_CHSH: &str = "/bin/shadow-chsh";

fn main() -> ExitCode {
    let passwd_path = env::var("JONERIX_PASSWD_FILE").unwrap_or_else(|_| "/etc/passwd".to_string());
    let shells_path = env::var("JONERIX_SHELLS_FILE").unwrap_or_else(|_| "/etc/shells".to_string());

    let mut list_only = false;
    let mut shell_arg: Option<String> = None;
    let mut user_arg: Option<String> = None;
    let mut number: Option<usize> = None;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "-l" | "--list" => {
                list_only = true;
            }
            "-s" | "--shell" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    eprintln!("chsh: missing argument for {}", arg);
                    return ExitCode::FAILURE;
                };
                shell_arg = Some(value.clone());
            }
            "-n" | "--select-number" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    eprintln!("chsh: missing argument for {}", arg);
                    return ExitCode::FAILURE;
                };
                match value.parse::<usize>() {
                    Ok(n) if n >= 1 => number = Some(n),
                    _ => {
                        eprintln!("chsh: --select-number wants a positive integer, got {}", value);
                        return ExitCode::FAILURE;
                    }
                }
            }
            _ if arg.starts_with("--shell=") => {
                shell_arg = Some(arg[8..].to_string());
            }
            _ if arg.starts_with("--select-number=") => match arg[16..].parse::<usize>() {
                Ok(n) if n >= 1 => number = Some(n),
                _ => {
                    eprintln!(
                        "chsh: --select-number wants a positive integer, got {}",
                        &arg[16..]
                    );
                    return ExitCode::FAILURE;
                }
            },
            _ if shell_arg.is_none() => {
                shell_arg = Some(arg.clone());
            }
            _ if user_arg.is_none() => {
                user_arg = Some(arg.clone());
            }
            _ => {
                eprintln!("chsh: unexpected argument '{}'", arg);
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let shells = list_shells(&shells_path);

    if list_only {
        for shell in &shells {
            println!("{}", shell.display());
        }
        return ExitCode::SUCCESS;
    }

    let euid = unsafe { geteuid() };
    let uid = unsafe { getuid() };
    let entries = match read_passwd(&passwd_path) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("chsh: cannot read {}: {}", passwd_path, err);
            return ExitCode::FAILURE;
        }
    };

    let Some(current) = current_user(&entries, uid) else {
        eprintln!("chsh: current uid {} not found in {}", uid, passwd_path);
        return ExitCode::FAILURE;
    };

    let target_user = user_arg.clone().unwrap_or_else(|| current.name.clone());
    if euid != 0 && target_user != current.name {
        eprintln!("chsh: only root can change another user's shell");
        return ExitCode::FAILURE;
    }

    let Some(target_entry) = entries.iter().find(|entry| entry.name == target_user) else {
        eprintln!("chsh: user '{}' not found", target_user);
        return ExitCode::FAILURE;
    };

    // Resolve the requested shell. Three paths:
    //   --select-number N -> shells[N-1]
    //   shell_arg / bare positional -> resolve_shell()
    //   nothing -> interactive prompt with the numbered list
    let shell_token = if let Some(n) = number {
        let Some(picked) = shells.get(n - 1) else {
            eprintln!(
                "chsh: --select-number {} is out of range (have {} shell(s)); run `chsh -l` to see them",
                n,
                shells.len()
            );
            return ExitCode::FAILURE;
        };
        picked.to_string_lossy().into_owned()
    } else if let Some(s) = shell_arg.clone() {
        s
    } else {
        match interactive_pick(&shells, &target_entry.shell) {
            Ok(Some(token)) => token,
            Ok(None) => return ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("chsh: {}", err);
                return ExitCode::FAILURE;
            }
        }
    };

    let shell_path = match resolve_shell(&shell_token) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("chsh: {}", err);
            return ExitCode::FAILURE;
        }
    };

    if target_entry.shell == shell_path {
        println!("{} already uses {}", target_user, shell_path);
        return ExitCode::SUCCESS;
    }

    // Run as root: write directly. Otherwise hand off to shadow-chsh for
    // the privileged write. We deliberately don't ship setuid because
    // shadow-chsh is already audited for that.
    if euid == 0 {
        if let Err(err) = ensure_shells_contains(&shells_path, &shell_path) {
            eprintln!("chsh: cannot update {}: {}", shells_path, err);
            return ExitCode::FAILURE;
        }
        if let Err(err) = write_passwd(&passwd_path, &entries, &target_user, &shell_path) {
            eprintln!("chsh: cannot update {}: {}", passwd_path, err);
            return ExitCode::FAILURE;
        }
        println!("{}: {} -> {}", target_user, target_entry.shell, shell_path);
        ExitCode::SUCCESS
    } else {
        delegate_to_shadow_chsh(&shell_path, &target_user, &current.name)
    }
}

/// Show the numbered list of shells, mark the user's current shell, and prompt.
/// Returns Ok(None) if the user cancels.
fn interactive_pick(shells: &[PathBuf], current_shell: &str) -> io::Result<Option<String>> {
    if shells.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no login shells available under /bin",
        ));
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "Available login shells (jonerix accepts only /bin):")?;
    writeln!(out)?;
    for (idx, shell) in shells.iter().enumerate() {
        let path = shell.to_string_lossy();
        let suffix = symlink_target_note(shell);
        let marker = if path == current_shell { " *" } else { "" };
        writeln!(out, "  {:>2}) {}{}{}", idx + 1, path, suffix, marker)?;
    }
    writeln!(out)?;
    writeln!(out, "* = your current shell")?;
    writeln!(out)?;
    write!(
        out,
        "Pick by number (1-{}), name (e.g. brash), or absolute path. Press Enter or 'q' to cancel: ",
        shells.len()
    )?;
    out.flush()?;
    drop(out);

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let answer = line.trim();
    if answer.is_empty() || answer.eq_ignore_ascii_case("q") {
        return Ok(None);
    }
    if let Ok(n) = answer.parse::<usize>() {
        let Some(picked) = shells.get(n.saturating_sub(1)) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{}' is out of range; have {} shell(s)", n, shells.len()),
            ));
        };
        return Ok(Some(picked.to_string_lossy().into_owned()));
    }
    Ok(Some(answer.to_string()))
}

/// Best-effort " -> target" annotation when the entry is a symlink (e.g. /bin/bash -> brash).
fn symlink_target_note(path: &Path) -> String {
    match fs::read_link(path) {
        Ok(target) => format!(" -> {}", target.display()),
        Err(_) => String::new(),
    }
}

/// We are not setuid, so a non-root caller cannot rewrite /etc/passwd directly.
/// Hand off to shadow-chsh, which is the audited setuid backend.
fn delegate_to_shadow_chsh(shell_path: &str, target_user: &str, caller: &str) -> ExitCode {
    if !is_executable(Path::new(SHADOW_CHSH)) {
        eprintln!(
            "chsh: cannot rewrite /etc/passwd as a non-root user, and {} is not installed.\n\
             Run `sudo chsh -s {}` (or rerun this command as root).",
            SHADOW_CHSH, shell_path
        );
        return ExitCode::FAILURE;
    }

    let mut cmd = Command::new(SHADOW_CHSH);
    cmd.arg("-s").arg(shell_path);
    if target_user != caller {
        cmd.arg(target_user);
    }
    let err = cmd.exec();
    eprintln!("chsh: failed to exec {}: {}", SHADOW_CHSH, err);
    ExitCode::FAILURE
}

fn print_usage() {
    println!("Usage: chsh                          # interactive: list shells, prompt to pick");
    println!("       chsh -l | --list              # list available login shells");
    println!("       chsh -n N | --select-number N # pick the Nth shell from the list");
    println!("       chsh -s SHELL [USER]          # set USER's shell (default: caller)");
    println!("       chsh SHELL [USER]             # same as -s");
    println!();
    println!("Login shells must live in /bin. Common picks: brash, bash, mksh, zsh, sh.");
    println!("Run as root (or via sudo) to change other users' shells.");
}

#[derive(Clone)]
struct PasswdEntry {
    raw: String,
    name: String,
    uid: u32,
    shell: String,
}

fn read_passwd(path: &str) -> io::Result<Vec<PasswdEntry>> {
    let data = fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for line in data.lines() {
        let mut parts = line.split(':');
        let name = parts.next().unwrap_or("").to_string();
        let _passwd = parts.next();
        let uid = parts
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX);
        let _gid = parts.next();
        let _gecos = parts.next();
        let _home = parts.next();
        let shell = parts.next().unwrap_or("").to_string();
        entries.push(PasswdEntry {
            raw: line.to_string(),
            name,
            uid,
            shell,
        });
    }
    Ok(entries)
}

fn current_user(entries: &[PasswdEntry], uid: u32) -> Option<PasswdEntry> {
    entries.iter().find(|entry| entry.uid == uid).cloned()
}

fn resolve_shell(token: &str) -> Result<String, String> {
    let candidate = if token.contains('/') {
        PathBuf::from(token)
    } else {
        Path::new("/bin").join(token)
    };

    if candidate.parent() != Some(Path::new("/bin")) {
        return Err(format!(
            "{} is outside /bin; jonerix login shells must live in /bin",
            candidate.display()
        ));
    }

    if !is_executable(&candidate) {
        return Err(format!(
            "{} is not an executable shell",
            candidate.display()
        ));
    }

    Ok(candidate.to_string_lossy().into_owned())
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    meta.is_file() && (meta.permissions().mode() & 0o111) != 0
}

fn list_shells(shells_path: &str) -> Vec<PathBuf> {
    let mut shells = BTreeSet::new();

    if let Ok(data) = fs::read_to_string(shells_path) {
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = PathBuf::from(line);
            if is_executable(&path) {
                shells.insert(path);
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/bin") {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if looks_like_shell(name) && is_executable(&path) {
                shells.insert(path);
            }
        }
    }

    shells
        .into_iter()
        .filter(|p| p.parent() == Some(Path::new("/bin")))
        .collect()
}

fn looks_like_shell(name: &str) -> bool {
    matches!(
        name,
        "sh" | "ash"
            | "dash"
            | "bash"
            | "brash"
            | "rash"
            | "mksh"
            | "lksh"
            | "ksh"
            | "oksh"
            | "zsh"
            | "fish"
            | "yash"
            | "xonsh"
            | "nu"
    )
}

fn ensure_shells_contains(shells_path: &str, shell_path: &str) -> io::Result<()> {
    let mut lines = if let Ok(data) = fs::read_to_string(shells_path) {
        data.lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
    } else {
        vec![
            "# /etc/shells — valid login shells".to_string(),
            "/bin/sh".to_string(),
            "/bin/mksh".to_string(),
        ]
    };

    if lines.iter().any(|line| line.trim() == shell_path) {
        return Ok(());
    }

    lines.push(shell_path.to_string());
    write_text_atomic(shells_path, &format!("{}\n", lines.join("\n")))
}

fn write_passwd(path: &str, entries: &[PasswdEntry], user: &str, shell: &str) -> io::Result<()> {
    let mut out = String::new();
    let mut changed = false;

    for entry in entries {
        if entry.name == user {
            let mut parts = entry
                .raw
                .split(':')
                .map(|part| part.to_string())
                .collect::<Vec<_>>();
            if parts.len() < 7 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "malformed passwd entry",
                ));
            }
            parts[6] = shell.to_string();
            out.push_str(&parts.join(":"));
            changed = true;
        } else {
            out.push_str(&entry.raw);
        }
        out.push('\n');
    }

    if !changed {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "user entry not found",
        ));
    }

    write_text_atomic(path, &out)
}

fn write_text_atomic(path: &str, data: &str) -> io::Result<()> {
    let meta = fs::metadata(path).ok();
    let mode = meta
        .as_ref()
        .map(|m| m.permissions().mode())
        .unwrap_or(0o644);
    let uid = meta.as_ref().map(|m| m.uid());
    let gid = meta.as_ref().map(|m| m.gid());

    let tmp = format!("{}.tmp.{}", path, std::process::id());
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    maybe_chown(&tmp, uid, gid)?;
    fs::rename(&tmp, path)
}

fn maybe_chown(path: &str, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    let (Some(uid), Some(gid)) = (uid, gid) else {
        return Ok(());
    };

    let c_path = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
    let rc = unsafe { chown(c_path.as_ptr(), uid, gid) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
