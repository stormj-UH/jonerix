* [DONE] Split LLVM into separate packages (commit 3a428f22):
    - libllvm: LLVM core, libLLVM-21.so, headers, cmake configs
    - clang: out-of-tree clang + compiler-rt builtins + /etc/clang
    - lld: out-of-tree LLD linker
    - llvm: metapackage (POSIX symlinks: cc, ld, ar, nm, etc.)
    - libcxx: stays as-is (no libLLVM.so link at runtime)
  Remaining: wire rust recipe to use llvm-config = "/bin/llvm-config"
  so Rust dist builds skip their own LLVM build (saves 1-2 hours).

* llvm-libc: ship as optional package, track for eventual musl
  replacement.  Goal is fully LLVM-native stack: llvm-libc + libc++ +
  compiler-rt + libunwind + clang + lld + rust — no GNU, no musl.
  Known gaps as of 2026: locale support, dlopen/dynamic linking, some
  POSIX interfaces incomplete.  Realistic path: ship as optional first,
  let users try building things against it, only consider replacing musl
  once the full package set (Python, OpenSSL, etc.) can bootstrap with
  it.

* UNIX V7 / SUSv4 full compatibility package
