# Add GNU make function compatibility

## Summary

This patch adds support for 27 GNU make built-in functions to bmake's variable
expansion engine. When bmake encounters a `$(function args)` expression where
the function name matches a known GNU make function, it evaluates it directly
instead of treating it as an undefined variable.

This allows bmake to process Makefiles that use GNU make extensions, enabling
projects to use bmake as a drop-in replacement for GNU make in many real-world
build systems.

## Functions implemented (27)

### String functions
- `$(subst from,to,text)` — literal string substitution
- `$(patsubst pattern,replacement,text)` — pattern substitution with `%` wildcard
- `$(strip text)` — collapse whitespace
- `$(findstring find,in)` — search for substring

### File name functions
- `$(dir names...)` — extract directory part
- `$(notdir names...)` — extract non-directory part
- `$(basename names...)` — strip suffix
- `$(suffix names...)` — extract suffix
- `$(addprefix prefix,names...)` — prepend prefix to each word
- `$(addsuffix suffix,names...)` — append suffix to each word
- `$(wildcard pattern...)` — glob expansion
- `$(abspath names...)` — absolute path (lexical)
- `$(realpath names...)` — canonical path (resolves symlinks)

### List/word functions
- `$(sort list)` — sort and deduplicate words
- `$(filter pattern...,text)` — keep matching words
- `$(filter-out pattern...,text)` — remove matching words
- `$(words text)` — count words
- `$(word n,text)` — extract nth word
- `$(firstword text)` — extract first word
- `$(lastword text)` — extract last word
- `$(wordlist s,e,text)` — extract word range

### Control functions
- `$(if condition,then[,else])` — conditional (non-empty = true)
- `$(or arg1[,arg2,...])` — return first non-empty argument
- `$(and arg1[,arg2,...])` — return last if all non-empty, else empty
- `$(foreach var,list,text)` — (detected but passed through to bmake's existing .for)

### I/O functions
- `$(shell command)` — execute shell command, capture stdout
- `$(error text)` — fatal error message
- `$(warning text)` — warning message
- `$(info text)` — informational message

## Design

The implementation hooks into bmake's variable lookup path in `VarFind()`. When
a variable name contains a space and the first word matches a known function
name, `GnuFunc_Eval()` is called before falling through to the "undefined
variable" path. This ensures:

1. Normal bmake variables take precedence over function names
2. No existing behavior is changed for valid bmake expressions
3. Functions are only evaluated when they would otherwise be undefined

### Argument parsing

Arguments are split by commas with awareness of nested `$(...)` and `${...}`
expressions, so function calls can be nested:

```makefile
$(filter %.o,$(patsubst %.c,%.o,$(wildcard src/*.c)))
```

The argument limit is 64 (sufficient for all GNU make functions).

### Pattern matching

The `%` wildcard in `filter`, `filter-out`, and `patsubst` works identically
to GNU make: `%` matches any number of characters, with at most one `%` per
pattern. Prefix and suffix around `%` must match literally.

## Bug fixes included

- **sort use-after-free**: The `gnufunc_sort()` implementation converts
  `SubstringWords` to owned C strings before sorting, then frees the
  `SubstringWords` immediately. This avoids a use-after-free that would occur
  if sorting operated on `Substring` pointers into a freed buffer.

## Motivation

Many real-world projects (musl, curl, Python, Node.js, Linux kernel config,
etc.) use GNU make functions in their Makefiles. Without this patch, bmake
silently expands these as empty strings, causing subtle build failures that
are difficult to diagnose.

This patch was developed for the [jonerix](https://github.com/stormj-UH/jonerix)
project, which uses bmake as its sole build tool (replacing GNU make to maintain
a permissive-license-only toolchain). The 27 functions implemented cover all
GNU make functions encountered in the bootstrap build of ~27 packages from
source.

## Testing

The patch has been tested by building the following packages from source using
bmake with this patch applied:

- musl, zstd, lz4, openssl, curl, dropbear, mksh, toybox, openrc, cmake,
  perl, python3, Node.js, ncurses, and others (27 packages total)

## Compatibility

- No changes to existing bmake behavior for valid bmake expressions
- Only activates when a variable lookup would otherwise return "undefined"
- Adds `#include <glob.h>` for the `wildcard` function
- No new command-line flags or configuration required
