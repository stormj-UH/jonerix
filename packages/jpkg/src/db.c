/*
 * jpkg - jonerix package manager
 * db.c - Local package database (/var/db/jpkg/)
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 *
 * Database layout:
 *   /var/db/jpkg/installed/<pkgname>/metadata.toml
 *   /var/db/jpkg/installed/<pkgname>/files
 *   /var/db/jpkg/lock
 */

#include "db.h"
#include "toml.h"
#include "util.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <dirent.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <time.h>

/* ========== Lock Management (POSIX fcntl locking) ========== */

static int db_lock(const char *db_dir) {
    char lock_path[512];
    snprintf(lock_path, sizeof(lock_path), "%s/lock", db_dir);

    int fd = open(lock_path, O_WRONLY | O_CREAT, 0644);
    if (fd < 0) {
        log_error("failed to open lock file: %s: %s", lock_path, strerror(errno));
        return -1;
    }

    struct flock fl;
    memset(&fl, 0, sizeof(fl));
    fl.l_type = F_WRLCK;
    fl.l_whence = SEEK_SET;
    fl.l_start = 0;
    fl.l_len = 0; /* entire file */

    if (fcntl(fd, F_SETLK, &fl) != 0) {
        if (errno == EACCES || errno == EAGAIN) {
            log_error("package database is locked by another process");
        } else {
            log_error("failed to lock database: %s", strerror(errno));
        }
        close(fd);
        return -1;
    }

    /* Write PID to lock file */
    char pid_str[32];
    int len = snprintf(pid_str, sizeof(pid_str), "%d\n", (int)getpid());
    if (ftruncate(fd, 0) == 0) {
        ssize_t w = write(fd, pid_str, (size_t)len);
        (void)w;
    }

    return fd;
}

static void db_unlock(int fd) {
    if (fd >= 0) {
        struct flock fl;
        memset(&fl, 0, sizeof(fl));
        fl.l_type = F_UNLCK;
        fl.l_whence = SEEK_SET;
        fl.l_start = 0;
        fl.l_len = 0;
        fcntl(fd, F_SETLK, &fl);
        close(fd);
    }
}

/* ========== File List Parsing ========== */

/*
 * files format: one line per file
 *   <sha256> <mode_octal> <path>
 */

static pkg_file_t *parse_files_list(const char *data, size_t *count) {
    pkg_file_t *head = NULL;
    *count = 0;

    const char *p = data;
    while (*p) {
        /* Skip blank lines */
        if (*p == '\n') { p++; continue; }

        /* Read sha256 (64 hex chars) */
        char sha[65];
        if (strlen(p) < 64) break;
        memcpy(sha, p, 64);
        sha[64] = '\0';
        p += 64;
        if (*p != ' ') break;
        p++;

        /* Read mode (octal) */
        char mode_str[16];
        int mi = 0;
        while (*p && *p != ' ' && mi < 15) {
            mode_str[mi++] = *p++;
        }
        mode_str[mi] = '\0';
        if (*p != ' ') break;
        p++;

        /* Read path until newline */
        const char *start = p;
        while (*p && *p != '\n') p++;
        size_t plen = (size_t)(p - start);

        pkg_file_t *f = xcalloc(1, sizeof(pkg_file_t));
        f->path = xstrndup(start, plen);
        memcpy(f->sha256, sha, 65);
        f->mode = (uint32_t)strtoul(mode_str, NULL, 8);
        f->next = head;
        head = f;
        (*count)++;

        if (*p == '\n') p++;
    }

    return head;
}

static char *serialize_files_list(const pkg_file_t *files) {
    size_t cap = 4096, len = 0;
    char *buf = xmalloc(cap);
    buf[0] = '\0';

    for (const pkg_file_t *f = files; f; f = f->next) {
        size_t needed = 64 + 1 + 8 + 1 + strlen(f->path) + 2;
        while (len + needed >= cap) { cap *= 2; buf = xrealloc(buf, cap); }
        len += (size_t)snprintf(buf + len, cap - len, "%s %06o %s\n",
                                f->sha256, f->mode, f->path);
    }

    return buf;
}

/* ========== Package Loading ========== */

