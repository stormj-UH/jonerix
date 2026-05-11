# Cross-compile design — jonerix x86_64 ↔ aarch64

Status: **planning** — not yet implemented.

## Motivation

Today every jonerix package recipe is built on its target arch. That
forces:

- aarch64 jpkgs come from a Mac/cloud arm64 runner.
- x86_64 jpkgs come from CI (often wedged) or `castle.great-morpho.ts.net`
  (Ryzen 5 1600, 15 GB RAM — slow and resource-tight).
- An outage on either side stalls the chain for that arch entirely.

`rustc` is the lone exception: `rust-dist.yml` already bakes
`aarch64-jonerix-linux-musl` *and* `x86_64-jonerix-linux-musl` into every
shipped rust toolchain (`rustc --print target-list` shows both). C and
C++ have no equivalent — the LLVM family is built single-target.

The goal of this document: make the LLVM family cross-capable so either
host arch can produce jpkgs for either target arch.

## Non-goals

- No support for any third arch (riscv, ppc64, mips). Two is the cap.
- No "fat" multi-arch jpkgs — each jpkg ships one architecture. This is
  about *production*, not packaging.
- No support for building Linux kernels for the other arch. Kernel
  cross-build is a separate, well-trodden problem and is not on the
  hot path.
- No sanitizer support (asan/ubsan/tsan) cross-built. Sanitizers stay
  native-target only.

## Current state inventory

Verified against the freshly-built `ghcr.io/stormj-uh/jonerix:builder-arm64`
on 2026-05-11:

| Component | Current | Cross-ready? |
|---|---|---|
| `clang -print-targets` | only AArch64 family (aarch64, aarch64_32, aarch64_be, arm64, arm64_32) | ❌ |
| `llvm-config --targets-built` | `AArch64` | ❌ |
| `/lib/clang/21/lib/linux/libclang_rt.builtins-*.a` | aarch64 only | ❌ |
| `/lib/lib{c,c++,c++abi,unwind}.so*` | host arch only | ❌ no x86_64 sysroot |
| `/lib/libc.so` | aarch64 ELF | ❌ |
| `jonerix-headers` | already per-arch jpkg | ✅ (just need both installed) |
| `rustc --print target-list` | both jonerix triples baked in | ✅ |
| `lld` | single binary handles every backend libllvm exposes | falls out of libllvm fix |
| `llvm-ar`, `llvm-nm`, `llvm-strip`, `llvm-objcopy`, `llvm-readelf`, `llvm-objdump` | arch-neutral | ✅ |

## Design

### Phase 1 — multi-target LLVM family

Affected recipes: `libllvm`, `clang`, `lld`, `llvm-extra`.

Single edit each:

```diff
- -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGET" \
+ -DLLVM_TARGETS_TO_BUILD="X86;AArch64" \
```

Leave the existing per-arch `case` alone — it still sets
`LLVM_DEFAULT_TARGET_TRIPLE` correctly per host so calls without
`--target=` still pick the host arch. The only thing that changes is
that the *other* backend is also linked into `libLLVM-21.so`.

`lld` and `llvm-extra` change for the same reason: lld's ELF backend
needs to know about both architectures' relocation handlers, and
llvm-extra's sanitizers + lldb want both for symbolication / disasm.

**Build-cost impact:** libllvm gets roughly 30% larger and 20–30%
slower. We pay this once per LLVM split rebuild. sccache amortises the
delta cleanly across clang/lld/llvm-extra in the same chain since the
LLVM-side compiles cache-hit.

**Bootstrap concern:** the first time this lands, the *currently*
single-target clang in the builder image is what compiles the new
two-target libllvm. That's fine — clang's host backend builds the new
LLVM source unchanged. Subsequent rebuilds use the two-target clang.

### Phase 2 — per-arch sysroots

Multi-target clang produces correct code for either arch, but it needs
target-specific runtime libraries to link against. We ship them as new
"cross" jpkgs that lay the existing arch jpkgs under a deterministic
sysroot prefix:

```
/cross/x86_64-jonerix-linux-musl/lib/{libc.so,libc++.so,libunwind.so,...}
/cross/x86_64-jonerix-linux-musl/include/{stdio.h,bits/...,...}
/cross/x86_64-jonerix-linux-musl/lib/Scrt1.o, crti.o, crtn.o
/cross/aarch64-jonerix-linux-musl/lib/...
/cross/aarch64-jonerix-linux-musl/include/...
```

