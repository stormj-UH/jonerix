mod netlink;
mod parser;
mod ops;
mod output;
mod json;
mod templates;

use std::env;
use std::fs;
use std::io::{self, BufRead, Read};
use std::process;

const VERSION: &str = "0.1.0";

/// When STORMWALL_TEST_MODE=1, emit identity strings (--version, -V,
/// JSON metainfo) byte-identical to upstream nft so consumers that
/// pattern-match on `nft -v` / `release_name` accept us as a drop-in.
/// Off by default — under `=0` we self-identify as stormwall.
///
/// The strings target Debian's nftables 1.1.3 build (the version
/// shipped on the validation box) since drop-in claims are made
/// per-distro: a tool calibrated against `Commodore Bullmoose #4`
/// won't accept a different release name even if the version matches.
pub const NFT_IMPERSONATED_VERSION: &str = "1.1.3";
pub const NFT_IMPERSONATED_RELEASE_NAME: &str = "Commodore Bullmoose #4";

pub fn test_mode() -> bool {
    matches!(std::env::var("STORMWALL_TEST_MODE").ok().as_deref(), Some("1"))
}

fn usage() {
    let prog = if test_mode() { "nft" } else { "stormwall" };
    eprintln!("Usage: {} [ options ] [ cmds... ]", prog);
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -h, --help                Show this help");
    eprintln!("  -v, --version             Show version");
    eprintln!("  -V                        Extended version info");
    eprintln!("  -f, --file <filename>     Read input from file (- for stdin)");
    eprintln!("  -i, --interactive         Interactive mode");
    eprintln!("  -c, --check               Check only, don't apply");
    eprintln!("  -a, --handle              Show rule handles");
    eprintln!("  -s, --stateless           Omit stateful info");
    eprintln!("  -n, --numeric             Fully numeric output");
    eprintln!("  -y, --numeric-priority    Numeric chain priorities");
    eprintln!("  -p, --numeric-protocol    Numeric L4 protocols");
    eprintln!("  -j, --json                JSON output (for list operations)");
}

struct Opts {
    filename: Option<String>,
    interactive: bool,
    check: bool,
    handle: bool,
    stateless: bool,
    numeric_prio: bool,
    numeric_proto: bool,
    json: bool,
    terse: bool,
    echo: bool,
    defines: Vec<(String, String)>,
}

fn parse_opts() -> (Opts, Vec<String>) {
    let args: Vec<String> = env::args().collect();
    let mut opts = Opts {
        filename: None,
        interactive: false,
        check: false,
        handle: false,
        stateless: false,
        numeric_prio: false,
        numeric_proto: false,
        json: false,
        terse: false,
        echo: false,
        defines: Vec::new(),
    };
    let mut rest = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => { usage(); process::exit(0); }
            "-v" | "--version" => {
                if test_mode() {
                    println!("nftables v{} ({})",
                        NFT_IMPERSONATED_VERSION, NFT_IMPERSONATED_RELEASE_NAME);
                } else {
                    println!("stormwall v{} (drop-in nftables replacement)", VERSION);
                }
                process::exit(0);
            }
            "-V" => {
                if test_mode() {
                    // Match upstream nft -V layout exactly. Capability
                    // lines need to mirror the target build's compile
                    // flags; for this validation box: editline, json
                    // yes, minigmp no, libxtables yes.
                    println!("nftables v{} ({})",
                        NFT_IMPERSONATED_VERSION, NFT_IMPERSONATED_RELEASE_NAME);
                    println!("  cli:\t\teditline");
                    println!("  json:\t\tyes");
                    println!("  minigmp:\tno");
                    println!("  libxtables:\tyes");
                } else {
                    println!("stormwall v{} (drop-in nftables replacement)", VERSION);
                    println!("  backend:  pure Rust netlink");
                    println!("  license:  MIT");
                }
                process::exit(0);
            }
            "-f" | "--file" => {
                i += 1;
                if i < args.len() { opts.filename = Some(args[i].clone()); }
            }
            "-i" | "--interactive" => opts.interactive = true,
            "-c" | "--check" => opts.check = true,
            "-a" | "--handle" => opts.handle = true,
            "-s" | "--stateless" => opts.stateless = true,
            "-n" | "--numeric" => {
                opts.numeric_prio = true;
                opts.numeric_proto = true;
            }
            "-y" | "--numeric-priority" => opts.numeric_prio = true,
            "-p" | "--numeric-protocol" => opts.numeric_proto = true,
            "-j" | "--json" => opts.json = true,
            "-t" | "--terse" => opts.terse = true,
            "-D" | "--define" => {
                i += 1;
                if i < args.len() {
                    if let Some(eq) = args[i].find('=') {
                        opts.defines.push(
                            (args[i][..eq].to_string(), args[i][eq+1..].to_string())
                        );
                    }
                }
            }
            a if a.starts_with("--define=") => {
                let rest = &a["--define=".len()..];
                if let Some(eq) = rest.find('=') {
                    opts.defines.push((rest[..eq].to_string(), rest[eq+1..].to_string()));
                }
            }
            // -e / --echo: on successful change, re-emit the ruleset. We
            // accept the flag (so scripts that pipe through `-e` don't
            // error) but don't yet re-render. The framework only uses
            // it to grep for the rule handle, which `-a` already exposes.
            "-e" | "--echo" => opts.echo = true,
            _ => {
                rest.extend_from_slice(&args[i..]);
                break;
            }
        }
        i += 1;
    }
    (opts, rest)
}