static db_pkg_t *load_package(const char *db_dir, const char *name) {
    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path), "%s/installed/%s/metadata.toml",
             db_dir, name);

    uint8_t *data;
    ssize_t len = file_read(meta_path, &data);
    if (len <= 0) return NULL;

    char *err = NULL;
    toml_doc_t *doc = toml_parse((const char *)data, &err);
    free(data);

    if (!doc) {
        log_warn("failed to parse metadata for %s: %s", name, err ? err : "?");
        free(err);
        return NULL;
    }

    db_pkg_t *pkg = xcalloc(1, sizeof(db_pkg_t));
    pkg->name = xstrdup(name);

    const char *s;
    if ((s = toml_get_string(doc, "package.version")))
        pkg->version = xstrdup(s);
    else
        pkg->version = xstrdup("0");

    if ((s = toml_get_string(doc, "package.license")))
        pkg->license = xstrdup(s);
    if ((s = toml_get_string(doc, "package.description")))
        pkg->description = xstrdup(s);
    if ((s = toml_get_string(doc, "package.arch")))
        pkg->arch = xstrdup(s);

    int64_t ts;
    if (toml_get_integer(doc, "package.install_time", &ts))
        pkg->install_time = (time_t)ts;

    /* Dependencies */
    const toml_array_t *arr = toml_get_array(doc, "depends.runtime");
    if (arr && arr->count > 0) {
        pkg->runtime_deps = xcalloc(arr->count, sizeof(char *));
        pkg->runtime_dep_count = arr->count;
        for (size_t i = 0; i < arr->count; i++)
            pkg->runtime_deps[i] = xstrdup(arr->items[i]);
    }

    toml_free(doc);

    /* Load file list */
    char files_path[512];
    snprintf(files_path, sizeof(files_path), "%s/installed/%s/files",
             db_dir, name);

    len = file_read(files_path, &data);
    if (len > 0) {
        pkg->files = parse_files_list((const char *)data, &pkg->file_count);
        free(data);
    }

    return pkg;
}

/* ========== Public API ========== */

jpkg_db_t *db_open(void) {
    jpkg_db_t *db = xcalloc(1, sizeof(jpkg_db_t));

    char db_dir[512];
    snprintf(db_dir, sizeof(db_dir), "%s%s", g_rootfs, JPKG_DB_DIR);
    db->db_dir = xstrdup(db_dir);

    /* Create directories */
    char installed_dir[512];
    snprintf(installed_dir, sizeof(installed_dir), "%s/installed", db_dir);
    mkdirs(installed_dir, 0755);

    /* Acquire lock */
    db->lock_fd = db_lock(db_dir);
    if (db->lock_fd < 0) {
        /* Non-fatal for read-only operations */
        log_debug("running without database lock");
    }

    return db;
}

void db_close(jpkg_db_t *db) {
    if (!db) return;

    db_unlock(db->lock_fd);

    /* Free package list */
    db_pkg_t *p = db->packages;
    while (p) {
        db_pkg_t *next = p->next;
        db_pkg_free(p);
        p = next;
    }

    free(db->db_dir);
    free(db);
}

int db_load(jpkg_db_t *db) {
    if (!db) return -1;

    char installed_dir[512];
    snprintf(installed_dir, sizeof(installed_dir), "%s/installed", db->db_dir);

    DIR *dir = opendir(installed_dir);
    if (!dir) {
        /* No installed packages */
        return 0;
    }

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (ent->d_name[0] == '.') continue;

        db_pkg_t *pkg = load_package(db->db_dir, ent->d_name);
        if (pkg) {
            pkg->next = db->packages;
            db->packages = pkg;
            db->package_count++;
        }
    }

    closedir(dir);
    log_debug("loaded %zu installed packages", db->package_count);
    return 0;
}

bool db_is_installed(const jpkg_db_t *db, const char *name) {
    return db_get_package(db, name) != NULL;
}

db_pkg_t *db_get_package(const jpkg_db_t *db, const char *name) {
    if (!db || !name) return NULL;
    for (db_pkg_t *p = db->packages; p; p = p->next) {
        if (strcmp(p->name, name) == 0) return p;
    }
    return NULL;
}

int db_register(jpkg_db_t *db, const pkg_meta_t *meta, const pkg_file_t *files) {
    if (!db || !meta || !meta->name) return -1;

    char pkg_dir[512];
    snprintf(pkg_dir, sizeof(pkg_dir), "%s/installed/%s", db->db_dir, meta->name);
    mkdirs(pkg_dir, 0755);

    /* Write metadata.toml */
    toml_doc_t *doc = toml_new();
    toml_set_string(doc, "package.name", meta->name);
    toml_set_string(doc, "package.version", meta->version);
    if (meta->license) toml_set_string(doc, "package.license", meta->license);
    if (meta->description)
        toml_set_string(doc, "package.description", meta->description);
    if (meta->arch) toml_set_string(doc, "package.arch", meta->arch);
    toml_set_integer(doc, "package.install_time", (int64_t)time(NULL));

    if (meta->runtime_dep_count > 0) {
        toml_set_array(doc, "depends.runtime",
                       (const char **)meta->runtime_deps, meta->runtime_dep_count);
    }

    char *toml_str = toml_serialize(doc);
    toml_free(doc);

    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata.toml", pkg_dir);
    int rc = file_write(meta_path, (const uint8_t *)toml_str, strlen(toml_str));
    free(toml_str);
    if (rc != 0) {
        log_error("failed to write metadata for %s", meta->name);
        return -1;
    }

    /* Write file list */
    if (files) {
        char *files_str = serialize_files_list(files);
        char files_path[512];
        snprintf(files_path, sizeof(files_path), "%s/files", pkg_dir);
        rc = file_write(files_path, (const uint8_t *)files_str, strlen(files_str));
        free(files_str);
        if (rc != 0) {
            log_error("failed to write file list for %s", meta->name);
            return -1;
        }
    }

    /* Add to in-memory database */
    db_pkg_t *pkg = xcalloc(1, sizeof(db_pkg_t));
    pkg->name = xstrdup(meta->name);
    pkg->version = xstrdup(meta->version);
    if (meta->license) pkg->license = xstrdup(meta->license);
    if (meta->description) pkg->description = xstrdup(meta->description);
    if (meta->arch) pkg->arch = xstrdup(meta->arch);
    pkg->install_time = time(NULL);

    if (meta->runtime_dep_count > 0) {
        pkg->runtime_deps = xcalloc(meta->runtime_dep_count, sizeof(char *));
        pkg->runtime_dep_count = meta->runtime_dep_count;
        for (size_t i = 0; i < meta->runtime_dep_count; i++)
            pkg->runtime_deps[i] = xstrdup(meta->runtime_deps[i]);
    }

    /* Copy file list */
    for (const pkg_file_t *f = files; f; f = f->next) {
        pkg_file_t *nf = xcalloc(1, sizeof(pkg_file_t));
        nf->path = xstrdup(f->path);
        memcpy(nf->sha256, f->sha256, 65);
        nf->size = f->size;
        nf->mode = f->mode;
        nf->next = pkg->files;
        pkg->files = nf;
        pkg->file_count++;
    }

    pkg->next = db->packages;
    db->packages = pkg;
    db->package_count++;

    log_debug("registered %s-%s in database", meta->name, meta->version);
    return 0;
}

