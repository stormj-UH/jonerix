/*
 * jpkg - jonerix package manager
 * cmd_info.c - jpkg info: show package metadata
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "repo.h"
#include "db.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <time.h>

static void print_entry_info(const repo_entry_t *entry, const db_pkg_t *installed) {
    printf("Name:         %s\n", entry->name);
    printf("Version:      %s\n", entry->version);
    printf("License:      %s\n", entry->license ? entry->license : "unknown");
    printf("Architecture: %s\n", entry->arch ? entry->arch : "unknown");
    printf("Description:  %s\n", entry->description ? entry->description : "");

    if (entry->size > 0) {
        if (entry->size >= 1048576)
            printf("Package size: %.1f MiB\n", (double)entry->size / 1048576.0);
        else if (entry->size >= 1024)
            printf("Package size: %.1f KiB\n", (double)entry->size / 1024.0);
        else
            printf("Package size: %llu bytes\n", (unsigned long long)entry->size);
    }

    if (entry->sha256 && entry->sha256[0])
        printf("SHA256:       %s\n", entry->sha256);

    if (entry->runtime_dep_count > 0) {
        printf("Dependencies: ");
        for (size_t i = 0; i < entry->runtime_dep_count; i++) {
            if (i > 0) printf(", ");
            printf("%s", entry->runtime_deps[i]);
        }
        printf("\n");
    }

    if (entry->build_dep_count > 0) {
        printf("Build deps:   ");
        for (size_t i = 0; i < entry->build_dep_count; i++) {
            if (i > 0) printf(", ");
            printf("%s", entry->build_deps[i]);
        }
        printf("\n");
    }

    if (installed) {
        printf("Status:       installed (%s", installed->version);
        if (installed->install_time > 0) {
            char timebuf[64];
            struct tm *tm = localtime(&installed->install_time);
            strftime(timebuf, sizeof(timebuf), "%Y-%m-%d %H:%M:%S", tm);
            printf(", %s", timebuf);
        }
        printf(")\n");
        if (installed->file_count > 0)
            printf("Installed files: %zu\n", installed->file_count);
    } else {
        printf("Status:       not installed\n");
    }

    /* License check */
    if (entry->license) {
        if (license_is_permissive(entry->license)) {
            printf("License OK:   yes (permissive)\n");
        } else {
            printf("License OK:   WARNING - not recognized as permissive\n");
        }
    }
}

static void print_installed_info(const db_pkg_t *pkg) {
    printf("Name:         %s\n", pkg->name);
    printf("Version:      %s\n", pkg->version);
    printf("License:      %s\n", pkg->license ? pkg->license : "unknown");
    printf("Architecture: %s\n", pkg->arch ? pkg->arch : "unknown");
    printf("Description:  %s\n", pkg->description ? pkg->description : "");

    if (pkg->install_time > 0) {
        char timebuf[64];
        struct tm *tm = localtime(&pkg->install_time);
        strftime(timebuf, sizeof(timebuf), "%Y-%m-%d %H:%M:%S", tm);
        printf("Installed:    %s\n", timebuf);
    }

    if (pkg->runtime_dep_count > 0) {
        printf("Dependencies: ");
        for (size_t i = 0; i < pkg->runtime_dep_count; i++) {
            if (i > 0) printf(", ");
            printf("%s", pkg->runtime_deps[i]);
        }
        printf("\n");
    }

    printf("Files:        %zu\n", pkg->file_count);
    printf("Status:       installed\n");
}

int cmd_info(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: jpkg info <package>\n");
        return 1;
    }

    bool show_files = false;
    const char *pkg_name = NULL;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--files") == 0 || strcmp(argv[i], "-f") == 0) {
            show_files = true;
        } else {
            pkg_name = argv[i];
        }
    }

    if (!pkg_name) {
        fprintf(stderr, "usage: jpkg info [--files] <package>\n");
        return 1;
    }

    /* Try loading from repository index */
    repo_index_t *idx = repo_index_load();

    /* Load installed packages */
    jpkg_db_t *db = db_open();
    db_load(db);

    db_pkg_t *installed = db_get_package(db, pkg_name);
    repo_entry_t *entry = idx ? repo_find_package(idx, pkg_name) : NULL;

    if (!installed && !entry) {
        log_error("package '%s' not found", pkg_name);
        db_close(db);
        if (idx) repo_index_free(idx);
        return 1;
    }

    if (entry) {
        print_entry_info(entry, installed);
    } else {
        print_installed_info(installed);
    }

    /* Show file list if requested */
    if (show_files && installed) {
        printf("\nInstalled files:\n");
        for (pkg_file_t *f = installed->files; f; f = f->next) {
            printf("  %s\n", f->path);
        }
    }

    db_close(db);
    if (idx) repo_index_free(idx);
    return 0;
}
