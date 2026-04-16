/*
 * jpkg - jonerix package manager
 * cmd_install.c - jpkg install: resolve deps, fetch, verify, extract
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
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

/* Run a package hook (pre_install, post_install, etc.) */
static int run_hook(const char *hook_name, const char *cmd, const char *pkg_name) {
    if (!cmd || !cmd[0]) return 0;
    log_info("running %s hook for %s...", hook_name, pkg_name);
    int rc = system(cmd);
    if (rc != 0) {
        log_error("%s hook failed for %s (exit %d)", hook_name, pkg_name, rc);
        return -1;
    }
    return 0;
}

/* Callback for db_check_conflicts — logs each conflict */
static void conflict_cb(const char *path, const char *owner, void *ctx) {
    (void)ctx;
    log_error("  %s: owned by %s", path, owner);
}

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
/*
 * Find a working tar command.  Returns a static string.
 *
 * Preference: bsdtar (handles symlinks correctly, good format support),
 * then toybox tar, then plain tar.  We test each by running it with
 * --version / --help and checking the exit code.
 */
static const char *find_tar(void) {
    static const char *cached = NULL;
    if (cached) return cached;

    /* bsdtar from libarchive — best option */
    if (system("bsdtar --version >/dev/null 2>&1") == 0) {
        cached = "bsdtar";
        return cached;
    }
    /* toybox tar — available on jonerix */
    if (system("toybox tar --help >/dev/null 2>&1") == 0) {
        cached = "toybox tar";
        return cached;
    }
    /* Generic tar (busybox, GNU, etc.) */
    if (system("tar --help >/dev/null 2>&1") == 0) {
        cached = "tar";
        return cached;
    }

    log_error("no tar implementation found (tried bsdtar, toybox tar, tar)");
    return NULL;
}

static int install_files(const char *stage_dir, const char *dest_root) {
    char cmd[4096];
    /*
     * Install files from staging to root filesystem.
     *
     * pkg_extract() already flattens usr/ in the staging directory,
     * so we just need a single recursive copy of the staging contents.
     *
     * IMPORTANT: Do NOT use `cp -a staging/. dest/` here.
     * toybox `cp -a` follows DESTINATION symlinks when the destination
     * path is a symlink to a file.  On jonerix, toybox applet symlinks
     * like /bin/clear -> toybox and /bin/reset -> toybox get followed,
     * causing packages (e.g. ncurses) to overwrite /bin/toybox with
     * their own binary.  This destroys the multicall binary.
     *
     * We use a two-step tar approach (create archive, then extract)
     * instead of a pipe.  Pipes can deadlock on large packages because
     * toybox sh keeps the read end of the pipe open in its fd table
     * while waiting for children — once the pipe buffer fills, the
     * writer blocks forever.  The LLVM package (200MB+, 1300+ files)
     * reliably triggers this.
     *
     * tar -x correctly replaces destination symlinks with new files
     * instead of following them.
     */

    const char *tar = find_tar();
    if (!tar) return -1;

    /* Safety: flatten usr/ if pkg_extract somehow missed it */
    snprintf(cmd, sizeof(cmd),
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cd '%s/usr' && %s -cf - . | %s -xpf - -C '%s' && "
             "cd / && rm -rf '%s/usr'; fi",
             stage_dir, stage_dir,
             stage_dir, tar, tar, dest_root,
             stage_dir);
    system(cmd);

    /* Create a temp tarball from the staging directory, then extract it
     * into the destination root.  Two steps avoids the pipe deadlock. */
    char tmp_tar[256];
    snprintf(tmp_tar, sizeof(tmp_tar), "/tmp/jpkg-install-%d.tar", (int)getpid());

    snprintf(cmd, sizeof(cmd),
             "cd '%s' && %s -cf '%s' .",
             stage_dir, tar, tmp_tar);
    int rc = system(cmd);
    if (rc != 0) {
        log_error("failed to create install tarball (exit %d)", rc);
        unlink(tmp_tar);
        return -1;
    }

    snprintf(cmd, sizeof(cmd),
             "%s -xf '%s' -C '%s'",
             tar, tmp_tar, dest_root);
    rc = system(cmd);
    unlink(tmp_tar);
    if (rc != 0 && rc != 256) {
        log_error("failed to extract install tarball (exit %d)", rc);
        return -1;
    }

    return 0;
}