int db_unregister(jpkg_db_t *db, const char *name) {
    if (!db || !name) return -1;

    /* Remove from filesystem */
    char pkg_dir[512];
    snprintf(pkg_dir, sizeof(pkg_dir), "%s/installed/%s", db->db_dir, name);

    char meta_path[512];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata.toml", pkg_dir);
    unlink(meta_path);

    char files_path[512];
    snprintf(files_path, sizeof(files_path), "%s/files", pkg_dir);
    unlink(files_path);

    rmdir(pkg_dir);

    /* Remove from in-memory list */
    db_pkg_t **pp = &db->packages;
    while (*pp) {
        if (strcmp((*pp)->name, name) == 0) {
            db_pkg_t *victim = *pp;
            *pp = victim->next;
            db_pkg_free(victim);
            db->package_count--;
            break;
        }
        pp = &(*pp)->next;
    }

    log_debug("unregistered %s from database", name);
    return 0;
}

db_pkg_t *db_list_installed(const jpkg_db_t *db) {
    return db ? db->packages : NULL;
}

char **db_get_dependents(const jpkg_db_t *db, const char *name, size_t *count) {
    if (!db || !name || !count) return NULL;

    size_t cap = 16;
    char **deps = xcalloc(cap, sizeof(char *));
    *count = 0;

    for (db_pkg_t *p = db->packages; p; p = p->next) {
        for (size_t i = 0; i < p->runtime_dep_count; i++) {
            if (strcmp(p->runtime_deps[i], name) == 0) {
                if (*count >= cap) {
                    cap *= 2;
                    deps = xrealloc(deps, cap * sizeof(char *));
                }
                deps[*count] = xstrdup(p->name);
                (*count)++;
                break;
            }
        }
    }

    return deps;
}

void db_pkg_free(db_pkg_t *pkg) {
    if (!pkg) return;
    free(pkg->name);
    free(pkg->version);
    free(pkg->license);
    free(pkg->description);
    free(pkg->arch);

    for (size_t i = 0; i < pkg->runtime_dep_count; i++)
        free(pkg->runtime_deps[i]);
    free(pkg->runtime_deps);

    pkg_file_t *f = pkg->files;
    while (f) {
        pkg_file_t *next = f->next;
        free(f->path);
        free(f);
        f = next;
    }

    free(pkg);
}

int db_verify_files(const jpkg_db_t *db, const char *name,
                    int (*callback)(const char *path, const char *expected,
                                    const char *actual, void *ctx),
                    void *ctx) {
    db_pkg_t *pkg = db_get_package(db, name);
    if (!pkg) {
        log_error("package %s is not installed", name);
        return -1;
    }

    int mismatches = 0;

    for (pkg_file_t *f = pkg->files; f; f = f->next) {
        char full_path[1024];
        snprintf(full_path, sizeof(full_path), "%s%s", g_rootfs, f->path);

        if (!file_exists(full_path)) {
            if (callback) callback(f->path, f->sha256, "(missing)", ctx);
            mismatches++;
            continue;
        }

        char actual_hash[65];
        if (sha256_file(full_path, actual_hash) != 0) {
            if (callback) callback(f->path, f->sha256, "(error)", ctx);
            mismatches++;
            continue;
        }

        if (strcmp(f->sha256, actual_hash) != 0) {
            if (callback) callback(f->path, f->sha256, actual_hash, ctx);
            mismatches++;
        }
    }

    return mismatches;
}
