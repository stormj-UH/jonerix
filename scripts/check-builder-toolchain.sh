#!/bin/sh
# Verify the builder image pulled the expected signed toolchain package set.

set -eu

fail() {
    printf 'builder-toolchain-check: %s\n' "$1" >&2
    exit 1
}

package_version() {
    pkg=$1
    meta="/var/db/jpkg/installed/$pkg/metadata.toml"
    [ -f "$meta" ] || fail "$pkg is not installed"
    sed -n 's/^version = "\(.*\)"/\1/p' "$meta" | sed -n '1p'
}

require_package_version() {
    pkg=$1
    want=$2
    got=$(package_version "$pkg")
    [ "$got" = "$want" ] || fail "$pkg version is $got, expected $want"
    printf 'OK package %s-%s\n' "$pkg" "$got"
}

require_target() {
    target=$1
    printf '%s\n' "$llvm_targets" | tr ' ' '\n' | grep -x "$target" >/dev/null 2>&1 ||
        fail "llvm-config --targets-built is missing $target"
}

require_package_version jpkg 2.2.8
require_package_version libllvm 21.1.2-r2
require_package_version clang 21.1.2-r1
require_package_version lld 21.1.2-r2
require_package_version llvm 21.1.2-r7
require_package_version llvm-extra 21.1.2-r3

jpkg --version | grep '2\.2\.8' >/dev/null 2>&1 ||
    fail "jpkg --version does not report 2.2.8"

clang --version | grep 'clang version 21\.1\.2' >/dev/null 2>&1 ||
    fail "clang --version does not report 21.1.2"

llvm_targets=$(llvm-config --targets-built 2>/dev/null) ||
    fail "llvm-config --targets-built failed"

for target in \
    AArch64 AMDGPU ARM AVR BPF Hexagon Lanai LoongArch Mips MSP430 NVPTX \
    PowerPC RISCV SPIRV Sparc SystemZ VE WebAssembly X86 XCore
do
    require_target "$target"
done
printf 'OK llvm target set: %s\n' "$llvm_targets"

triple=$(clang -dumpmachine 2>/dev/null) ||
    fail "clang -dumpmachine failed"
profile="/lib/clang/21/lib/$triple/libclang_rt.profile.a"
[ -f "$profile" ] || fail "missing profile runtime: $profile"
printf 'OK profile runtime %s\n' "$profile"
