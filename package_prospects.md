# jonerix package prospects

A working list of permissively-licensed tools that could land in jonerix
with relatively low effort, tiered by build complexity. Each entry's
license has been pre-vetted against [the licensing rule](README.md#licensing-rule);
nothing here is GPL/LGPL/AGPL/SSPL/MPL or otherwise copyleft.

## Tier 1 — pure Rust/Go single-binary, recipe is ~30 lines

These build with the existing toolchain (`rust` or `go`), no new build
deps, no library headaches. Each is recipe-equivalent in shape to
`ripgrep` or `uutils`.

| Package | License | What | Why it's easy |
|---|---|---|---|
| **`fd`** | MIT/Apache-2.0 | `find` replacement (sane defaults, regex, gitignore-aware) | Pure Rust, vendored cargo build, ~1.4 MB. |
| **`bat`** | MIT/Apache-2.0 | `cat` with syntax highlighting + paging | Pure Rust. Pairs naturally with `pico` for terminal users. |
| **`eza`** | MIT | Modern `ls` with git status, tree mode, colors | Pure Rust. Successor to `exa`. Drop-in. |
| **`fzf`** | MIT | Fuzzy finder for any line-oriented input | Pure Go single binary. Massive multiplier in shell workflows; integrates trivially with mksh/zsh. |
| **`zoxide`** | MIT | Smarter `cd` (frecency-ranked) | Pure Rust. Drops `z` shim into the shell. |
| **`delta`** | MIT | git-diff viewer with side-by-side, syntax highlighting | Rust. Pairs with our gitredoxide for a noticeable quality bump. |
| **`hyperfine`** | MIT/Apache-2.0 | Benchmarking tool | Rust. CI-friendly. |
| **`tokei`** | MIT/Apache-2.0 | Lines-of-code counter | Rust. Useful for jpkg recipe accounting. |
| **`just`** | CC0 | Task runner (Makefile-esque but sane) | Rust. Sits next to `jmake`/`samurai` as a build-flow helper. |
| **`watchexec`** | MIT/Apache-2.0 | File-change watcher | Rust. Useful for dev loops. |

Pick any 3–4 from this list and you've meaningfully improved daily
workflow with maybe an hour per recipe.

## Tier 2 — small Go/Rust with light system integration

| Package | License | What | Caveat |
|---|---|---|---|
| **`gh`** | MIT | GitHub CLI | Pure Go. CI-relevant since you're publishing releases there constantly. ~25 MB binary (Go's static linking, but everything else jonerix Go is similar). |
| **`lazygit`** | MIT | Terminal UI for git | Go. Real value if you ever drive gitredoxide from a TUI. |
| **`age`** | BSD-3 | Modern file encryption (Curve25519, ChaCha20) | Go. Fills a real gap — jonerix has no `gpg` and shouldn't, but file encryption is sometimes wanted. |
| **`rage`** | MIT/Apache-2.0 | Rust port of `age`, same wire format | Pick this OR `age`, not both. |
| **`iperf3`** | BSD-3 | Bandwidth testing | C, jmake-friendly. Useful on Pi 5 + router scenarios. |
| **`valkey`** | BSD-3 | Redis fork after Redis went SSPL | C, jmake. Real KV store with a clean license. Larger surface — modules, RDB, AOF, replication — but none of those are blockers. |
| **`prometheus-node-exporter`** | Apache-2.0 | Host metrics for Prometheus scraping | Go. If you ever set up monitoring across the deployments, this is the canonical agent. |
| **`caddy`** | Apache-2.0 | HTTP server with automatic Let's Encrypt | Go. Alternative to the `nginx` you already ship for use cases where ACME-by-default is the win. |

## Tier 3 — useful but bigger

| Package | License | Notes |
|---|---|---|
| **`nushell`** | MIT | Structured-data shell. Coexists with mksh/zsh as a third option. Rust, big dep tree, but vendored. |
| **`kakoune`** | Public Domain | Modal editor in the vim/neovim space, but PD-licensed. Slight build complexity (C++17, ncurses). |
| **`tinyssh`** | Public Domain | Minimal SSH server alternative to dropbear. Useful if you ever want to ship even smaller. |
| **`darkhttpd`** | ISC | 1-file static HTTP server. Good for embedded scenarios where nginx is overkill. |
| **`tmate`** | MIT | tmux fork that pairs sessions over SSH. C build but small. |

## Off-limits — looks easy, license-blocked

These come up often as "why isn't X in jonerix?" — the answer is always license:

| Tool | License | Why not |
|---|---|---|
| **redis** | SSPL-1.0 (post-2024) | Server-side public license = copyleft via SaaS clause. Use `valkey`. |
| **mtr** | GPL-2 | Use BSD `traceroute` (toybox) or write a Rust one. |
| **plocate** / **mlocate** | GPL-2 | No good permissive replacement; `fd` or `ripgrep --files` covers many use cases. |
| **bash** | GPL-3 | We have `brash`. |
| **GNU coreutils** | GPL-3 | We have `uutils` + toybox. |
| **gpg** | GPL-3 | `age` covers symmetric/asymmetric file encryption with simpler UX. |
| **helix** | MPL-2.0 | Weak copyleft, file-level. Same gate as LGPL — denied. |
| **neovim** | Apache-2.0 + Vim License | The Apache-2.0 portion is fine; the Vim License has copyleft-adjacent terms (clause 2 patches). Currently a debate — review needed before packaging. |

## Recommended starting set

If only doing three:

1. **`fzf`** — biggest workflow multiplier per byte of binary, integrates with every shell instantly.
2. **`age`** (or `rage`) — fills a real cryptography gap with a modern, simple tool.
3. **`delta`** — meaningfully improves the gitredoxide experience, which is the dev story we've been investing in heavily.

If you want a fourth: **`fd`** — the discoverability + speed jump over `find` is real and the recipe is trivial.