static int install_single_package(const repo_config_t *cfg, const repo_index_t *idx,
                                  jpkg_db_t *db, const char *name, bool force) {
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

    /* Flatten sbin/ → bin/ if present (jonerix uses flat /bin layout) */
    {
        char sbin_dir[512];
        snprintf(sbin_dir, sizeof(sbin_dir), "%s/sbin", stage_dir);
        if (dir_exists(sbin_dir)) {
            char flatten[1024];
            snprintf(flatten, sizeof(flatten),
                "mkdir -p '%s/bin' && cp -a '%s/sbin/.' '%s/bin/' && rm -rf '%s/sbin'",
                stage_dir, stage_dir, stage_dir, stage_dir);
            system(flatten);
            log_info("flattened /sbin → /bin for %s", meta->name);
        }
    }

    {
        char problem[1024];
        tree_audit_result_t audit = audit_layout_tree(stage_dir, problem, sizeof(problem));
        if (audit != TREE_AUDIT_OK) {
            log_error("refusing to install %s: %s at %s",
                      meta->name, audit_layout_result_string(audit),
                      problem[0] ? problem : "(unknown)");
            char cmd[512];
            snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
            system(cmd);
            pkg_meta_free(meta);
            free(pkg_path);
            return -1;
        }
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

    /* Check for file conflicts with other installed packages.
     * Packages listed in meta->replaces are silently allowed — we'll
     * transfer file ownership from them after the install succeeds. */
    int conflicts = db_check_conflicts(db, files,
                                       installed ? name : NULL,
                                       (const char *const *)meta->replaces,
                                       meta->replaces_count,
                                       conflict_cb, NULL);
    if (conflicts > 0) {
        if (!force) {
            log_error("%d file conflict(s) detected for %s — use --force to override",
                      conflicts, meta->name);
            char cmd[512];
            snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
            system(cmd);
            pkg_file_t *f = files;
            while (f) { pkg_file_t *n = f->next; free(f->path); free(f->link_target); free(f); f = n; }
            pkg_meta_free(meta);
            free(pkg_path);
            return -1;
        }
        log_warn("%d file conflict(s) detected — proceeding (--force)", conflicts);
    }

    /* Run pre-install hook */
    if (run_hook("pre_install", meta->pre_install, meta->name) != 0) {
        char cmd[512];
        snprintf(cmd, sizeof(cmd), "rm -rf '%s'", stage_dir);
        system(cmd);
        pkg_file_t *f = files;
        while (f) { pkg_file_t *n = f->next; free(f->path); free(f->link_target); free(f); f = n; }
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

    /* Transfer ownership of any files this package's `replaces` list
     * claims from previously-installed packages. Rewrites the replaced
     * packages' manifests so `jpkg verify` stays clean. */
    if (meta->replaces_count > 0) {
        int n = db_transfer_ownership(db, files,
                                      (const char *const *)meta->replaces,
                                      meta->replaces_count);
        if (n > 0) log_info("transferred %d file(s) from replaced package(s)", n);
    }

    /* Run post-install hook */
    run_hook("post_install", meta->post_install, meta->name);

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
        fprintf(stderr, "usage: jpkg install [--force] <package> [package...]\n");
        return 1;
    }

    /* Parse options */
    bool force = false;
    int pkg_start = 0;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--force") == 0 || strcmp(argv[i], "-f") == 0) {
            force = true;
            pkg_start = i + 1;
        } else {
            break;
        }
    }

    if (pkg_start >= argc) {
        fprintf(stderr, "usage: jpkg install [--force] <package> [package...]\n");
        return 1;
    }

    /* Initialize subsystems */
    sign_load_keys();

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
        (const char **)(argv + pkg_start), (size_t)(argc - pkg_start), force);

    if (!install_list) {
        log_error("dependency resolution failed");
        db_close(db);
        repo_index_free(idx);
        repo_config_free(cfg);
        fetch_cleanup();
        return 1;
    }

    /* Report status of explicitly requested packages (skip when --force) */
    if (!force) for (int i = pkg_start; i < argc; i++) {
        const char *name = argv[i];
        db_pkg_t *installed = db_get_package(db, name);
        if (!installed) continue;

        repo_entry_t *entry = repo_find_package(idx, name);
        if (!entry) continue;

        if (strcmp(installed->version, entry->version) == 0) {
            log_info("%s-%s is already installed", name, installed->version);
        } else if (version_compare(entry->version, installed->version) > 0) {
            log_info("%s-%s is installed; a newer version (%s) is available"
                     " \xe2\x80\x94 run 'jpkg upgrade %s' to update",
                     name, installed->version, entry->version, name);
        }
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
        if (install_single_package(cfg, idx, db, install_list->packages[i], force) != 0) {
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
