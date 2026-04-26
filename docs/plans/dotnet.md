# Plan: .NET 10 SDK + runtime, source-built against libc++

**Status:** WIP. The first attempt (repackage Microsoft's `linux-musl-arm64`
SDK as a jpkg) shipped in commit 04566f3 and was yanked the same day —
Microsoft's musl tarballs `DT_NEEDED` GNU `libstdc++.so.6` and
`libgcc_s.so.1`, which violates the zero-GPL runtime policy. This plan
describes the from-source path that does not.

---

## 1. Why the binary repackage failed

`packages/extra/dotnet/recipe.toml@04566f3` extracted Microsoft's official
`dotnet-sdk-10.0.203-linux-musl-{x64,arm64}.tar.gz` into `/lib/dotnet`,
patched `/lib64` references, and shipped it under MIT. The MIT license is
fine — the binary is not.

```
$ readelf -d /lib/dotnet/dotnet | grep NEEDED
 NEEDED  Shared library: [libstdc++.so.6]   # GNU, GCC Runtime Library Exception
 NEEDED  Shared library: [libgcc_s.so.1]    # GNU, GCC Runtime Library Exception
 NEEDED  Shared library: [libc.musl-aarch64.so.1]
```

Microsoft's "musl" build is built on Alpine, which ships GNU `libstdc++`
(via `apk add libstdc++`) even on a musl base. jonerix ships LLVM
`libcxx` and has no `libstdc++` package by policy. On a real jonerix
host, every C++ symbol fails to resolve at load time:

```
Error relocating /lib/dotnet/dotnet: _ZNSt7__cxx1112basic_string...: symbol not found
```

The `_ZNSt7__cxx11...` mangling is GCC's dual-ABI namespace, which
`libcxx` does not provide.

Adding a `gcc-libs` runtime jpkg would unblock the binary trivially but
expands the GPL exception from "Linux kernel only" to "Linux kernel +
all of GCC's runtime." Out of scope unless explicitly approved.

## 2. Target

- `dotnet` jpkg, MIT-licensed end-to-end, built from source against
  `clang` + `lld` + `libcxx` + `musl`, no `libstdc++` / `libgcc_s`
  references in any shipped ELF.
- SDK + runtime in one jpkg (matches the repackage layout).
- Both `linux-musl-x64` and `linux-musl-arm64`.

## 3. Upstream surface

### 3.1 dotnet/dotnet (the VMR)

Microsoft consolidated source-build inputs into a single mono-repo at
[dotnet/dotnet](https://github.com/dotnet/dotnet). Tag `v10.0.203`
contains the exact sources that produced SDK 10.0.203. This is the only
sane entry point — the per-component repos (dotnet/runtime,
dotnet/sdk, dotnet/roslyn, ...) are pulled in via git subtrees and
should not be built individually.

Tarball: `https://github.com/dotnet/dotnet/archive/refs/tags/v10.0.203.tar.gz`
(checked-out size ≈ 5–7 GB; tarball ≈ 1 GB).

### 3.2 source-build prerequisites

`./build.sh -sb` requires a "Previously Source-Built" tarball — a
tarball of the prior version's source-build output, used to bootstrap
the current version. For 10.0.203 this is the 10.0.20x predecessor
output, published at:

`https://builds.dotnet.microsoft.com/dotnet/internal/release-metadata/source-build-prerequisites/<sha>/Private.SourceBuilt.Prereqs.<ver>.tar.gz`

Red Hat / Fedora maintain these prerequisite tarballs publicly; we
should pull from the same source they do (see Fedora `dotnet10.0.spec`).

This is the chicken-and-egg of source-build: bootstrapping from absolute
zero means walking back to the first source-buildable .NET, which is
.NET 6 (`source-build` became official in .NET 6). Reasonable choices:

1. **Trust upstream prereqs** (what every distro does). Pull the
   PreviouslySourceBuilt tarball, audit its license manifest, build
   from there. Cuts the build to ~2 hours.
2. **Walk the chain** (.NET 6 → 7 → 8 → 9 → 10). Each step requires
   the prior step's output. Adds ~10 hours of build for the walk plus
   a separately-managed cache. Provides reproducibility but is not what
   the policy actually asks for — the policy is about the *runtime*
   shipping no GPL, not about bootstrap purity.

Recommendation: option 1. Stash the prereqs tarball alongside our other
vendored sources and treat it like the Go bootstrap binary — a
build-time-only artifact whose license is checked but whose contents
never enter a jpkg.

## 4. The libc++ work

This is the actual engineering. `dotnet/runtime` (the CoreCLR + CoreLib
+ libraries) is ≈ 5 M lines of C++/C# and is currently built against
`libstdc++` on every distro. Building it against `libcxx` requires:

### 4.1 Toolchain plumbing

- `eng/SourceBuild.props` and `eng/native/configurecompiler.cmake`
  unconditionally pass `-stdlib=libstdc++` on Linux. Need to flip to
  `-stdlib=libc++` and make sure cmake's compiler probes pick up
  `libcxx`'s headers at the right paths (jonerix's `libcxx` ships
  headers at `/include/c++/v1`).
