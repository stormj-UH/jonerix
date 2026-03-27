# packages/core/llvm/Makefile — LLVM compiler suite (Clang + lld + libc++ + libc++abi)
# SPDX-License-Identifier: MIT

PKG_NAME     = llvm-project
PKG_VERSION  = 19.1.5
PKG_LICENSE  = Apache-2.0
PKG_SOURCE   = https://github.com/llvm/llvm-project/releases/download/llvmorg-$(PKG_VERSION)/llvm-project-$(PKG_VERSION).src.tar.xz
PKG_SHA256   = FIXME

include ../../rules.mk

configure: patch
	mkdir -p $(BUILD_DIR)
	cd $(BUILD_DIR) && \
	cmake $(SRC_DIR)/llvm \
		-G Ninja \
		-DCMAKE_C_COMPILER="$(CC)" \
		-DCMAKE_CXX_COMPILER="clang++" \
		-DCMAKE_C_FLAGS="$(CFLAGS)" \
		-DCMAKE_CXX_FLAGS="$(CFLAGS)" \
		-DCMAKE_EXE_LINKER_FLAGS="$(LDFLAGS)" \
		-DCMAKE_INSTALL_PREFIX=/ \
		-DCMAKE_BUILD_TYPE=Release \
		-DLLVM_ENABLE_PROJECTS="clang;lld" \
		-DLLVM_ENABLE_RUNTIMES="compiler-rt;libcxx;libcxxabi" \
		-DLLVM_TARGETS_TO_BUILD="X86;AArch64" \
		-DLLVM_DEFAULT_TARGET_TRIPLE="x86_64-linux-musl" \
		-DLLVM_ENABLE_ASSERTIONS=OFF \
		-DLLVM_ENABLE_ZSTD=ON \
		-DLLVM_ENABLE_ZLIB=ON \
		-DLLVM_ENABLE_LIBXML2=OFF \
		-DLLVM_ENABLE_TERMINFO=OFF \
		-DLLVM_ENABLE_LIBEDIT=OFF \
		-DLLVM_INCLUDE_BENCHMARKS=OFF \
		-DLLVM_INCLUDE_EXAMPLES=OFF \
		-DLLVM_INCLUDE_TESTS=OFF \
		-DLLVM_INCLUDE_DOCS=OFF \
		-DCLANG_DEFAULT_LINKER=lld \
		-DCLANG_DEFAULT_CXX_STDLIB=libc++ \
		-DCLANG_DEFAULT_RTLIB=compiler-rt \
		-DBUILD_SHARED_LIBS=OFF \
		-DLLVM_BUILD_LLVM_DYLIB=OFF \
		-DLLVM_INSTALL_TOOLCHAIN_ONLY=OFF \
		-DLIBCXX_HAS_MUSL_LIBC=ON \
		-DLIBCXX_USE_COMPILER_RT=ON \
		-DLIBCXX_ENABLE_STATIC=ON \
		-DLIBCXX_ENABLE_SHARED=OFF \
		-DLIBCXXABI_USE_COMPILER_RT=ON \
		-DLIBCXXABI_USE_LLVM_UNWINDER=OFF \
		-DLIBCXXABI_ENABLE_STATIC=ON \
		-DLIBCXXABI_ENABLE_SHARED=OFF

build: configure
	ninja -C $(BUILD_DIR) -j$$(nproc)

install: build
	DESTDIR=$(DESTDIR) ninja -C $(BUILD_DIR) install
	# Verify libc++ headers and libraries were installed
	@test -f $(DESTDIR)/include/c++/v1/__config || \
		echo "WARNING: libc++ headers not found in $(DESTDIR)/include/c++/v1/"
	@test -f $(DESTDIR)/lib/libc++.a || \
		echo "WARNING: libc++.a not found in $(DESTDIR)/lib/"
	@test -f $(DESTDIR)/lib/libc++abi.a || \
		echo "WARNING: libc++abi.a not found in $(DESTDIR)/lib/"
	# Create convenience symlinks
	ln -sf clang $(DESTDIR)/bin/cc
	ln -sf clang++ $(DESTDIR)/bin/c++
	ln -sf lld $(DESTDIR)/bin/ld
	ln -sf llvm-ar $(DESTDIR)/bin/ar
	ln -sf llvm-ranlib $(DESTDIR)/bin/ranlib
	ln -sf llvm-nm $(DESTDIR)/bin/nm
	ln -sf llvm-objcopy $(DESTDIR)/bin/objcopy
	ln -sf llvm-objdump $(DESTDIR)/bin/objdump
	ln -sf llvm-strip $(DESTDIR)/bin/strip
