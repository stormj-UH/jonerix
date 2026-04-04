/*
 * jpkg - jonerix package manager
 * cmd_install.c - jpkg install: resolve deps, fetch, verify, extract
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "repo.h"
#include "db.h"
#include "deps.h"
#include "fetch.h"
#include "pkg.h"
#include "sign.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

/* Build a file manifest by walking a directory tree */
static pkg_file_t *build_file_manifest(const char *root_dir, const char *prefix) {
    pkg_file_t *head = NULL;
    DIR *dir = opendir(root_dir);
    if (!dir) return NULL;

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0)
            continue;

        char *full_path = path_join(root_dir, ent->d_name);
        char *rel_path = path_join(prefix, ent->d_name);

        struct stat st;
        if (lstat(full_path, &st) != 0) {
            free(full_path);
            free(rel_path);
            continue;
        }

        if (S_ISDIR(st.st_mode)) {
            /* Recurse into subdirectory */
            pkg_file_t *sub = build_file_manifest(full_path, rel_path);
            if (sub) {
                /* Append sub-list to head */
                pkg_file_t *tail = sub;
                while (tail->next) tail = tail->next;
                tail->next = head;
                head = sub;
            }
        } else if (S_ISLNK(st.st_mode)) {
            /* Symlink */
            char target[1024];
            ssize_t tlen = readlink(full_path, target, sizeof(target) - 1);
            if (tlen > 0) {
                target[tlen] = '\0';
                pkg_file_t *f = xcalloc(1, sizeof(pkg_file_t));
                f->path = xstrdup(rel_path);
                f->link_target = xstrdup(target);
                f->mode = (uint32_t)st.st_mode & 07777;
                memset(f->sha256, '0', 64);
                f->sha256[64] = '\0';
                f->next = head;
                head = f;
            }
        } else if (S_ISREG(st.st_mode)) {
            pkg_file_t *f = xcalloc(1, sizeof(pkg_file_t));
            f->path = xstrdup(rel_path);
            f->size = (uint64_t)st.st_size;
            f->mode = (uint32_t)st.st_mode & 07777;

            /* Compute SHA256 */
            sha256_file(full_path, f->sha256);

            f->next = head;
            head = f;
        }

        free(full_path);
        free(rel_path);
    }
    closedir(dir);
    return head;
}

/* Copy extracted files from staging to root filesystem */
static int install_files(const char *stage_dir, const char *dest_root) {
    char cmd[4096];
    /*
     * Install files from staging to root filesystem.
     *
     * pkg_extract() already flattens usr/ in the staging directory,
     * so we just need a single recursive copy of the staging contents.
     *
     * Uses a single `cp -a staging/. dest/` to copy everything in
     * one process.  This replaces the old per-file glob loop which
     * forked cp once per top-level entry and hung on packages with
     * many files (e.g. perl with ~900 man pages at root level).
     *
     * Minor caveat: cp -a follows destination symlinks, so if a
     * destination path is a symlink to a running binary (e.g.
     * /bin/clear -> toybox while toybox sh executes), cp gets
     * ETXTBSY.  This only affects a handful of toybox applet
     * symlinks and is non-fatal — the real binaries from later
     * package installs will overwrite them.
     */
    snprintf(cmd, sizeof(cmd),
             /* Safety: flatten usr/ if pkg_extract somehow missed it */
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cp -a '%s/usr/.' '%s/' && rm -rf '%s/usr'; fi; "
             /* Copy all staging contents to root in one shot.
              * Errors (ETXTBSY on toybox symlinks) are non-fatal. */
             "cp -a '%s/.' '%s/' 2>/dev/null; true",
             stage_dir, stage_dir,
             stage_dir, stage_dir, stage_dir,
             stage_dir, dest_root);
    return system(cmd);
}

