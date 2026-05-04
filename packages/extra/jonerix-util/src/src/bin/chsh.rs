//! chsh — change a user's login shell by editing /etc/passwd.
//! Clean-room implementation for jonerix with a small scope:
//!   - list installed/login shells (`-l`, `--list`)
//!   - change the caller's shell or, as root, another user's shell
//!   - accept either a shell path or a short name such as `mksh`
//! No util-linux source consulted.

use std::collections::BTreeSet;
use std::env;
use std::ffi::CString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

extern "C" {
    fn geteuid() -> u32;
    fn getuid() -> u32;
    fn chown(path: *const std::os::raw::c_char, owner: u32, group: u32) -> i32;
}

fn main() -> ExitCode {
    let passwd_path = env::var("JONERIX_PASSWD_FILE").unwrap_or_else(|_| "/etc/passwd".to_string());
    let shells_path = env::var("JONERIX_SHELLS_FILE").unwrap_or_else(|_| "/etc/shells".to_string());

    let mut list_only = false;
    let mut shell_arg: Option<String> = None;
    let mut user_arg: Option<String> = None;

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
            _ if arg.starts_with("--shell=") => {
                shell_arg = Some(arg[8..].to_string());
            }
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

    if list_only {
        for shell in list_shells(&shells_path) {
            println!("{}", shell.display());
        }
        return ExitCode::SUCCESS;
    }

    let Some(shell_token) = shell_arg else {
        print_usage();
        return ExitCode::FAILURE;
    };

    let euid = unsafe { geteuid() };
    let uid = unsafe { getuid() };
    let entries = match read_passwd(&passwd_path) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("chsh: cannot read {}: {}", passwd_path, err);
            return ExitCode::FAILURE;
        }
    };

    let current = match current_user(&entries, uid) {
        Some(entry) => entry,
        None => {
            eprintln!("chsh: current uid {} not found in {}", uid, passwd_path);
            return ExitCode::FAILURE;
        }
    };

    let target_user = user_arg.unwrap_or_else(|| current.name.clone());
    if euid != 0 && target_user != current.name {
        eprintln!("chsh: only root can change another user's shell");
        return ExitCode::FAILURE;
    }

    let Some(target_entry) = entries.iter().find(|entry| entry.name == target_user) else {
        eprintln!("chsh: user '{}' not found", target_user);
        return ExitCode::FAILURE;
    };

    let shell_path = match resolve_shell(&shell_token) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("chsh: {}", err);
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = ensure_shells_contains(&shells_path, &shell_path) {
        eprintln!("chsh: cannot update {}: {}", shells_path, err);
        return ExitCode::FAILURE;
    }

    if target_entry.shell == shell_path {
        println!("{} already uses {}", target_user, shell_path);
        return ExitCode::SUCCESS;
    }

    if let Err(err) = write_passwd(&passwd_path, &entries, &target_user, &shell_path) {
        eprintln!("chsh: cannot update {}: {}", passwd_path, err);
        return ExitCode::FAILURE;
    }

    println!("{}: {} -> {}", target_user, target_entry.shell, shell_path);
    ExitCode::SUCCESS
}

fn print_usage() {
    println!("Usage: chsh [-l] [-s SHELL] [USER]");
    println!("       chsh SHELL [USER]");
    println!();
    println!("  -l, --list          list available login shells");
    println!("  -s, --shell SHELL   select a shell by path or short name");
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
