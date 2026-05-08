* Split LLVM into separate packages so each consumer builds against a
  shared libLLVM.so instead of rebuilding LLVM from source:
    - libllvm: core libraries, libLLVM.so, headers, cmake configs,
      llvm-config
    - clang: clang + compiler-rt, built with -DLLVM_DIR against libllvm
    - lld: linker, built with -DLLVM_DIR against libllvm
    - rust: rustc + cargo, built with llvm-config = "/path" against
      libllvm (saves 1-2 hours per dist build)
    - libcxx: libc++/libc++abi/libunwind stay as-is — they need clang
      to build but don't link libLLVM.so at runtime
  Current builder image builds LLVM+clang+lld monolithically with
  -DLLVM_ENABLE_PROJECTS.  Split means building libllvm first, then
  each consumer separately — more recipes but each rebuild is faster
  and only what changed gets rebuilt.

* llvm-libc: ship as optional package, track for eventual musl
  replacement.  Goal is fully LLVM-native stack: llvm-libc + libc++ +
  compiler-rt + libunwind + clang + lld + rust — no GNU, no musl.
  Known gaps as of 2026: locale support, dlopen/dynamic linking, some
  POSIX interfaces incomplete.  Realistic path: ship as optional first,
  let users try building things against it, only consider replacing musl
  once the full package set (Python, OpenSSL, etc.) can bootstrap with
  it.

* UNIX V7 / SUSv4 full compatibility package
