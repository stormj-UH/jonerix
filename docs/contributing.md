# Contributing to jonerix

Thank you for your interest in contributing to jonerix. This document covers guidelines for contributing code, packages, documentation, and bug reports.

## Licensing Requirement

**This is the most important rule.** Every contribution to jonerix must be compatible with the project's permissive licensing policy:

- All code you contribute must be released under the **MIT License** (the project's license).
- All packages you add must use a permissive license: MIT, BSD, ISC, Apache-2.0, 0BSD, CC0, Zlib, public domain, or equivalent.
- **GPL, LGPL, and AGPL contributions will not be accepted.** No exceptions for userland code.
- The Linux kernel (GPLv2) is the sole documented exception.

By submitting a pull request, you agree that your contribution is licensed under MIT.

### How to Check

Before adding a new package or dependency:

1. Read the upstream project's `LICENSE` or `COPYING` file.
2. Verify the SPDX license identifier.
3. Run the license audit: `sh scripts/license-audit.sh --recipes --verbose`
4. If in doubt, ask in an issue before investing time on the implementation.

## Getting Started

### Setting Up the Development Environment

```sh
# Clone the repository
git clone https://github.com/jonerix/jonerix.git
cd jonerix

# Docker-based development (recommended)
docker build -f Dockerfile.develop --tag jonerix-develop:latest .
docker run -it -v $(pwd):/workspace -w /workspace jonerix-develop:latest
```

### Repository Structure

```
jonerix/
  bootstrap/            build-all.sh and build environment docs
  packages/core/        Package metadata (recipe.toml per package)
  packages/bootstrap/   From-source build recipes (recipe.toml per package)
  packages/jpkg/        Package manager source code (C)
  config/               OpenRC services, system defaults
  scripts/              Utility scripts (license audit)
  docs/                 Documentation
  .github/              CI workflows
```

## Types of Contributions

### 1. Package Recipes

Adding new packages is the most common contribution. See [docs/packaging.md](packaging.md) for the full recipe format.

Quick checklist:
- [ ] License is permissive (run `scripts/license-audit.sh`)
- [ ] Source URL is stable (prefer tagged releases, not `master` branches)
- [ ] SHA256 hash is correct
- [ ] Package builds cleanly with `bootstrap/build-all.sh`
- [ ] Package installs to `$DESTDIR` correctly (no hardcoded paths)
- [ ] Patches are minimal and well-documented

### 2. Bug Fixes

- Open an issue first to discuss the bug (unless it's trivial)
- Include reproduction steps in the issue
- Reference the issue number in your PR

### 3. Bootstrap Improvements

The build system (`bootstrap/build-all.sh` and `packages/bootstrap/*/recipe.toml`) is a critical path. Changes here should be:
- Well-tested (ideally in CI)
- Backward-compatible where possible
- Documented if they change the build process

### 4. Documentation

Documentation improvements are always welcome:
- Fix typos, clarify confusing sections
- Add examples
- Document new features or packages
- Translate to other languages

### 5. CI/CD Improvements

Improvements to `.github/workflows/` are welcome, especially:
- Faster builds (caching, parallelism)
- Better error reporting
- New checks (security scanning, size regression)

## Code Style

### Shell Scripts

jonerix shell scripts target POSIX `sh`. Do not use bashisms.

```sh
# Good: POSIX-compatible
if [ "$var" = "value" ]; then
    command
fi

for item in a b c; do
    echo "$item"
done

# Bad: bash-specific
if [[ "$var" == "value" ]]; then  # [[ is not POSIX
    command
fi

array=(a b c)  # Arrays are not POSIX
for item in "${array[@]}"; do
    echo "$item"
done
```

Guidelines:
- Use `#!/bin/sh` shebang (not `#!/bin/bash`)
- Use `set -eu` at the top of scripts
- Quote all variable expansions: `"$var"`, not `$var`
- Use `$(command)` instead of backticks
- Use `printf` instead of `echo` for portability in critical paths
- Add error handling: check return codes, use `|| die "message"`
- Include a usage comment at the top of each script

### Makefiles

- Use tabs for indentation (required by Make)
- Use `$(VARIABLE)` syntax, not `${VARIABLE}`
- Keep recipes simple and readable
- Document non-obvious targets

### C Code (jpkg, jnft)

- C11 standard
- musl-compatible (no glibc extensions)
- No external dependencies beyond musl and OpenSSL
- Compile with `-Wall -Wextra -Werror`
- Use static analysis: `clang --analyze`
- Follow the existing code style in `packages/jpkg/src/`

## Submitting Changes

### Branch Naming

- `feature/<description>` for new features
- `fix/<description>` for bug fixes
- `pkg/<package-name>` for new package recipes
- `docs/<description>` for documentation changes

### Commit Messages

Write clear, descriptive commit messages:

```
Add toybox 0.8.11 package recipe

- Build recipe with BSD-compatible configuration
- Enable coreutils, file utils, and network applets
- Disable applets that overlap with dedicated packages
- Tested on x86_64 with musl 1.2.5
```

Guidelines:
- First line: imperative mood, max 72 characters
- Blank line after the first line
- Body: explain *what* and *why*, not *how*
- Reference issues: `Fixes #42` or `Relates to #17`

### Pull Request Process

1. **Fork the repository** and create your branch.

2. **Make your changes**. Ensure:
   - `scripts/license-audit.sh --recipes` passes
   - Shell scripts pass `shellcheck` (if available)
   - New scripts are executable (`chmod +x`)
   - Documentation is updated if applicable

3. **Test locally**:
   ```sh
   # Build inside develop container
   docker run --rm -v "$PWD:/workspace" -w /workspace jonerix-develop:latest \
     sh bootstrap/build-all.sh --package mypackage

   # License audit
   sh scripts/license-audit.sh --recipes --verbose
   ```

4. **Open a Pull Request**:
   - Fill in the PR template
   - Describe what changed and why
   - Include test results
   - Reference related issues

5. **Review process**:
   - A maintainer will review your PR
   - CI must pass (license check, build)
   - Address review feedback with new commits (do not force-push)

6. **Merge**: After approval, a maintainer will merge your PR.

## Reporting Issues

### Bug Reports

Include:
- What you expected to happen
- What actually happened
- Steps to reproduce
- System information (architecture, host OS)
- Relevant log output

### Feature Requests

Include:
- Description of the feature
- Use case (why is it needed?)
- License of any proposed new dependencies
- Willingness to implement it yourself

## Security Issues

For security vulnerabilities, do **not** open a public issue. Instead:
- Email the maintainers directly (see the project README for contact info)
- Include a description of the vulnerability
- Include reproduction steps if possible
- Allow time for a fix before public disclosure

## Governance

jonerix is maintained by its contributors. Decisions are made through:
1. **Discussion in issues** for proposals and design decisions
2. **Pull request review** for code changes
3. **Maintainer consensus** for significant architectural changes

The licensing policy (permissive-only userland) is non-negotiable. It is the foundational principle of the project.

## Code of Conduct

Be respectful and constructive. Technical disagreements are fine; personal attacks are not. We are all here to build something useful.

Focus on:
- Technical merit of contributions
- Clear communication
- Helping newcomers get started
- Maintaining the project's quality and principles
