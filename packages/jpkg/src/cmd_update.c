/*
 * jpkg - jonerix package manager
 * cmd_update.c - jpkg update: fetch INDEX.zst from mirrors, verify signature
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "repo.h"
#include "sign.h"
#include "util.h"
#include <stdio.h>
#include <string.h>

int cmd_update(int argc, char **argv) {
    (void)argc;
    (void)argv;

    log_info("updating package index...");

    /* Initialize signing subsystem */
    sign_load_keys();

    /* Load repository configuration */
    repo_config_t *cfg = repo_config_load();
    if (!cfg) {
        log_error("failed to load repository configuration");
        return 1;
    }

    log_info("architecture: %s", cfg->arch);
    log_info("configured %zu mirror(s)", cfg->mirror_count);

    /* Fetch and verify INDEX */
    int rc = repo_update(cfg);
    if (rc != 0) {
        log_error("update failed");
        repo_config_free(cfg);
        return 1;
    }

    /* Load the index to report statistics */
    repo_index_t *idx = repo_index_load();
    if (idx) {
        log_info("index contains %zu packages", idx->entry_count);
        if (idx->timestamp)
            log_info("index timestamp: %s", idx->timestamp);
        repo_index_free(idx);
    }

    repo_config_free(cfg);
    log_info("update complete");
    return 0;
}
