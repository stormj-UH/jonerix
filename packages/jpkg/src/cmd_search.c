/*
 * jpkg - jonerix package manager
 * cmd_search.c - jpkg search: search package names/descriptions
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "repo.h"
#include "db.h"
#include "util.h"
#include <stdio.h>
#include <string.h>

int cmd_search(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: jpkg search <query>\n");
        return 1;
    }

    repo_index_t *idx = repo_index_load();
    if (!idx) {
        log_error("no package index. Run 'jpkg update' first.");
        return 1;
    }

    /* Optionally load installed packages to show status */
    jpkg_db_t *db = db_open();
    db_load(db);

    /* Concatenate all arguments as the search query */
    char query[1024] = "";
    for (int i = 0; i < argc; i++) {
        if (i > 0) strcat(query, " ");
        strncat(query, argv[i], sizeof(query) - strlen(query) - 1);
    }

    size_t result_count = 0;
    repo_entry_t **results = repo_search(idx, query, &result_count);

    if (result_count == 0) {
        printf("No packages found matching '%s'\n", query);
    } else {
        printf("Found %zu package(s) matching '%s':\n\n", result_count, query);

        for (size_t i = 0; i < result_count; i++) {
            repo_entry_t *e = results[i];
            bool installed = db_is_installed(db, e->name);

            printf("  %-24s %-12s %-12s %s\n",
                   e->name, e->version, e->license ? e->license : "",
                   installed ? "[installed]" : "");

            if (e->description && e->description[0]) {
                printf("    %s\n", e->description);
            }
        }
    }

    repo_search_free(results);
    db_close(db);
    repo_index_free(idx);
    return 0;
}
