# Java Bootstrap Plan

## Overview

This document analyzes whether and how to add a Java runtime to jonerix. The central challenge
is twofold: OpenJDK is licensed under `GPL-2.0 WITH Classpath-exception-2.0`, which requires
a policy decision, and building OpenJDK from source requires a working JDK already present on
the build system — the classic chicken-and-egg bootstrap problem.

The document covers the license analysis, bootstrap chain, musl compatibility, size tradeoffs,
a draft recipe skeleton, and a priority recommendation.

---

## 1. License Analysis

### 1.1 OpenJDK's License: GPL-2.0 WITH Classpath-exception-2.0

OpenJDK (and all downstream builds including Eclipse Temurin, Amazon Corretto, Azul Zulu, and
the reference builds at jdk.java.net) is distributed under:

```
GPL-2.0-only WITH Classpath-exception-2.0
```

These are two separable considerations:

**The GPL-2.0 base**: The JDK source code and the resulting binaries (`java`, `javac`, `jar`,
the JVM itself, the class libraries) are GPL-2.0. If jonerix ships a JDK, those binaries are
GPL-2.0 programs on the running system.

**The Classpath Exception**: The Classpath Exception is a carefully drafted carve-out appended
to the GPL-2.0 license. Its effect is:

> "As a special exception... you may link or combine this library code with independent modules
> to produce an executable... without causing the resulting executable to be covered by the GNU
> General Public License."

In plain terms: Java programs *compiled with* or *running on* the JDK are NOT subject to the
GPL. The exception covers the standard class library linkage. A developer can write a
proprietary (or MIT, Apache-2.0, etc.) application in Java, compile it with OpenJDK javac, and
run it on OpenJDK without any GPL obligations on their application. The GPL-2.0 applies only to
the JDK binaries themselves — not to the programs they compile or host.

### 1.2 Comparison to the Linux Kernel

The Linux kernel is jonerix's established GPL exception. The reasoning for accepting it is:
"No permissive OS kernel with equivalent hardware and container support exists." The kernel is
a single practical monopoly at its layer.

Java's position is structurally different but functionally similar at the JVM layer:

| Factor | Linux Kernel | OpenJDK |
|--------|-------------|---------|
| License | GPL-2.0-only | GPL-2.0 WITH Classpath-exception-2.0 |
| Alternatives | None viable (see DESIGN.md §1) | GraalVM CE (GPL+CPE), Eclipse OpenJ9 (EPL-2.0 — copyleft) |
| Is the GPL viral to user code? | No (syscall interface exception, kernel ABI is not a derived work) | No (Classpath Exception explicitly covers this) |
| Is the GPL viral to the OS userland? | No | No — the JDK is a self-contained runtime |
| Permissive JVM implementations | None production-ready for modern Java bytecode | None viable for Java 21+ |
| Sole exception in DESIGN.md | Yes | Would be second |

The key structural difference: the Linux kernel is a hardware interface and ships as a boot
blob under `/boot`. The JDK is a large userland runtime tree under `/bin` and `/lib/jvm/`. It
is unambiguously "userland," while the kernel is not. Adding it as a second GPL exception is
a more visible policy expansion than the kernel exception.

### 1.3 Permissive JVM Alternatives

No production-ready permissive-licensed JVM exists for modern Java. A survey:

| Project | License | Java Version | Status |
|---------|---------|-------------|--------|
| OpenJDK (Eclipse Temurin, etc.) | GPL-2.0+CPE | 8, 11, 17, 21, 25 | Production-ready |
| GraalVM Community Edition | GPL-2.0+CPE | 21, 25 | Production-ready (same license) |
| Eclipse OpenJ9 | EPL-2.0 + Apache-2.0 | 8, 11, 17, 21 | IBM fork; EPL is copyleft |
| Excelsior JET | Proprietary (discontinued) | 8 | Dead |
| IKVM.NET | MIT | (JVM in .NET CLR) | Not a standalone JVM; .NET dependency |
| Avian JVM | ISC | Java 7 (partial) | Abandoned since 2017; Java 7 only |
| CaffeineVM | MIT | Partial | Research-only; not production-usable |
| Kaffe | GPL-2.0 | Java 1.5 era | Abandoned; GPL anyway |

There is no permissive, production-ready JVM for any Java version newer than approximately 2007.
The ecosystem consolidation around OpenJDK is total.

### 1.4 Recommendation: Treat as a Conditional Exception