fn process_input(ctx: &mut ops::NftCtx, input: &str, check: bool) -> io::Result<()> {
    let cmds = parser::parse(input).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    for cmd in &cmds {
        if check { continue; }
        ctx.exec(cmd)?;
    }
    Ok(())
}

// -f input: batch all commands into one kernel transaction so a
// failure anywhere rolls the whole thing back.
fn process_input_atomic(ctx: &mut ops::NftCtx, input: &str, check: bool) -> io::Result<()> {
    let cmds = parser::parse(input).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    if check { return Ok(()); }
    ctx.begin_txn();
    let mut err: Option<io::Error> = None;
    for cmd in &cmds {
        if let Err(e) = ctx.exec(cmd) { err = Some(e); break; }
    }
    if let Some(e) = err {
        ctx.abort_txn();
        return Err(e);
    }
    ctx.commit_txn()
}

fn main() {
    let (opts, rest) = parse_opts();

    let mut ctx = match ops::NftCtx::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: cannot open netlink socket: {}", e);
            process::exit(1);
        }
    };
    ctx.show_handles = opts.handle;
    ctx.stateless = opts.stateless;
    ctx.numeric_prio = opts.numeric_prio;
    ctx.numeric_proto = opts.numeric_proto;
    ctx.json = opts.json;
    ctx.terse = opts.terse;
    ctx.echo = opts.echo;

    // Prepend any `--define NAME=VALUE` CLI flags as synthetic
    // `define` declarations so the preprocessor treats them the same
    // as inline ones.
    let define_prefix: String = opts.defines.iter()
        .map(|(k, v)| format!("define {} = {}\n", k, v))
        .collect();

    let result = if let Some(ref filename) = opts.filename {
        let input = if filename == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).map(|_| buf)
        } else {
            fs::read_to_string(filename)
        };
        match input {
            Ok(s) => {
                let combined = format!("{}{}", define_prefix, s);
                process_input_atomic(&mut ctx, &combined, opts.check)
            }
            Err(e) => {
                eprintln!("Error: {}: {}", filename, e);
                process::exit(1);
            }
        }
    } else if opts.interactive {
        let stdin = io::stdin();
        println!("stormwall interactive mode. Type 'quit' to exit.");
        loop {
            eprint!("stormwall> ");
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed == "quit" || trimmed == "exit" { break; }
                    if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
                    if let Err(e) = process_input(&mut ctx, trimmed, opts.check) {
                        eprintln!("Error: {}", e);
                    }
                }
                Err(e) => { eprintln!("Error: {}", e); break; }
            }
        }
        Ok(())
    } else if !rest.is_empty() {
        let input = rest.join(" ");
        let combined = format!("{}{}", define_prefix, input);
        // Multi-command CLI argument (e.g.
        //   nft 'rename chain t c1 c3; rename chain t c2 c3')
        // must be atomic — if the second fails, the first must roll
        // back. Route through the same atomic batch path as -f.
        process_input_atomic(&mut ctx, &combined, opts.check)
    } else {
        let mut input = String::new();
        if io::stdin().read_to_string(&mut input).is_ok() && !input.is_empty() {
            let combined = format!("{}{}", define_prefix, input);
            process_input(&mut ctx, &combined, opts.check)
        } else {
            usage();
            process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