- `--rid linux-musl-x64` already exists. Add or override the cmake
  toolchain file to set `CMAKE_CXX_FLAGS=-stdlib=libc++ -nostdinc++
  -isystem/include/c++/v1` and
  `CMAKE_EXE_LINKER_FLAGS=-stdlib=libc++ -lc++abi`.

### 4.2 Source patches likely required

CoreCLR has GCC-isms that compile under clang+libstdc++ but not
clang+libc++. Known bug surface from past porting efforts:

- `<ext/...>` includes from libstdc++'s GNU extensions namespace.
  Replace with libc++ equivalents or `<algorithm>` standard ones.
- Implicit `<string>` / `<sstream>` includes via libstdc++'s transitive
  headers — libc++ is stricter, surface as missing-include compile
  errors.
- `__cxa_demangle` paths differ between libc++abi and libstdc++ —
  CoreCLR uses demangling in stack-trace formatting.
- `std::filesystem` ABI differences — libc++ pre-15 had a filesystem
  TS ABI; jonerix ships libc++ 21 so this is moot, but watch for
  `__fs::filesystem::` namespace explicit references.

Patch budget estimate: 200–800 lines, in `patches/dotnet/`.

### 4.3 Static-link the C++ runtime instead?

CoreCLR ships its own copies of `libcoreclr.so`, `libhostpolicy.so`,
etc. — each one is a small enough .so that `-static-libstdc++
-static-libgcc` would inline GCC's runtime into the .so and remove the
DT_NEEDED. This is what some embedded distros do.

This is *not* a clean answer for jonerix because the inlined code is
still GPL-with-runtime-exception. The Runtime Library Exception
permits combining libstdc++ with non-GPL code, but the policy here is
philosophical, not strictly legal — we don't want GCC in the runtime
even when the license technically allows it. Mention this option for
completeness, do not pursue.

## 5. Build budget

- **Disk:** 80–120 GB during build. The Pi 5's USB SSD probably has
  it; CI runners do not — `ubuntu-24.04-arm` has 14 GB and will OOM
  on disk. Either expand the runner volume, build in stages with
  `--clean-while-building`, or move the .NET build off the standard
  `publish-packages.yml` runner pool.
- **RAM:** 16+ GB recommended. CI runners are 7 GB; expect heavy
  swapping. Build will succeed, slowly.
- **Time:** 90–180 min on a warm runner with prereqs cached, 4–6 h
  cold. Castle (16 GB / 12 CPU) is a better build host than a CI
  runner for iteration.

## 6. Phased rollout

1. **Phase 0 — this commit.** Plan written, broken jpkgs yanked from
   release, recipe replaced with a stub that fails fast with a pointer
   to this doc. Excluded from build-order.
2. **Phase 1 — local proof on castle.** Pull dotnet/dotnet v10.0.203,
   pull prereqs, run `./build.sh -sb` with the stock libstdc++ path
   to confirm the upstream build works on jonerix-builder. Sets a
   baseline before any libc++ work.
3. **Phase 2 — toolchain switch.** Add the libc++ cmake flags. Iterate
   on the resulting compile errors with a patch series in
   `patches/dotnet/`. Goal: `./build.sh -sb` produces an SDK whose
   `readelf -d` shows no `libstdc++` / `libgcc_s` DT_NEEDED entries.
4. **Phase 3 — recipe + CI.** Replace the stub recipe with the real
   build (curl prereqs, untar, apply patches, build, install). Add to
   build-order Tier 11. Trigger `publish-packages.yml`. Iterate until
   green on both arches.
5. **Phase 4 — smoke test.** Reinstall on jonerix-tormenta, build
   fizzbuzz, confirm `dotnet --info` clean. Same hello-world that
   blew up in commit 04566f3, this time it works.

## 7. Open questions

- Is option 1 in §3.2 (trust upstream prereqs tarball) acceptable, or
  should we walk the full .NET 6 → 10 chain? Recommendation: trust
  upstream, but flag for your call.
- libc++abi vs libcxxrt — jonerix's `libcxx` package is paired with
  libc++abi (LLVM). Confirm this is the case before assuming
  `__cxa_demangle` is available.
- Does `publish-packages.yml` need a runner upgrade for dotnet, or do
  we move it to its own workflow with bigger runners?
