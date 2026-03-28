/*
 * jpkg - jonerix package manager
 * cmd_remove.c - jpkg remove: remove package files, update database
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "db.h"
#include "deps.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>

static int remove_package_files(const jpkg_db_t *db, const char *name) {
    db_pkg_t *pkg = db_get_package(db, name);
    if (!pkg) {
        log_error("package %s is not installed", name);
        return -1;
    }

    int errors = 0;

    /* Remove files in reverse order (files before directories) */
    for (pkg_file_t *f = pkg->files; f; f = f->next) {
        char full_path[1024];
        snprintf(full_path, sizeof(full_path), "%s%s", g_rootfs, f->path);

        struct stat st;
        if (lstat(full_path, &st) != 0) {
            log_debug("file already removed: %s", f->path);
            continue;
        }

        if (S_ISDIR(st.st_mode)) {
            /* Only remove directory if empty */
            if (rmdir(full_path) != 0) {
                log_debug("directory not empty, keeping: %s", f->path);
            }
        } else {
            if (unlink(full_path) != 0) {
                log_warn("failed to remove: %s: %s", f->path, strerror(errno));
                errors++;
            } else {
                log_debug("removed: %s", f->path);
            }
        }
    }

    return errors;
}

int cmd_remove(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: jpkg remove <package> [package...]\n");
        return 1;
    }

    /* Parse options */
    bool remove_orphans = true;
    bool force = false;
    int pkg_start = 0;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--orphans") == 0 || strcmp(argv[i], "-o") == 0) {
            remove_orphans = true;
            pkg_start = i + 1;
        } else if (strcmp(argv[i], "--force") == 0 || strcmp(argv[i], "-f") == 0) {
            force = true;
            pkg_start = i + 1;
        } else {
            break;
        }
    }

    if (pkg_start >= argc) {
        fprintf(stderr, "usage: jpkg remove [--orphans] [--force] <package> [package...]\n");
        return 1;
    }

    jpkg_db_t *db = db_open();
    db_load(db);

    int failures = 0;

    for (int i = pkg_start; i < argc; i++) {
        const char *name = argv[i];

        if (!db_is_installed(db, name)) {
            log_error("package %s is not installed", name);
            failures++;
            continue;
        }

        /* Check for dependents (packages that depend on this one) */
        if (!force) {
            size_t dep_count = 0;
            char **dependents = db_get_dependents(db, name, &dep_count);
            if (dep_count > 0) {
                log_error("cannot remove %s: required by:", name);
                for (size_t j = 0; j < dep_count; j++) {
                    fprintf(stderr, "  %s\n", dependents[j]);
                    free(dependents[j]);
                }
                free(dependents);
                failures++;
                continue;
            }
            if (dependents) {
                for (size_t j = 0; j < dep_count; j++) free(dependents[j]);
                free(dependents);
            }
        }

        /* Get removal order (includes orphans if requested) */
        dep_list_t *removal = deps_removal_order(db, name, remove_orphans);
        if (!removal) {
            log_error("failed to compute removal order for %s", name);
            failures++;
            continue;
        }

        /* Show what will be removed */
        if (removal->count > 1) {
            log_info("removing %s and %zu orphaned dependencies:",
                     name, removal->count - 1);
            for (size_t j = 0; j < removal->count; j++) {
                db_pkg_t *pkg = db_get_package(db, removal->packages[j]);
                if (pkg) printf("  %s-%s\n", pkg->name, pkg->version);
            }
        }

        /* Remove each package */
        for (size_t j = 0; j < removal->count; j++) {
            const char *pkg_name = removal->packages[j];

            log_info("removing %s...", pkg_name);

            int rc = remove_package_files(db, pkg_name);
            if (rc > 0) {
                log_warn("%d file(s) could not be removed from %s", rc, pkg_name);
            }

            db_unregister(db, pkg_name);
            log_info("removed %s", pkg_name);
        }

        dep_list_free(removal);
    }

    db_close(db);

    if (failures > 0) {
        log_error("%d package(s) could not be removed", failures);
        return 1;
    }

    log_info("removal complete");
    return 0;
}
