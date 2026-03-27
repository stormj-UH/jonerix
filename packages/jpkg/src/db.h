/*
 * jpkg - jonerix package manager
 * db.h - Local package database (/var/db/jpkg/)
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 *
 * Database layout:
 *   /var/db/jpkg/
 *   +-- installed/
 *   |   +-- <pkgname>/
 *   |       +-- metadata.toml   <- package metadata
 *   |       +-- files           <- list of installed files with sha256
 *   +-- lock                    <- PID lockfile
 */

#ifndef JPKG_DB_H
#define JPKG_DB_H

#include "pkg.h"
#include <stdbool.h>
#include <stddef.h>
#include <time.h>

/* Installed package record */
typedef struct db_pkg {
    char *name;
    char *version;
    char *license;
    char *description;
    char *arch;
    char **runtime_deps;
    size_t runtime_dep_count;
    pkg_file_t *files;
    size_t file_count;
    time_t install_time;
    struct db_pkg *next;
} db_pkg_t;

/* Database handle */
typedef struct jpkg_db {
    char *db_dir;         /* e.g., "/var/db/jpkg" */
    db_pkg_t *packages;
    size_t package_count;
    int lock_fd;
} jpkg_db_t;

/* Open the package database (acquires lock) */
jpkg_db_t *db_open(void);

/* Close the database (releases lock) */
void db_close(jpkg_db_t *db);

/* Load all installed package info */
int db_load(jpkg_db_t *db);

/* Check if a package is installed */
bool db_is_installed(const jpkg_db_t *db, const char *name);

/* Get installed package info (returns NULL if not installed) */
db_pkg_t *db_get_package(const jpkg_db_t *db, const char *name);

/* Register a newly installed package */
int db_register(jpkg_db_t *db, const pkg_meta_t *meta, const pkg_file_t *files);

/* Remove a package from the database */
int db_unregister(jpkg_db_t *db, const char *name);

/* Get list of all installed packages */
db_pkg_t *db_list_installed(const jpkg_db_t *db);

/* Get packages that depend on the given package */
char **db_get_dependents(const jpkg_db_t *db, const char *name, size_t *count);

/* Free a package record */
void db_pkg_free(db_pkg_t *pkg);

/* Verify installed files against database manifests.
 * Returns number of mismatches (0 = all OK). */
int db_verify_files(const jpkg_db_t *db, const char *name,
                    int (*callback)(const char *path, const char *expected,
                                    const char *actual, void *ctx),
                    void *ctx);

#endif /* JPKG_DB_H */