The Linux kernel exception was accepted because there is no alternative. The same logic applies
to OpenJDK at the JVM layer, but with one important qualifier: Java is not a *required* system
component, so the exception should be scoped to an opt-in package rather than a base image
component.

**Recommendation**: Accept `GPL-2.0 WITH Classpath-exception-2.0` as a second named exception
in jonerix's license policy, subject to the following constraints:

1. OpenJDK packages are placed in `packages/develop/` — never in `core/` or `extra/`
2. They are not installed in `minimal`, `core`, or `router` images
3. The `builder` image may optionally include a JDK as a build tool
4. A separate `jonerix:java` image can be defined for users who need a Java runtime
5. `jpkg audit` treats `GPL-2.0 WITH Classpath-exception-2.0` as a named exception (not
   permissive, but explicitly allowed in the `develop/` package category)
6. DESIGN.md §1 Known Compromises table is updated to document the exception

The Classpath Exception means that running Java applications on jonerix does not propagate any
GPL obligation to those applications — behavior identical to the kernel's syscall ABI exception.

---

## 2. Bootstrap Chain

### 2.1 The Chicken-and-Egg Problem

OpenJDK is written in a combination of Java and C++. Building OpenJDK from source requires a
pre-existing JDK to compile the Java portions of the JDK itself. The OpenJDK build system
explicitly requires a "boot JDK" that is one major version behind the version being built:

| Target version | Required boot JDK | Notes |
|---------------|------------------|-------|
| OpenJDK 21 (LTS) | JDK 20 or JDK 21 | JDK 20 is non-LTS; use JDK 21 boot |
| OpenJDK 25 (LTS) | JDK 24 or JDK 25 | JDK 24 is non-LTS; use JDK 25 boot |

Unlike Go's C bootstrap (the Go 1.4 compiler is written entirely in C, enabling a pure C
first step) or Rust's rustc-bootstrap, there is no version of OpenJDK written in a language
other than Java at its core. The bootstrap must always start with a prebuilt binary JDK.

### 2.2 Eclipse Temurin: The Bootstrap Provider

Eclipse Temurin is the reference distribution of Eclipse Adoptium — the OpenJDK downstream
that replaced AdoptOpenJDK. It provides prebuilt JDK binaries for many platforms including
`linux-musl-x64` and `linux-musl-aarch64`.

| Property | Details |
|----------|---------|
| Source | https://github.com/adoptium/temurin-build |
| Releases | https://api.adoptium.net/v3/binary/latest/21/ga/linux/x64/jdk/hotspot/normal/eclipse |
| License | GPL-2.0 WITH Classpath-exception-2.0 (same as OpenJDK) |
| musl builds | Available for x64 and aarch64 |
| LTS versions available | 8, 11, 17, 21 (25 when released) |

Temurin musl builds are statically linked against musl and do not require glibc. They run on
Alpine Linux and will run on jonerix without modification.

The Adoptium project maintains a separate musl build infrastructure and publishes official
musl tarballs at:
```
https://github.com/adoptium/temurin21-binaries/releases/
  └── OpenJDK21U-jdk_x64_alpine-linux_hotspot_<date>.tar.gz
  └── OpenJDK21U-jdk_aarch64_alpine-linux_hotspot_<date>.tar.gz
```

The `alpine-linux` suffix means musl. These can be used directly as the bootstrap JDK.

### 2.3 The Full Bootstrap Chain

```
Step 0: Eclipse Temurin 21 prebuilt (musl/alpine binary)
           ↓  (used as boot JDK, never ships in final image)
Step 1: Build OpenJDK 21 from source with clang + musl
           ↓  (produces the actual jonerix OpenJDK package)
Step 2: Produce openjdk-21.jpkg  (ships in final image if desired)
           ↓  (optional: use as boot JDK for building newer LTS)
Step 3: Build OpenJDK 25 from source using openjdk-21 as boot JDK
           ↓
Step 4: Produce openjdk-25.jpkg
```

This mirrors the Go chain pattern used in jonerix today:
```
Go chain:  prebuilt C source (Go 1.4) → 1.17 → 1.20 → 1.22 → 1.24 → 1.26
Java chain: Temurin 21 prebuilt → OpenJDK 21 from source → OpenJDK 25 from source
```

The Temurin prebuilt is a build-time-only dependency. The GPL binary is used to compile the
JDK but is discarded afterward — only the from-source build lands in the package.

### 2.4 OpenJDK Version Target

