/*
 * jpkg - jonerix package manager
 * cmd_audit.c - jpkg license-audit: verify all installed packages are permissive
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "db.h"
#include "repo.h"
#include "util.h"
#include <stdio.h>
#include <string.h>

int cmd_license_audit(int argc, char **argv) {
    bool verbose = false;
    bool check_repo = false;

    /* Parse options */
    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--verbose") == 0 || strcmp(argv[i], "-v") == 0) {
            verbose = true;
        } else if (strcmp(argv[i], "--repo") == 0 || strcmp(argv[i], "-r") == 0) {
            check_repo = true;
        }
    }

    int violations = 0;
    int unknown = 0;
    int total = 0;

    /* Audit installed packages */
    if (!check_repo) {
        jpkg_db_t *db = db_open();
        db_load(db);

        if (db->package_count == 0) {
            log_info("no packages installed");
            db_close(db);
            return 0;
        }

        printf("License audit of installed packages:\n\n");
        printf("  %-24s %-16s %s\n", "PACKAGE", "LICENSE", "STATUS");
        printf("  %-24s %-16s %s\n", "-------", "-------", "------");

        for (db_pkg_t *pkg = db->packages; pkg; pkg = pkg->next) {
            total++;
            const char *license = pkg->license ? pkg->license : "unknown";
            const char *status;

            if (!pkg->license || strcmp(pkg->license, "unknown") == 0) {
                status = "UNKNOWN";
                unknown++;
            } else if (license_is_permissive(pkg->license)) {
                status = "OK";
            } else {
                status = "VIOLATION";
                violations++;
            }

            if (verbose || strcmp(status, "OK") != 0) {
                printf("  %-24s %-16s %s\n", pkg->name, license, status);
            }
        }

        db_close(db);
    } else {
        /* Audit repository index */
        repo_index_t *idx = repo_index_load();
        if (!idx) {
            log_error("no package index. Run 'jpkg update' first.");
            return 1;
        }

        printf("License audit of repository packages:\n\n");
        printf("  %-24s %-12s %-16s %s\n", "PACKAGE", "VERSION", "LICENSE", "STATUS");
        printf("  %-24s %-12s %-16s %s\n", "-------", "-------", "-------", "------");

        for (repo_entry_t *e = idx->entries; e; e = e->next) {
            total++;
            const char *license = e->license ? e->license : "unknown";
            const char *status;

            if (!e->license || strcmp(e->license, "unknown") == 0) {
                status = "UNKNOWN";
                unknown++;
            } else if (license_is_permissive(e->license)) {
                status = "OK";
            } else {
                status = "VIOLATION";
                violations++;
            }

            if (verbose || strcmp(status, "OK") != 0) {
                printf("  %-24s %-12s %-16s %s\n",
                       e->name, e->version, license, status);
            }
        }

        repo_index_free(idx);
    }

    printf("\n");
    printf("Audit summary:\n");
    printf("  Total packages:     %d\n", total);
    printf("  Permissive (OK):    %d\n", total - violations - unknown);
    printf("  Unknown license:    %d\n", unknown);
    printf("  License violations: %d\n", violations);

    if (violations > 0) {
        printf("\nFAILED: %d package(s) have non-permissive licenses.\n", violations);
        printf("jonerix requires all packages to use permissive licenses\n");
        printf("(MIT, BSD, ISC, Apache-2.0, public domain, etc.)\n");
        return 1;
    }

    if (unknown > 0) {
        printf("\nWARNING: %d package(s) have unknown licenses.\n", unknown);
        printf("Verify these manually before deployment.\n");
        return 0; /* Not a hard failure, but a warning */
    }

    printf("\nPASSED: All packages have permissive licenses.\n");
    return 0;
}