New jpkgs (each is a thin repacker — extracts the corresponding
existing jpkg's payload under `/cross/<triple>/`):

- `musl-cross-x86_64` (depends on `musl-1.2.6-x86_64.jpkg` content)
- `libcxx-cross-x86_64` (depends on `libcxx-21.1.2-r1-x86_64.jpkg` content)
- `jonerix-headers-cross-x86_64`
- `compiler-rt-builtins-cross-x86_64` (new; see below)
- … and the mirror set for aarch64.

Each "cross" jpkg's recipe:

```toml
[package]
name = "musl-cross-x86_64"
version = "1.2.6"
license = "MIT"
description = "musl libc x86_64 sysroot for cross-compilation"

[source]
url = "local"

[build]
system = "custom"
build = """
set -e
# Pull the actual jpkg content from /var/cache/jpkg-published.
JPKG=$(ls /var/cache/jpkg-published/musl-1.2.6-x86_64.jpkg 2>/dev/null | head -1)
[ -f "$JPKG" ] || { echo "missing host musl jpkg" >&2; exit 1; }
# Extract jpkg payload (12-byte hdr + meta length).
hdr_len=$(od -An -v -tu4 -N4 -j8 "$JPKG" | tr -d ' ')
skip=$((12 + hdr_len))
mkdir -p "$DESTDIR/cross/x86_64-jonerix-linux-musl"
tail -c +$((skip + 1)) "$JPKG" |
    zstd -dc |
    tar xf - -C "$DESTDIR/cross/x86_64-jonerix-linux-musl"
"""

[depends]
build = []
runtime = []
```

Sysroot size is ~30 MB per arch — comfortably fits regular git, no LFS.

#### compiler-rt-builtins cross-build

Standard upstream pattern is `LLVM_BUILTIN_TARGETS="x86_64-...;aarch64-..."`
in the compiler-rt cmake call. Today the llvm-extra recipe builds
builtins for the host triple only:

```sh
cmake -S compiler-rt -B build-rt \
  -DCOMPILER_RT_BUILD_BUILTINS=ON \
  -DCOMPILER_RT_BUILD_SANITIZERS=ON \
  ...
```

Change pattern: split out a new `compiler-rt-builtins-cross-<arch>` jpkg
that builds *only* builtins (no sanitizers) for the cross-target. Drops
into `/lib/clang/21/lib/<triple>/libclang_rt.builtins.a` where clang
looks by default for `--target=<triple> --rtlib=compiler-rt`.

### Phase 3 — clang config files

Drop two files into `/etc/clang/`:

```
# /etc/clang/x86_64-jonerix-linux-musl.cfg
--sysroot=/cross/x86_64-jonerix-linux-musl
-resource-dir=/lib/clang/21
--unwindlib=libunwind
--rtlib=compiler-rt
-stdlib=libc++

# /etc/clang/aarch64-jonerix-linux-musl.cfg
--sysroot=/cross/aarch64-jonerix-linux-musl
-resource-dir=/lib/clang/21
--unwindlib=libunwind
--rtlib=compiler-rt
-stdlib=libc++
```

Clang auto-loads `<argv0>.cfg` and `<target-triple>.cfg` from the same
directory as the binary, *and* from `/etc/clang/`. Adding the config
files is the trigger — no clang code change.

Verification:

```sh
echo 'int main(void){return 0;}' > /tmp/h.c
clang --target=x86_64-jonerix-linux-musl -o /tmp/h /tmp/h.c
file /tmp/h  # should report ELF x86-64
```

### Phase 4 — recipe cross-compile awareness (optional)

Today's recipes branch on `$(uname -m)`. To opt in to cross-build,
add a `TARGET_ARCH` env var the build harness can set:

```sh
ARCH=${TARGET_ARCH:-$(uname -m)}
case $ARCH in
    x86_64)  TRIPLE=x86_64-jonerix-linux-musl  ;;
    aarch64) TRIPLE=aarch64-jonerix-linux-musl ;;
esac
export CC="clang --target=$TRIPLE"
export CXX="clang++ --target=$TRIPLE"
```

This is *opt-in per recipe*. Existing recipes stay native. Phase 4 is
not on the critical path — Phases 1-3 enable cross-compile; Phase 4
makes recipes use it conveniently.

## File-system layout (target end state)

```
/bin/clang, clang++, ld.lld, llvm-ar, llvm-nm, llvm-strip, ...   (multi-target)
/lib/libLLVM-21.so                                                (X86;AArch64)

/lib/clang/21/lib/aarch64-jonerix-linux-musl/libclang_rt.builtins.a
/lib/clang/21/lib/x86_64-jonerix-linux-musl/libclang_rt.builtins.a
/lib/clang/21/lib/aarch64-jonerix-linux-musl/libclang_rt.asan.a   (host arch only)
/lib/clang/21/lib/aarch64-jonerix-linux-musl/libclang_rt.ubsan_*  (host arch only)

/cross/x86_64-jonerix-linux-musl/lib/{libc,libc++,libc++abi,libunwind}.so*
/cross/x86_64-jonerix-linux-musl/lib/{Scrt1,crti,crtn}.o
/cross/x86_64-jonerix-linux-musl/include/{stdio.h,bits/...,...}
/cross/aarch64-jonerix-linux-musl/...                              (mirror)

/etc/clang/x86_64-jonerix-linux-musl.cfg
/etc/clang/aarch64-jonerix-linux-musl.cfg
```

## Rollout plan

Strict ordering to avoid bootstrap deadlocks:

1. **Phase 1 patch** — edit four recipes (libllvm, clang, lld,
   llvm-extra). Ship in a normal chain rebuild on either arch. After
   landing, every fresh `clang` ships with both backends linked in.
2. **Verify Phase 1** — `clang -print-targets` shows both arches,
   `clang --target=<other-arch>-jonerix-linux-musl -c hello.c` produces
   a correct object file (linking will fail — no sysroot yet).
3. **Phase 2 cross-jpkgs** — land 8 thin repacker recipes:
   - `musl-cross-{x86_64,aarch64}`
   - `libcxx-cross-{x86_64,aarch64}`
   - `jonerix-headers-cross-{x86_64,aarch64}`
   - `compiler-rt-builtins-cross-{x86_64,aarch64}` (this one is a real
     cmake build, not a repacker)
4. **Verify Phase 2** — `clang --sysroot=/cross/x86_64-jonerix-linux-musl
   --target=x86_64-jonerix-linux-musl -o hello hello.c` produces a
   linkable ELF binary; `file hello` reports x86_64.
5. **Phase 3** — drop the two clang config files (small recipe, or fold
   into the existing `clang` recipe's `install` section).
6. **Verify Phase 3** — `clang --target=x86_64-jonerix-linux-musl -o
   hello hello.c` (no `--sysroot` arg) works because the config file
   carries it.
7. **Smoke test the whole stack** — pick a real jpkg recipe (e.g.
   `jmake`) and rebuild it cross-target from the other host. Compare
   binary output against the native-built jpkg.

## Risks and open questions

- **libllvm-21.so ABI:** does the shared lib stay ABI-compatible when a
  second backend is linked in? Expected: yes (LLVM's public C API is
  arch-independent; backends are loaded as part of the dylib but only
  used via the same TargetMachine surface). Worth verifying by linking
  one of the rustc-built test binaries against the two-backend dylib.
- **clang config file precedence:** `--target=` on the command line
  loads the per-target `.cfg`. Need to confirm that recipe-level
  `CMAKE_C_FLAGS="-target ..."` triggers the same loader path. Quick
  test: `clang -v --target=x86_64-jonerix-linux-musl 2>&1 | grep
  configuration`.
- **musl sysroot symlinks:** musl's headers use `/lib/ld-musl-*.so.1`
  as the dynamic linker name. When we lay an x86_64 musl under
  `/cross/x86_64-.../lib/`, the dynamic linker needs to be reachable.
  jonerix's merged-usr layout keeps things simple; the cross-sysroot is
  only for *link-time*, the produced binaries still run on a native
  x86_64 system where `/lib/ld-musl-x86_64.so.1` exists.
- **rust cross-build cargo deps:** rust already supports both targets,
  but real-world cargo packages sometimes have build.rs scripts that
  shell out to the *host* compiler. Each such recipe needs to plumb
  `CC_<target_triple>` / `CXX_<target_triple>` env vars to point at the
  multi-target clang with explicit `--target=`. Handle per recipe as
  needed; not a Phase 1-3 concern.
- **Smoke-test gating:** the CI `Smoke test (amd64)` / `Smoke test
  (arm64)` jobs already exercise the builder image. Add a third job
  that cross-builds a tiny recipe (e.g. `bc`) from the *other* arch's
  builder image and verifies the output runs under qemu-user.

## Alternatives considered and rejected

| Alternative | Why rejected |
|---|---|
| Stay single-target (status quo) | Forever-dependent on castle for x86_64 or on a cloud arm64 runner for aarch64. CI outage = chain dead. |
| QEMU-user multi-arch container build | 5–20× slowdown on LLVM compiles. Opaque to local dev. Doesn't help anyone who isn't running CI. |
| Cross-LLVM only, no sysroot package | Users have to assemble their own sysroot manually. Fragile, every recipe re-invents `--sysroot=` pointer. |
| `gcc-aarch64-linux-gnu` style separate cross-binary | Defeats the point of LLVM being one binary that handles every target it was built with. |
| Full Yocto/Buildroot-style sysroot manifest | Over-engineered for two arches with one libc. |

## Estimated effort

| Phase | Effort | Touchpoints |
|---|---|---|
| 1 — multi-target recipes | ~30 min | 4 recipe edits, 1 chain rebuild to verify |
| 2 — cross sysroot jpkgs | ~3 hours | 8 new recipe files (6 are thin repackers), 1 cross-rebuild of compiler-rt builtins |
| 3 — clang configs | ~15 min | 2 small files dropped into `clang` recipe |
| 4 — recipe TARGET_ARCH awareness | rolling | per-recipe, opt-in over time |

Total for Phases 1-3 (the gate to ditching castle for x86_64 builds):
**roughly one focused half-day plus one chain rebuild's wall time.**

## When to do this

Not now — castle is mid-chain producing x86_64 LLVM-family jpkgs.
Interrupting that loses ~2h of wall time. The natural moment is the
*next* time the LLVM family needs a rebuild (kernel header bump, new
LLVM minor version, etc.), so the multi-target switch piggy-backs on
work that has to happen anyway.