| Version | Type | Support End | Recommendation |
|---------|------|-------------|----------------|
| OpenJDK 21 | LTS | September 2028 | **Primary target** — widest ecosystem adoption |
| OpenJDK 25 | LTS | September 2030 | **Secondary target** — next LTS (due Sept 2025) |
| OpenJDK 24 | Non-LTS | September 2025 | Skip — short support window |
| OpenJDK 11 | LTS (winding down) | September 2026 | Not worth building; 21 covers it |
| OpenJDK 8 | LTS (Extended) | December 2026 | Skip — ancient; complex build; AWT issues |

**Primary recommendation**: Target OpenJDK 21, the current stable LTS. It is the version most
widely required by server workloads (Spring Boot 3.x, Gradle, Kafka, Elasticsearch, etc.).

**Secondary recommendation**: Add OpenJDK 25 once 21 is working and 25 reaches GA release
(targeted September 2025).

### 2.5 Source Location

```
OpenJDK 21: https://github.com/openjdk/jdk21u
             Tags: jdk-21.0.x+y (e.g., jdk-21.0.7+6)
OpenJDK 25: https://github.com/openjdk/jdk
             (main development; LTS tag when released)
```

OpenJDK 21u is the "update" repository — stabilized LTS updates. It is the correct source
for a distribution shipping OpenJDK 21.

---

## 3. musl Compatibility

### 3.1 Project Portola

Oracle initiated Project Portola within OpenJDK to port the JDK to Alpine Linux (musl libc).
It was upstreamed into mainline OpenJDK and is not a separate fork — musl support is built
into standard OpenJDK releases since JDK 16 (experimental) and is production-grade in
JDK 17+.

Key engineering work done by Portola:
- Replaced glibc-specific `dl_iterate_phdr` calls with musl equivalents
- Rewrote the memory mapping and thread stack detection code for musl's different layout
- Fixed `pthread_getattr_np` differences between glibc and musl
- Adapted the signal handling and `SA_SIGINFO` structures
- Fixed the dynamic linker path (`/lib/ld-musl-x86_64.so.1` vs glibc's `/lib64/ld-linux-x86-64.so.2`)
- JVM JIT (C2 compiler) tested and verified on musl

### 3.2 Current Distro Support

| Distribution | OpenJDK version | musl status | Notes |
|---|---|---|---|
| Alpine Linux | 8, 11, 17, 21, 22 | Stable, in official repos | Alpine's packages are the reference implementation |
| Wolfi (Chainguard) | 11, 17, 21, 22, 23 | Stable | musl-based distro; good reference build scripts |
| Eclipse Temurin | 8, 11, 17, 21 | Official musl binaries available | `alpine-linux` label |
| Amazon Corretto | 8, 11, 17, 21 | Experimental musl builds exist | Not officially supported on musl |
| Azul Zulu | 8, 11, 17, 21 | Community musl builds | Unofficial |

Alpine's OpenJDK packages are the strongest evidence that musl JDK is production-ready. Alpine
has shipped OpenJDK for years; it is used in production by millions of containerized Java apps.

### 3.3 Clang Build Support

OpenJDK's build system (`configure` + GNU make + custom Java build framework) has historically
assumed GCC. The situation as of JDK 21:

- The JDK builds with Clang on macOS (the only Apple toolchain) — this proves the C/C++ code
  is clang-compatible.
- Linux builds with Clang are supported as of JDK 21 (`--with-toolchain-type=clang`).
- Alpine's OpenJDK packages use GCC (via Alpine's apk infrastructure). Chainguard's Wolfi uses GCC as well.
- The `hotspot` JVM source code has conditional compilation guards for GCC and Clang.

