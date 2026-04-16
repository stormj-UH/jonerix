/*
 * jpkg - jonerix package manager
 * pkg.h - Package format parsing
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#ifndef JPKG_PKG_H
#define JPKG_PKG_H

#include "toml.h"
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

/* JPKG magic: "JPKG\x00\x01\x00\x00" */
#define JPKG_MAGIC      "JPKG\x00\x01\x00\x00"
#define JPKG_MAGIC_LEN  8
#define JPKG_HEADER_MIN 12  /* magic(8) + header_len(4) */

/* File entry in a package manifest */
typedef struct pkg_file {
    char *path;
    char sha256[65];  /* hex string */
    uint64_t size;
    uint32_t mode;
    char *link_target; /* symlink target (NULL for regular files) */
    struct pkg_file *next;
} pkg_file_t;

/* Parsed package metadata */
typedef struct pkg_meta {
    char *name;
    char *version;
    char *license;
    char *description;
    char *arch;

    /* dependencies */
    char **runtime_deps;
    size_t runtime_dep_count;
    char **build_deps;
    size_t build_dep_count;

    /* `replaces` — packages whose files this one may silently overwrite.
     * When installing pkg X, files that would conflict with a package in
     * replaces[] are transferred: the replaced package's manifest loses
     * the entry, X's manifest gains it. Models the intentional "full
     * impl shadows toybox" overrides (mksh replaces toybox's /bin/sh,
     * bsdtar replaces /bin/tar, etc). Parsed from package.replaces. */
    char **replaces;
    size_t replaces_count;

    /* `conflicts` — packages this one refuses to coinstall with.
     * Unlike `replaces` (silent file-ownership transfer), conflicts is
     * a hard gate: if any listed package is already installed, the
     * install is refused unless --force is given. Use for genuine
     * mutual-exclusion cases: e.g. two DHCP clients (dhcpcd vs udhcpc),
     * two init systems, two cron daemons. Parsed from
     * package.conflicts. */
    char **conflicts;
    size_t conflicts_count;

    /* file info */
    char *content_sha256;   /* hash of zstd tar payload */
    uint64_t content_size;

    /* file manifest */
    pkg_file_t *files;
    size_t file_count;

    /* install/remove hooks (shell commands, may be NULL) */
    char *pre_install;
    char *post_install;
    char *pre_remove;
    char *post_remove;
} pkg_meta_t;

/* Parse a .jpkg file from a memory buffer.
 * Extracts metadata and provides offset/length of the zstd payload.
 * Returns NULL on error. */
pkg_meta_t *pkg_parse_buffer(const uint8_t *data, size_t len,
                             size_t *payload_offset, size_t *payload_len);

/* Parse a .jpkg file on disk */
pkg_meta_t *pkg_parse_file(const char *path,
                           size_t *payload_offset, size_t *payload_len);

/* Parse only the TOML metadata from a metadata string */
pkg_meta_t *pkg_meta_from_toml(const char *toml_str);

/* Create a .jpkg file from a TOML metadata string and a zstd tar payload */
int pkg_create(const char *output_path,
               const char *toml_metadata,
               const uint8_t *zstd_payload, size_t zstd_len);

/* Extract the zstd tar payload from a .jpkg file to a directory */
int pkg_extract(const char *jpkg_path, const char *dest_dir);

/* Free package metadata */
void pkg_meta_free(pkg_meta_t *meta);

/* Build a TOML metadata string from pkg_meta_t (caller frees) */
char *pkg_meta_to_toml(const pkg_meta_t *meta);

/* Validate the magic header bytes */
bool pkg_validate_magic(const uint8_t *data, size_t len);

/* Get the filename for a package: "name-version.jpkg" (caller frees) */
char *pkg_filename(const char *name, const char *version);

#endif /* JPKG_PKG_H */
