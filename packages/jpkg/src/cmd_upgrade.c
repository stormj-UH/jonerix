/*
 * jpkg - jonerix package manager
 * cmd_upgrade.c - jpkg upgrade: compare installed vs INDEX, install newer
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "repo.h"
#include "db.h"
#include "pkg.h"
#include "fetch.h"
#include "sign.h"
#include "util.h"
#include <stdio.h>
#include <string.h>

/* Forward declaration of install function from cmd_install.c */
extern int cmd_install(int argc, char **argv);

int cmd_upgrade(int argc, char **argv) {
    log_info("checking for upgrades...");

    /* Initialize subsystems */
    sign_load_keys();

    /* First, update the index */
    repo_config_t *cfg = repo_config_load();
    if (!cfg) {
        log_error("failed to load repository configuration");
        fetch_cleanup();
        return 1;
    }

    log_info("fetching latest package index...");
    if (repo_update(cfg) != 0) {
        log_error("failed to update package index");
        repo_config_free(cfg);
        fetch_cleanup();
        return 1;
    }

    repo_index_t *idx = repo_index_load();
    if (!idx) {
        log_error("no package index available");
        repo_config_free(cfg);
        fetch_cleanup();
        return 1;
    }

    jpkg_db_t *db = db_open();
    db_load(db);

    /* Find packages that need upgrading */
    size_t upgrade_cap = 32;
    size_t upgrade_count = 0;
    char **upgrades = xcalloc(upgrade_cap, sizeof(char *));
    int selection_failures = 0;
    bool explicit_targets = argc > 0;

    if (explicit_targets) {
        for (int i = 0; i < argc; i++) {
            const char *name = argv[i];
            db_pkg_t *pkg = db_get_package(db, name);
            if (!pkg) {
                log_error("package %s is not installed", name);
                selection_failures++;
                continue;
            }

            repo_entry_t *entry = repo_find_package(idx, pkg->name);
            if (!entry) {
                log_error("package %s not found in repository index", pkg->name);
                selection_failures++;
                continue;
            }

            if (upgrade_count >= upgrade_cap) {
                upgrade_cap *= 2;
                upgrades = xrealloc(upgrades, upgrade_cap * sizeof(char *));
            }
            upgrades[upgrade_count++] = xstrdup(pkg->name);

            int cmp = version_compare(entry->version, pkg->version);
            if (cmp > 0) {
                log_info("  %s: %s -> %s", pkg->name, pkg->version, entry->version);
            }
        }
    } else {
        for (db_pkg_t *pkg = db->packages; pkg; pkg = pkg->next) {
            repo_entry_t *entry = repo_find_package(idx, pkg->name);
            if (!entry) {
                log_debug("package %s not found in repository index", pkg->name);
                continue;
            }

            int cmp = version_compare(entry->version, pkg->version);
            if (cmp > 0) {
                if (upgrade_count >= upgrade_cap) {
                    upgrade_cap *= 2;
                    upgrades = xrealloc(upgrades, upgrade_cap * sizeof(char *));
                }
                upgrades[upgrade_count++] = xstrdup(pkg->name);
                log_info("  %s: %s -> %s", pkg->name, pkg->version, entry->version);
            }
        }
    }

    if (upgrade_count == 0) {
        if (selection_failures > 0) {
            log_error("%d requested package(s) could not be upgraded", selection_failures);
            free(upgrades);
            db_close(db);
            repo_index_free(idx);
            repo_config_free(cfg);
            fetch_cleanup();
            return 1;
        }

        log_info("all packages are up to date");
        free(upgrades);
        db_close(db);
        repo_index_free(idx);
        repo_config_free(cfg);
        fetch_cleanup();
        return 0;
    }

    if (explicit_targets) {
        log_info("%zu package(s) requested for upgrade", upgrade_count);
    } else {
        log_info("%zu package(s) to upgrade", upgrade_count);
    }

    /* Use install command to upgrade each package. Prepend --force so that
     * cmd_install's "already installed / suggest upgrade" filter doesn't
     * block us — we've already determined these are real upgrades. */
    char **install_argv = xcalloc(upgrade_count + 1, sizeof(char *));
    install_argv[0] = xstrdup("--force");
    for (size_t i = 0; i < upgrade_count; i++) install_argv[i + 1] = upgrades[i];
    int rc = cmd_install((int)(upgrade_count + 1), install_argv);

    free(install_argv[0]);
    free(install_argv);
    for (size_t i = 0; i < upgrade_count; i++) free(upgrades[i]);
    free(upgrades);
    db_close(db);
    repo_index_free(idx);
    repo_config_free(cfg);
    fetch_cleanup();

    if (selection_failures > 0) {
        log_error("%d requested package(s) could not be upgraded", selection_failures);
        return 1;
    }

    return rc;
}
