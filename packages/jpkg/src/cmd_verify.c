/*
 * jpkg - jonerix package manager
 * cmd_verify.c - jpkg verify: check installed files against manifests (SHA256)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "db.h"
#include "util.h"
#include <stdio.h>
#include <string.h>

typedef struct verify_ctx {
    int missing;
    int modified;
    int errors;
    bool verbose;
} verify_ctx_t;

static int verify_callback(const char *path, const char *expected,
                           const char *actual, void *ctx) {
    verify_ctx_t *vc = (verify_ctx_t *)ctx;

    if (strcmp(actual, "(missing)") == 0) {
        if (vc->verbose) printf("  MISSING: %s\n", path);
        vc->missing++;
    } else if (strcmp(actual, "(error)") == 0) {
        if (vc->verbose) printf("  ERROR:   %s\n", path);
        vc->errors++;
    } else {
        if (vc->verbose) {
            printf("  MODIFIED: %s\n", path);
            printf("    expected: %s\n", expected);
            printf("    actual:   %s\n", actual);
        }
        vc->modified++;
    }

    return 0;
}

int cmd_verify(int argc, char **argv) {
    bool verbose = true;
    const char *pkg_name = NULL;

    /* Parse options */
    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--quiet") == 0 || strcmp(argv[i], "-q") == 0) {
            verbose = false;
        } else {
            pkg_name = argv[i];
        }
    }

    jpkg_db_t *db = db_open();
    db_load(db);

    if (db->package_count == 0) {
        log_info("no packages installed");
        db_close(db);
        return 0;
    }

    int total_mismatches = 0;
    int packages_ok = 0;
    int packages_bad = 0;

    if (pkg_name) {
        /* Verify a single package */
        if (!db_is_installed(db, pkg_name)) {
            log_error("package %s is not installed", pkg_name);
            db_close(db);
            return 1;
        }

        printf("Verifying %s...\n", pkg_name);

        verify_ctx_t vc = { .verbose = verbose };
        int mismatches = db_verify_files(db, pkg_name, verify_callback, &vc);

        if (mismatches == 0) {
            printf("  OK: all files verified\n");
        } else {
            printf("  FAIL: %d missing, %d modified, %d errors\n",
                   vc.missing, vc.modified, vc.errors);
            total_mismatches += mismatches;
        }
    } else {
        /* Verify all installed packages */
        printf("Verifying all installed packages...\n\n");

        for (db_pkg_t *pkg = db->packages; pkg; pkg = pkg->next) {
            if (verbose) printf("Checking %s-%s...", pkg->name, pkg->version);

            verify_ctx_t vc = { .verbose = false };
            int mismatches = db_verify_files(db, pkg->name, verify_callback, &vc);

            if (mismatches == 0) {
                if (verbose) printf(" OK\n");
                packages_ok++;
            } else {
                if (verbose) {
                    printf(" FAIL (%d missing, %d modified, %d errors)\n",
                           vc.missing, vc.modified, vc.errors);
                }
                packages_bad++;
                total_mismatches += mismatches;
            }
        }

        printf("\nVerification summary:\n");
        printf("  Packages OK:     %d\n", packages_ok);
        printf("  Packages failed: %d\n", packages_bad);
        printf("  Total issues:    %d\n", total_mismatches);
    }

    db_close(db);
    return total_mismatches > 0 ? 1 : 0;
}