**Known issue**: The JDK build uses GNU make extensively. This is a build-time dependency only
(in Alpine's build container, acceptable under jonerix policy), but it means the build itself
requires GNU make even though the resulting JDK binary is pure clang/musl output.

**Recommendation**: Use Clang as the compiler (`CC=clang CXX=clang++`) with GNU make as a
build-time-only tool (already acceptable in jonerix's Alpine container builds).

### 3.4 Notable musl-Specific Caveats

| Issue | Status | Notes |
|-------|--------|-------|
| Thread-local storage (TLS) differences | Fixed in JDK 16+ | glibc and musl TLS layout differs; Portola fixed this |
| Stack size defaults | Fixed | musl default stack is smaller; OpenJDK tuned for this |
| `SIGPROF` profiling | Mostly works | Some JVM profiling tools have edge cases on musl |
| `backtrace()` / `libunwind` | Works | OpenJDK uses its own unwinder; no libc backtrace dependency |
| Time zone data | Ships in JRE | OpenJDK bundles its own `tzdata` — no system tzdata dep |
| Font rendering (AWT/Swing) | Requires libfreetype, fontconfig | Only needed for GUI; server JRE avoids this |
| DNS resolution | Uses its own resolver | OpenJDK has a built-in DNS stack; no nsswitch dependency |

For a headless server JDK (no AWT/Swing/JavaFX), the musl story is clean. GUI components
add significant native library dependencies.

---

## 4. Size and Complexity

### 4.1 Size Breakdown

A full OpenJDK 21 installation (HotSpot JDK, stripped) on musl x86_64:

| Component | Size (approx) | Notes |
|-----------|---------------|-------|
| `java` launcher + JVM libs | ~120 MB | `libjvm.so`, `libserver.so`, `libjimage.so` |
| Class library (`modules` image) | ~110 MB | All standard library classes; JIMAGE format |
| `javac` compiler + tools | ~15 MB | `javac`, `jar`, `javap`, `jdeps`, `jlink` |
| Debug symbols (stripped) | ~5 MB | With `--strip-debug` during build |
| Headers | ~2 MB | JNI headers, internal headers |
| **Full JDK total** | **~250 MB** | Full development kit |
| JRE subset (`jlink --add-modules java.base`) | ~30-50 MB | Minimal runtime for simple apps |
| JRE + web subset (base+sql+net+xml) | ~80-120 MB | Adequate for most server workloads |

For comparison:
- Full LLVM/Clang toolchain: ~350 MB
- Go toolchain: ~120 MB
- Rust toolchain: ~600 MB
- Node.js: ~60 MB

The JDK is large but not uniquely so relative to other developer toolchains.

### 4.2 Packaging Options

Three packaging strategies are viable:

**Option A: Package the full JDK (~250 MB)**

Pros:
- Single package; works for all Java workloads (development + runtime)
- Users can use `jlink` to create their own trimmed custom runtimes
- Simplest to build and maintain

Cons:
- 250 MB is a large package; suitable for `builder` image only
- Not appropriate for minimal containers running Java apps

**Option B: Two packages — JDK (dev) + JRE (runtime) (~250 MB + ~120 MB)**

Pros:
- Separation of concerns: servers install JRE, developers install JDK
- JRE is a subset of JDK; built from the same sources using `jlink`
- Alpine's approach; well-understood

Cons:
- Two recipes to maintain (or one recipe that produces two output packages)
- JRE is still ~120 MB; not appropriate for minimal Java containers

**Option C: Base JRE via `jlink` + optional modules (~50 MB base)**

Pros:
- `jlink` can produce a custom JRE containing only the modules a specific application needs
- A base `java.base`-only JRE is ~30-50 MB — competitive with Node.js
- This is the modern cloud-native Java approach (used by GraalVM native image users)

Cons:
- Requires knowing which modules an application needs at package time
- Not a general-purpose JRE; application-specific

**Recommended approach**: Start with **Option A** (full JDK) for the initial build. Once the
build is proven, add **Option B** (split JDK/JRE) as a second package generated from the same
recipe. Option C (application-specific jlink runtimes) is a user workflow, not a jonerix
package distribution decision.

### 4.3 Build Complexity

Building OpenJDK from source is significantly more complex than any current jonerix recipe:

| Factor | Detail |
|--------|--------|
| Build system | `configure` (autoconf-style) + GNU make + Java build system |
| Build time | ~45-90 minutes on Castle (12 CPUs, 16 GB RAM) |
| Build deps | clang, lld, GNU make, bash, cups-dev (optional), libx11-dev (optional) |
| Boot JDK required | Yes — Temurin 21 prebuilt (~250 MB download) |
| Headless build possible | Yes — `--with-x=no --with-cups=no --with-fontconfig=no` |
| Output validation | `make test-tier1` runs basic JVM smoke tests; recommended |

GNU make is required at build time. This is acceptable under jonerix policy (same as Ruby,
hostapd, wpa_supplicant — built in Alpine containers where GNU make is available).

The recipe will be one of the most complex in the project but follows a known pattern:
download boot JDK, configure build, compile, produce JDK tree, strip, package.

---

## 5. Draft Recipe

The recipe file is at `packages/develop/openjdk/recipe.toml`. It is marked not-yet-buildable
pending full testing of the configure/build flags on musl with Clang.

Key aspects of the build:
- `--with-toolchain-type=clang` — use Clang instead of GCC
- `--with-extra-cflags` / `--with-extra-cxxflags` — apply jonerix hardening flags
- `--with-x=no --with-cups=no --with-fontconfig=no` — headless server build
- `--disable-warnings-as-errors` — Clang produces different warnings than GCC; some
  upstream files trigger `-Werror` failures with Clang; this flag avoids blocking the build
- `--with-boot-jdk` — path to the Temurin 21 prebuilt bootstrap JDK

The Temurin bootstrap JDK is downloaded and verified at build time, used only for compilation,
and is NOT installed into `$DESTDIR` — only the freshly compiled JDK is packaged.

See `packages/develop/openjdk/recipe.toml` for the full draft.

---

## 6. Known Gaps and Open Questions

### 6.1 Build System Requirement: GNU make

The OpenJDK build requires GNU make. It is already accepted as a build-time tool in Alpine
containers. No policy change needed, but it should be documented explicitly in the recipe.

### 6.2 jpkg License Allowlist

The SPDX expression `GPL-2.0-only WITH Classpath-exception-2.0` is not currently in jpkg's
allowlist. To allow this package to build and install, `cmd_build.c` and `cmd_install.c` need
an entry for this expression. The parsing is:
- `GPL-2.0-only` is not permissive
- `WITH Classpath-exception-2.0` modifies it
- The combined expression should be added as a named acceptable identifier for the `develop/`
  package category only (not `core/` or `extra/`)

This requires a small code change to jpkg.

### 6.3 `--disable-warnings-as-errors` and Clang Version

OpenJDK's HotSpot source code has GCC-specific warning suppressions. With Clang, new warnings
may fire that are treated as errors by the build. The `--disable-warnings-as-errors` flag
is a blunt instrument. Alternatively, specific `-Wno-X` flags can be passed via
`--with-extra-cflags`. Testing on the actual build will reveal which warnings fire.

### 6.4 GCC CRT Files Not Needed

Unlike LLVM itself (which needs GCC CRT files like `crtbeginS.o` per `docs/JONERIX-BUILD-ENVIRONMENT.md`),
OpenJDK links as a normal userland application. With `--with-toolchain-type=clang` and
`LD=ld.lld`, the standard musl CRT files (`crt1.o`, `crti.o`, `crtn.o`) are sufficient.

### 6.5 The `modules` Image Format

OpenJDK 9+ uses a custom JIMAGE format for the class library (`lib/modules`). This is a
self-contained binary format read by the JVM at startup. It ships as a single file (~110 MB)
rather than individual `.jar` files. The `jimage` tool (part of the JDK) can inspect it.
No special jonerix packaging work is needed — it is just a large file in the package.

### 6.6 Java's `JAVA_HOME` Convention

Java expects to find its installation at `$JAVA_HOME`. Under jonerix's merged-usr layout,
the JDK would install to `/lib/jvm/openjdk-21/` with:
- `/bin/java -> /lib/jvm/openjdk-21/bin/java` (symlink)
- `/bin/javac -> /lib/jvm/openjdk-21/bin/javac` (symlink)
- `JAVA_HOME=/lib/jvm/openjdk-21` in `/etc/profile.d/java.sh`

This is the same approach used by Alpine and Wolfi.

### 6.7 Architecture: x86_64 First

The HotSpot JIT (C2 compiler) is well-tested on x86_64. aarch64 HotSpot is also mature —
Temurin ships official aarch64 musl builds. Build aarch64 second, once x86_64 is working.

---

## 7. License Concern Summary

| Component | License | Verdict |
|-----------|---------|---------|
| OpenJDK source + binaries | GPL-2.0 WITH Classpath-exception-2.0 | **Conditional exception** — see §1.4 |
| Eclipse Temurin bootstrap JDK | GPL-2.0 WITH Classpath-exception-2.0 | Build-time only; never ships |
| Java applications compiled/run on it | User's choice | No GPL obligation (Classpath Exception) |
| OpenJDK build system (GNU make) | GPL-2.0 | Build-time only; in Alpine container |
| GraalVM Community Edition | GPL-2.0 WITH Classpath-exception-2.0 | Same as OpenJDK — same verdict |
| Eclipse OpenJ9 | EPL-2.0 + Apache-2.0 | EPL is copyleft; not acceptable |
| Avian JVM | ISC | Too old (Java 7); not viable |
| IcedTea | GPL-2.0 | Wrapper patches; same as OpenJDK |
| `tzdata` (bundled in JDK) | Public Domain | Fully permissive |
| Zlib (compression in JDK) | zlib license | Fully permissive |

---

## 8. Recommended Package Roadmap

### Phase 1: Policy Update

Update DESIGN.md §12 Known Compromises table to document `GPL-2.0 WITH Classpath-exception-2.0`
as a named second exception, scoped to `develop/` packages. Update jpkg allowlist code.

Estimated effort: 1-2 hours.

### Phase 2: OpenJDK 21 from Source (x86_64)

Build OpenJDK 21 from source in the Alpine build container using Temurin 21 as boot JDK.

1. Verify Temurin 21 musl x86_64 binary runs cleanly in the Alpine build container
2. Confirm `configure --with-toolchain-type=clang` succeeds
3. Identify and resolve any Clang-specific warnings that break the build
4. Produce a JDK installation tree; verify `java -version` runs in a jonerix container
5. Package as `openjdk-21.jpkg` (~250 MB)
6. Write smoke test: `java -e 'System.out.println("Hello")'` equivalent

Estimated build time: 60-90 minutes on Castle.
Estimated recipe development time: 1-2 days (most time in debugging configure/build flags).

### Phase 3: JRE Split Package

Using `jlink`, produce a server JRE from the Phase 2 JDK:

```
jlink --add-modules java.base,java.logging,java.naming,java.net.http,java.sql,java.xml \
      --output $DESTDIR/lib/jvm/openjdk-21-jre \
      --strip-debug --compress=2 --no-header-files --no-man-pages
```

Package as `openjdk-21-jre.jpkg` (~120 MB). This is the package users would install to run
existing Java applications on jonerix without full development tools.

Estimated effort: 2-4 hours (recipe addition only; no new build).

### Phase 4: aarch64 Support

Repeat Phase 2 using the Temurin 21 musl aarch64 prebuilt as boot JDK.
Expected: mostly the same flags, minor architecture-specific differences.

Estimated effort: 4-8 hours.

### Phase 5: OpenJDK 25 (future LTS)

When OpenJDK 25 reaches GA (targeted September 2025), bootstrap it from the Phase 2/4
from-source JDK 21 packages rather than a prebuilt binary.

---

## 9. Priority Assessment

Java on jonerix is a significant undertaking — 250 MB of GPL runtime, complex build, and a
policy exception — but Java remains one of the most widely used server-side languages.

**Factors favoring high priority:**
- Java workloads (Spring Boot, Kafka, Elasticsearch, Gradle builds) are common in production
- Many DevOps tools are Java-based (Nexus, Jenkins, Sonarqube)
- No permissive JVM alternative exists; Java users currently cannot run their workloads on jonerix

**Factors favoring deferral:**
- Go, Python, Node.js, and Rust already cover many workloads without GPL concerns
- The build complexity is high; the recipe will be the most complex in the project
- 250 MB is a large package to maintain across two architectures
- The `builder` image is already 350+ MB with LLVM; adding 250 MB of JDK makes it very large

**Priority recommendation**: **Medium — schedule after the Linux kernel recipe (TODO #17)**

Java is a valuable addition but not on the critical path for the base OS. The bootloader,
kernel build, and existing package gaps (btop, sqlite on x86_64) are higher priority. Java
should be scheduled once the build infrastructure is stable and there is user demand for
containerized Java workloads on jonerix.

Suggested milestone: `jonerix:java` image tag — a dedicated container image based on `builder`
that adds OpenJDK 21 JRE, suitable as a base image for Java application containers.

---

## 10. References

- OpenJDK 21 source (LTS update repo): https://github.com/openjdk/jdk21u
- OpenJDK 25 source (next LTS): https://github.com/openjdk/jdk
- Project Portola (musl port): https://openjdk.org/projects/portola/
- Eclipse Temurin musl builds: https://github.com/adoptium/temurin21-binaries/releases/
- Adoptium Temurin API: https://api.adoptium.net/q/swagger-ui/
- Classpath Exception text: https://openjdk.org/legal/gplv2+ce.html
- Alpine OpenJDK packages (reference): https://pkgs.alpinelinux.org/packages?name=openjdk21
- Wolfi OpenJDK packages (reference): https://github.com/wolfi-dev/os/blob/main/openjdk-21.yaml
- OpenJDK build docs: https://github.com/openjdk/jdk/blob/master/doc/building.md
- JEP 220 (JIMAGE modular runtime): https://openjdk.org/jeps/220
- jlink docs: https://docs.oracle.com/en/java/javase/21/docs/specs/man/jlink.html
