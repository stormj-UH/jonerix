/*
 * jpkg - jonerix package manager
 * cmd_upgrade.c - jpkg upgrade: compare installed vs INDEX, install newer
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
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
    (void)argc;
    (void)argv;

    log_info("checking for upgrades...");

    /* Initialize subsystems */
    sign_load_keys();
    if (fetch_init() != 0) return 1;

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

    if (upgrade_count == 0) {
        log_info("all packages are up to date");
        free(upgrades);
        db_close(db);
        repo_index_free(idx);
        repo_config_free(cfg);
        fetch_cleanup();
        return 0;
    }

    log_info("%zu package(s) to upgrade", upgrade_count);

    /* Use install command to upgrade each package (it handles upgrades) */
    int rc = cmd_install((int)upgrade_count, upgrades);

    for (size_t i = 0; i < upgrade_count; i++) free(upgrades[i]);
    free(upgrades);
    db_close(db);
    repo_index_free(idx);
    repo_config_free(cfg);
    fetch_cleanup();

    return rc;
}