static int install_single_package(const repo_config_t *cfg, const repo_index_t *idx,
                                  jpkg_db_t *db, const char *name) {
    repo_entry_t *entry = repo_find_package(idx, name);
    if (!entry) {
        log_error("package not found: %s", name);
        return -1;
    }

    /* Check if already installed with same version */
    db_pkg_t *installed = db_get_package(db, name);
    if (installed && strcmp(installed->version, entry->version) == 0) {
        log_info("%s-%s is already installed", name, entry->version);
        return 0;
    }

    /* Download package */
    char *pkg_path = repo_fetch_package(cfg, entry);
    if (!pkg_path) {
        log_error("failed to download %s", name);
        return -1;
    }

    /* Parse and verify package */
    size_t payload_off, payload_len;
    pkg_meta_t *meta = pkg_parse_file(pkg_path, &payload_off, &payload_len);
    if (!meta) {
        log_error("failed to parse package: %s", pkg_path);
        free(pkg_path);
        return -1;
    }

    log_info("installing %s-%s...", meta->name, meta->version);

    /* Extract to staging directory */
    char stage_dir[256];
    snprintf(stage_dir, sizeof(stage_dir), "/tmp/jpkg-stage-%s-%d",
             meta->name, (int)getpid());

    int rc = pkg_extract(pkg_path, stage_dir);
    if (rc != 0) {
        log_error("failed to extract %s", meta->name);
        pkg_meta_free(meta);
        free(pkg_path);
        return -1;
    }

    /* Build file manifest from extracted files */
    pkg_file_t *files = build_file_manifest(stage_dir, "/");
    if (!files) {
        log_error("package %s extracted no installable files", meta->name);
        char cmd[512];
        snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
        system(cmd);
        pkg_meta_free(meta);
        free(pkg_path);
        return -1;
    }

    /* Copy files to root filesystem */
    char dest_root[512];
    snprintf(dest_root, sizeof(dest_root), "%s/", g_rootfs[0] ? g_rootfs : "");
    if (dest_root[0] == '\0') strcpy(dest_root, "/");

    rc = install_files(stage_dir, dest_root);
    if (rc != 0) {
        log_error("failed to install files for %s", meta->name);
        /* Clean up staging directory */
        char cmd[512];
        snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
        system(cmd);
        pkg_meta_free(meta);
        free(pkg_path);
        return -1;
    }

    /* Register in database */
    if (installed) {
        /* Upgrading - remove old registration first */
        db_unregister(db, name);
    }
    db_register(db, meta, files);

    /* Clean up */
    char cmd[512];
    snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
    system(cmd);

    /* Free file list */
    pkg_file_t *f = files;
    while (f) {
        pkg_file_t *next = f->next;
        free(f->path);
        free(f->link_target);
        free(f);
        f = next;
    }

    log_info("installed %s-%s", meta->name, meta->version);
    pkg_meta_free(meta);
    free(pkg_path);
    return 0;
}

int cmd_install(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: jpkg install <package> [package...]\n");
        return 1;
    }

    /* Initialize subsystems */
    sign_load_keys();
    if (fetch_init() != 0) return 1;

    repo_config_t *cfg = repo_config_load();
    repo_index_t *idx = repo_index_load();
    if (!idx) {
        log_error("no package index. Run 'jpkg update' first.");
        repo_config_free(cfg);
        fetch_cleanup();
        return 1;
    }

    jpkg_db_t *db = db_open();
    db_load(db);

    /* Resolve dependencies for all requested packages */
    dep_list_t *install_list = deps_resolve_multi(idx, db,
        (const char **)argv, (size_t)argc, false);

    if (!install_list) {
        log_error("dependency resolution failed");
        db_close(db);
        repo_index_free(idx);
        repo_config_free(cfg);
        fetch_cleanup();
        return 1;
    }

    if (install_list->count == 0) {
        log_info("nothing to install - all packages are up to date");
        dep_list_free(install_list);
        db_close(db);
        repo_index_free(idx);
        repo_config_free(cfg);
        fetch_cleanup();
        return 0;
    }

    /* Show what will be installed */
    log_info("packages to install (%zu):", install_list->count);
    for (size_t i = 0; i < install_list->count; i++) {
        repo_entry_t *e = repo_find_package(idx, install_list->packages[i]);
        if (e) {
            printf("  %s-%s\n", e->name, e->version);
        }
    }

    /* Install each package in dependency order */
    int failures = 0;
    for (size_t i = 0; i < install_list->count; i++) {
        if (install_single_package(cfg, idx, db, install_list->packages[i]) != 0) {
            failures++;
        }
    }

    dep_list_free(install_list);
    db_close(db);
    repo_index_free(idx);
    repo_config_free(cfg);
    fetch_cleanup();

    if (failures > 0) {
        log_error("%d package(s) failed to install", failures);
        return 1;
    }

    log_info("installation complete");
    return 0;
}
