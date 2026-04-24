/*
 * jpkg-local - jpkg subcommand for local .jpkg install and recipe builds
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Usage:
 *   jpkg-local install <file.jpkg|url|-> [--root <dir>]
 *   jpkg-local build   <recipe-dir|recipe.toml|url|-> [--output <dir>] [--build-jpkg]
 *
 * Sources can be a local path, an HTTP/HTTPS URL (fetched via curl),
 * or "-" to read from stdin.
 *
 * Compiled standalone against jpkg's shared source files:
 *   util.c, toml.c, pkg.c, db.c
 */

#include "util.h"
#include "pkg.h"
#include "db.h"
#include "toml.h"

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <dirent.h>
#include <errno.h>

/* ========== Source fetching (local file, URL, stdin) ========== */

static bool is_url(const char *s) {
    return strncmp(s, "http://", 7) == 0 || strncmp(s, "https://", 8) == 0;
}

/*
 * Resolve a source argument to a local file path.
 * - Local path: returned as-is, *needs_cleanup = false
 * - URL: fetched via curl to a tmpfile, *needs_cleanup = true
 * - "-": stdin read to a tmpfile, *needs_cleanup = true
 * Returns NULL on failure.
 */
static char *fetch_to_tmp(const char *source, const char *suffix, bool *needs_cleanup) {
    *needs_cleanup = false;

    if (strcmp(source, "-") == 0) {
        /* Read stdin to a temp file */
        char *tmp = xstrdup("/tmp/jpkg-local-stdin-XXXXXX");
        int fd = mkstemp(tmp);
        if (fd < 0) {
            log_error("failed to create temp file: %s", strerror(errno));
            free(tmp);
            return NULL;
        }
        char buf[8192];
        ssize_t n;
        while ((n = read(STDIN_FILENO, buf, sizeof(buf))) > 0) {
            if (write(fd, buf, (size_t)n) != n) {
                log_error("failed to write stdin to temp file");
                close(fd);
                unlink(tmp);
                free(tmp);
                return NULL;
            }
        }
        close(fd);
        *needs_cleanup = true;
        return tmp;
    }

    if (is_url(source)) {
        /* Determine a temp filename using the URL basename or suffix */
        const char *basename = strrchr(source, '/');
        basename = (basename && basename[1]) ? basename + 1 : suffix;
        char *tmp;
        if (asprintf(&tmp, "/tmp/jpkg-local-fetch-%d-%s", (int)getpid(), basename) < 0)
            return NULL;

        char cmd[4096];
        snprintf(cmd, sizeof(cmd), "curl -fsSL --connect-timeout 30 -o '%s' '%s'",
                 tmp, source);
        log_info("fetching %s ...", source);
        if (system(cmd) != 0) {
            log_error("download failed: %s", source);
            unlink(tmp);
            free(tmp);
            return NULL;
        }
        *needs_cleanup = true;
        return tmp;
    }

    /* Local path — return as-is */
    return xstrdup(source);
}

/* ========== Shared helpers (mirrors jpkg's cmd_install.c / cmd_build.c) ========== */

/* Build a file manifest by walking a directory tree (same logic as cmd_install.c) */
static pkg_file_t *build_file_manifest(const char *root_dir, const char *prefix) {
    pkg_file_t *head = NULL;
    DIR *dir = opendir(root_dir);
    if (!dir) return NULL;

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0)
            continue;

        char *full_path = path_join(root_dir, ent->d_name);
        char *rel_path  = path_join(prefix, ent->d_name);

        struct stat st;
        if (lstat(full_path, &st) != 0) {
            free(full_path);
            free(rel_path);
            continue;
        }

        if (S_ISDIR(st.st_mode)) {
            pkg_file_t *sub = build_file_manifest(full_path, rel_path);
            if (sub) {
                pkg_file_t *tail = sub;
                while (tail->next) tail = tail->next;
                tail->next = head;
                head = sub;
            }
        } else if (S_ISLNK(st.st_mode)) {
            char target[1024];
            ssize_t tlen = readlink(full_path, target, sizeof(target) - 1);
            if (tlen > 0) {
                target[tlen] = '\0';
                pkg_file_t *f = xcalloc(1, sizeof(pkg_file_t));
                f->path        = xstrdup(rel_path);
                f->link_target = xstrdup(target);
                f->mode        = (uint32_t)st.st_mode & 07777;
                memset(f->sha256, '0', 64);
                f->sha256[64]  = '\0';
                f->next = head;
                head = f;
            }
        } else if (S_ISREG(st.st_mode)) {
            pkg_file_t *f = xcalloc(1, sizeof(pkg_file_t));
            f->path = xstrdup(rel_path);
            f->size = (uint64_t)st.st_size;
            f->mode = (uint32_t)st.st_mode & 07777;
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

static void file_manifest_free(pkg_file_t *f) {
    while (f) {
        pkg_file_t *next = f->next;
        free(f->path);
        free(f->link_target);
        free(f);
        f = next;
    }
}

/*
 * Copy extracted staging directory to root filesystem.
 * Flattens usr/ first (jonerix merged-usr layout).
 */
static int install_files(const char *stage_dir, const char *dest_root) {
    char cmd[4096];
    snprintf(cmd, sizeof(cmd),
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cp -a '%s/usr/.' '%s/' && rm -rf '%s/usr'; fi; "
             "cp -a '%s/.' '%s/' 2>/dev/null; true",
             stage_dir, stage_dir,
             stage_dir, stage_dir, stage_dir,
             stage_dir, dest_root);
    return system(cmd);
}

/* ========== Build recipe (mirrors cmd_build.c) ========== */

typedef struct {
    char *name;
    char *version;
    char *license;
    char *description;
    char *arch;
    char *source_url;
    char *source_sha256;
    char **runtime_deps;
    size_t runtime_dep_count;
    char **build_deps;
    size_t build_dep_count;
    char *configure_cmd;
    char *build_cmd;
    char *install_cmd;
} build_recipe_t;

static void recipe_free(build_recipe_t *r) {
    if (!r) return;
    free(r->name);
    free(r->version);
    free(r->license);
    free(r->description);
    free(r->arch);
    free(r->source_url);
    free(r->source_sha256);
    for (size_t i = 0; i < r->runtime_dep_count; i++) free(r->runtime_deps[i]);
    free(r->runtime_deps);
    for (size_t i = 0; i < r->build_dep_count; i++) free(r->build_deps[i]);
    free(r->build_deps);
    free(r->configure_cmd);
    free(r->build_cmd);
    free(r->install_cmd);
    free(r);
}

/*
 * Load a recipe.toml from either:
 *  - a directory path containing recipe.toml
 *  - a direct path to a recipe.toml file
 */
static build_recipe_t *load_recipe(const char *path) {
    char recipe_path[512];

    struct stat st;
    if (stat(path, &st) == 0 && S_ISDIR(st.st_mode)) {
        snprintf(recipe_path, sizeof(recipe_path), "%s/recipe.toml", path);
    } else {
        snprintf(recipe_path, sizeof(recipe_path), "%s", path);
    }

    uint8_t *data;
    ssize_t len = file_read(recipe_path, &data);
    if (len <= 0) {
        log_error("cannot read recipe: %s", recipe_path);
        return NULL;
    }

    char *err = NULL;
    toml_doc_t *doc = toml_parse((const char *)data, &err);
    free(data);
    if (!doc) {
        log_error("failed to parse recipe: %s", err ? err : "unknown error");
        free(err);
        return NULL;
    }

    build_recipe_t *r = xcalloc(1, sizeof(build_recipe_t));
    const char *s;

    if ((s = toml_get_string(doc, "package.name")))        r->name        = xstrdup(s);
    if ((s = toml_get_string(doc, "package.version")))     r->version     = xstrdup(s);
    if ((s = toml_get_string(doc, "package.license")))     r->license     = xstrdup(s);
    if ((s = toml_get_string(doc, "package.description"))) r->description = xstrdup(s);
    if ((s = toml_get_string(doc, "package.arch")))        r->arch        = xstrdup(s);
    if ((s = toml_get_string(doc, "source.url")))          r->source_url  = xstrdup(s);
    if ((s = toml_get_string(doc, "source.sha256")))       r->source_sha256 = xstrdup(s);
    if ((s = toml_get_string(doc, "build.configure")))     r->configure_cmd = xstrdup(s);
    if ((s = toml_get_string(doc, "build.build")))         r->build_cmd   = xstrdup(s);
    if ((s = toml_get_string(doc, "build.install")))       r->install_cmd = xstrdup(s);

    const toml_array_t *arr;
    if ((arr = toml_get_array(doc, "depends.runtime"))) {
        r->runtime_deps = xcalloc(arr->count, sizeof(char *));
        r->runtime_dep_count = arr->count;
        for (size_t i = 0; i < arr->count; i++)
            r->runtime_deps[i] = xstrdup(arr->items[i]);
    }
    if ((arr = toml_get_array(doc, "depends.build"))) {
        r->build_deps = xcalloc(arr->count, sizeof(char *));
        r->build_dep_count = arr->count;
        for (size_t i = 0; i < arr->count; i++)
            r->build_deps[i] = xstrdup(arr->items[i]);
    }

    toml_free(doc);

    if (!r->name || !r->version) {
        log_error("recipe is missing required fields: name, version");
        recipe_free(r);
        return NULL;
    }

    if (r->license && !license_is_permissive(r->license)) {
        log_error("BLOCKED: %s uses license %s — not permitted in jonerix",
                  r->name, r->license);
        recipe_free(r);
        return NULL;
    }

    return r;
}

/*
 * Run a single build step in a shell script (same logic as cmd_build.c).
 * Uses /bin/sh (toybox sh on jonerix). Pre-expands $(nproc) to avoid
 * command substitution issues in minimal shells.
 */
static int run_build_step(const char *step_name, const char *cmd,
                          const char *work_dir, const char *dest_dir) {
    if (!cmd || cmd[0] == '\0') {
        log_debug("skipping %s (no command)", step_name);
        return 0;
    }

    log_info("  %s: %s", step_name, cmd);

    const char *shell = "/bin/sh";

    char script_path[256];
    snprintf(script_path, sizeof(script_path), "/tmp/jpkg-local-build-%d.sh", (int)getpid());

    FILE *sf = fopen(script_path, "w");
    if (!sf) {
        log_error("failed to create build script: %s", strerror(errno));
        return -1;
    }
    fprintf(sf, "#!%s\n", shell);

    /* Export NPROC so $(nproc) can be pre-expanded */
    long ncpu = sysconf(_SC_NPROCESSORS_ONLN);
    if (ncpu < 1) ncpu = 1;
    fprintf(sf, "export NPROC=%ld\n", ncpu);
    fprintf(sf, "nproc() { printf '%%ld\\n' %ld; }\n", ncpu);

    fprintf(sf, "cd '%s'\n", work_dir);
    fprintf(sf, "export CC=clang\nexport LD=ld.lld\n");
    fprintf(sf, "export AR=llvm-ar\nexport NM=llvm-nm\nexport RANLIB=llvm-ranlib\n");
    fprintf(sf, "export CFLAGS='-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2'\n");
    fprintf(sf, "export LDFLAGS='-Wl,-z,relro,-z,now -pie'\n");
    fprintf(sf, "export DESTDIR='%s'\n", dest_dir);
    fprintf(sf, "export C_INCLUDE_PATH=/include\n");
    fprintf(sf, "export LIBRARY_PATH=/lib\n");

    /* Replace $(nproc) with literal CPU count */
    {
        char nproc_str[32];
        snprintf(nproc_str, sizeof(nproc_str), "%ld", ncpu);
        const char *p = cmd;
        while (*p) {
            if (strncmp(p, "$(nproc)", 8) == 0) {
                fputs(nproc_str, sf);
                p += 8;
            } else if (strncmp(p, "`nproc`", 7) == 0) {
                fputs(nproc_str, sf);
                p += 7;
            } else {
                fputc(*p, sf);
                p++;
            }
        }
        fputc('\n', sf);
    }
    fclose(sf);
    chmod(script_path, 0755);

    int rc = system(script_path);
    unlink(script_path);

    if (rc != 0) {
        log_error("%s step failed (exit %d)", step_name, WEXITSTATUS(rc));
        return -1;
    }
    return 0;
}

/* Create a .jpkg archive from a staging DESTDIR and recipe metadata */
static int create_package(const build_recipe_t *recipe, const char *dest_dir,
                          const char *output_dir) {
    /* Flatten usr/ -> / (jonerix merged-usr) */
    char flatten[2048];
    snprintf(flatten, sizeof(flatten),
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cp -a '%s/usr/.' '%s/' && rm -rf '%s/usr'; fi",
             dest_dir, dest_dir, dest_dir, dest_dir, dest_dir);
    system(flatten);

    /* Flatten lib64/ -> lib/ (cmake default on x86_64) */
    char flatten64[2048];
    snprintf(flatten64, sizeof(flatten64),
             "if [ -d '%s/lib64' ] && [ ! -L '%s/lib64' ]; then "
             "cp -a '%s/lib64/.' '%s/lib/' && rm -rf '%s/lib64'; fi",
             dest_dir, dest_dir, dest_dir, dest_dir, dest_dir);
    system(flatten64);

    {
        char problem[1024];
        tree_audit_result_t audit = audit_layout_tree(dest_dir, problem, sizeof(problem));
        if (audit != TREE_AUDIT_OK) {
            log_error("refusing to package %s: %s at %s",
                      recipe->name, audit_layout_result_string(audit),
                      problem[0] ? problem : "(unknown)");
            return -1;
        }
    }

    /* Create zstd tarball of DESTDIR contents */
    char tar_path[512];
    snprintf(tar_path, sizeof(tar_path), "/tmp/jpkg-local-payload-%s-%d.tar.zst",
             recipe->name, (int)getpid());

    char cmd[2048];
    snprintf(cmd, sizeof(cmd), "cd '%s' && tar cf - . | zstd -c > '%s'",
             dest_dir, tar_path);
    if (system(cmd) != 0) {
        log_error("failed to create package tarball");
        return -1;
    }

    uint8_t *payload;
    ssize_t payload_len = file_read(tar_path, &payload);
    unlink(tar_path);
    if (payload_len < 0) {
        log_error("failed to read package payload");
        return -1;
    }

    /* Compute payload hash */
    uint8_t hash[32];
    sha256_hash(payload, (size_t)payload_len, hash);
    char hash_hex[65];
    sha256_hex(hash, hash_hex);

    /* Build TOML metadata */
    toml_doc_t *doc = toml_new();
    toml_set_string(doc, "package.name",    recipe->name);
    toml_set_string(doc, "package.version", recipe->version);
    if (recipe->license)
        toml_set_string(doc, "package.license", recipe->license);
    if (recipe->description)
        toml_set_string(doc, "package.description", recipe->description);

    if (!recipe->arch) {
        struct utsname uts;
        char auto_arch[32] = "x86_64";
        if (uname(&uts) == 0)
            snprintf(auto_arch, sizeof(auto_arch), "%s", uts.machine);
        toml_set_string(doc, "package.arch", auto_arch);
    } else {
        toml_set_string(doc, "package.arch", recipe->arch);
    }

    if (recipe->runtime_dep_count > 0)
        toml_set_array(doc, "depends.runtime",
                       (const char **)recipe->runtime_deps,
                       recipe->runtime_dep_count);
    if (recipe->build_dep_count > 0)
        toml_set_array(doc, "depends.build",
                       (const char **)recipe->build_deps,
                       recipe->build_dep_count);

    char *toml_str = toml_serialize(doc);
    toml_free(doc);

    /* Append payload hash/size to metadata */
    size_t toml_base_len = strlen(toml_str);
    char extra[256];
    int elen = snprintf(extra, sizeof(extra),
                        "\n[files]\nsha256 = \"%s\"\nsize = %zd\n",
                        hash_hex, (ssize_t)payload_len);
    toml_str = xrealloc(toml_str, toml_base_len + (size_t)elen + 1);
    memcpy(toml_str + toml_base_len, extra, (size_t)elen + 1);

    /* Ensure output directory exists */
    mkdirs(output_dir, 0755);

    char *filename    = pkg_filename(recipe->name, recipe->version);
    char *output_path = path_join(output_dir, filename);

    int rc = pkg_create(output_path, toml_str, payload, (size_t)payload_len);
    if (rc == 0) {
        log_info("package created: %s", output_path);
        struct stat st;
        if (stat(output_path, &st) == 0) {
            if (st.st_size >= 1048576)
                log_info("  size: %.1f MiB", (double)st.st_size / 1048576.0);
            else
                log_info("  size: %.1f KiB", (double)st.st_size / 1024.0);
        }
    }

    free(toml_str);
    free(payload);
    free(filename);
    free(output_path);
    return rc;
}

/* ========== jpkg-local install ========== */

static void usage_install(void) {
    fprintf(stderr, "usage: jpkg-local install <file.jpkg|url|-> [--root <dir>]\n");
}

static int cmd_local_install(int argc, char **argv) {
    if (argc < 1) {
        usage_install();
        return 1;
    }

    const char *source = NULL;

    for (int i = 0; i < argc; i++) {
        if ((strcmp(argv[i], "--root") == 0 || strcmp(argv[i], "-r") == 0)
            && i + 1 < argc) {
            set_rootfs(argv[++i]);
        } else if (argv[i][0] != '-' || strcmp(argv[i], "-") == 0) {
            source = argv[i];
        } else {
            fprintf(stderr, "jpkg-local install: unknown option: %s\n", argv[i]);
            usage_install();
            return 1;
        }
    }

    if (!source) {
        usage_install();
        return 1;
    }

    /* Resolve source to a local file (fetch URL or read stdin if needed) */
    bool needs_cleanup = false;
    char *pkg_path = fetch_to_tmp(source, "package.jpkg", &needs_cleanup);
    if (!pkg_path) return 1;

    /* Parse the .jpkg file */
    size_t payload_off, payload_len;
    pkg_meta_t *meta = pkg_parse_file(pkg_path, &payload_off, &payload_len);
    if (!meta) {
        log_error("failed to parse package: %s", pkg_path);
        if (needs_cleanup) { unlink(pkg_path); free(pkg_path); }
        return 1;
    }

    log_info("installing %s-%s from %s...", meta->name, meta->version, source);

    /* Warn if runtime dependencies are not installed */
    jpkg_db_t *db = db_open();
    if (db) {
        db_load(db);
        for (size_t i = 0; i < meta->runtime_dep_count; i++) {
            if (!db_is_installed(db, meta->runtime_deps[i])) {
                log_warn("runtime dependency '%s' is not installed",
                         meta->runtime_deps[i]);
            }
        }
    }

    /* Extract to staging directory */
    char stage_dir[256];
    snprintf(stage_dir, sizeof(stage_dir), "/tmp/jpkg-local-stage-%s-%d",
             meta->name, (int)getpid());

    int rc = pkg_extract(pkg_path, stage_dir);
    if (rc != 0) {
        log_error("failed to extract %s", meta->name);
        if (db) db_close(db);
        pkg_meta_free(meta);
        if (needs_cleanup) { unlink(pkg_path); free(pkg_path); }
        return 1;
    }

    {
        char problem[1024];
        tree_audit_result_t audit = audit_layout_tree(stage_dir, problem, sizeof(problem));
        if (audit != TREE_AUDIT_OK) {
            log_error("refusing to install %s: %s at %s",
                      meta->name, audit_layout_result_string(audit),
                      problem[0] ? problem : "(unknown)");
            char rmcmd[512];
            snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", stage_dir);
            system(rmcmd);
            if (db) db_close(db);
            pkg_meta_free(meta);
            if (needs_cleanup) { unlink(pkg_path); free(pkg_path); }
            return 1;
        }
    }

    /* Build file manifest from extracted files */
    pkg_file_t *files = build_file_manifest(stage_dir, "/");

    /* Copy files to root filesystem */
    const char *dest_root = (g_rootfs && g_rootfs[0]) ? g_rootfs : "/";
    rc = install_files(stage_dir, dest_root);
    if (rc != 0) {
        log_error("failed to install files for %s", meta->name);
        char rmcmd[512];
        snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", stage_dir);
        system(rmcmd);
        file_manifest_free(files);
        if (db) db_close(db);
        pkg_meta_free(meta);
        if (needs_cleanup) { unlink(pkg_path); free(pkg_path); }
        return 1;
    }

    /* Register in the package database */
    if (db) {
        /* Remove any previous registration first */
        if (db_is_installed(db, meta->name))
            db_unregister(db, meta->name);
        db_register(db, meta, files);
        db_close(db);
    } else {
        log_warn("could not open package database — not registering %s", meta->name);
    }

    /* Clean up staging directory */
    char rmcmd[512];
    snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", stage_dir);
    system(rmcmd);

    file_manifest_free(files);
    log_info("installed %s-%s", meta->name, meta->version);
    pkg_meta_free(meta);
    if (needs_cleanup) { unlink(pkg_path); free(pkg_path); }
    return 0;
}

/* ========== jpkg-local build ========== */

static void usage_build(void) {
    fprintf(stderr,
            "usage: jpkg-local build <recipe-dir|recipe.toml|url|-> "
            "[--output <dir>] [--build-jpkg]\n");
}

static int cmd_local_build(int argc, char **argv) {
    if (argc < 1) {
        usage_build();
        return 1;
    }

    const char *source = NULL;
    const char *output_dir  = ".";
    bool build_jpkg = false;

    for (int i = 0; i < argc; i++) {
        if (strcmp(argv[i], "--build-jpkg") == 0) {
            build_jpkg = true;
        } else if ((strcmp(argv[i], "--output") == 0 || strcmp(argv[i], "-o") == 0)
                   && i + 1 < argc) {
            output_dir = argv[++i];
        } else if (argv[i][0] != '-' || strcmp(argv[i], "-") == 0) {
            source = argv[i];
        } else {
            fprintf(stderr, "jpkg-local build: unknown option: %s\n", argv[i]);
            usage_build();
            return 1;
        }
    }

    if (!source) {
        usage_build();
        return 1;
    }

    /* Resolve source: URL or stdin → fetch to a temp recipe.toml */
    bool needs_cleanup = false;
    char *recipe_path = NULL;
    char recipe_tmpdir[256] = {0};

    if (is_url(source) || strcmp(source, "-") == 0) {
        char *fetched = fetch_to_tmp(source, "recipe.toml", &needs_cleanup);
        if (!fetched) return 1;
        /* Place in a temp directory so load_recipe's dir-based logic works
         * and the recipe can reference patches/ relative to itself */
        snprintf(recipe_tmpdir, sizeof(recipe_tmpdir),
                 "/tmp/jpkg-local-recipe-%d", (int)getpid());
        mkdirs(recipe_tmpdir, 0755);
        char dest[512];
        snprintf(dest, sizeof(dest), "%s/recipe.toml", recipe_tmpdir);
        rename(fetched, dest);
        if (needs_cleanup) free(fetched);
        recipe_path = xstrdup(recipe_tmpdir);
        needs_cleanup = true;
    } else {
        recipe_path = xstrdup(source);
    }

    build_recipe_t *recipe = load_recipe(recipe_path);
    if (!recipe) {
        if (needs_cleanup && recipe_tmpdir[0]) {
            char rmcmd[512];
            snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", recipe_tmpdir);
            system(rmcmd);
        }
        free(recipe_path);
        return 1;
    }

    log_info("building %s-%s...", recipe->name, recipe->version);

    /* Determine recipe directory for patches etc. */
    char recipe_dir[512];
    struct stat st;
    if (stat(recipe_path, &st) == 0 && S_ISDIR(st.st_mode)) {
        snprintf(recipe_dir, sizeof(recipe_dir), "%s", recipe_path);
    } else {
        /* recipe_path is a file — parent directory is the recipe dir */
        const char *slash = strrchr(recipe_path, '/');
        if (slash) {
            size_t dlen = (size_t)(slash - recipe_path);
            if (dlen >= sizeof(recipe_dir)) dlen = sizeof(recipe_dir) - 1;
            memcpy(recipe_dir, recipe_path, dlen);
            recipe_dir[dlen] = '\0';
        } else {
            snprintf(recipe_dir, sizeof(recipe_dir), ".");
        }
    }

    /* Create working directories */
    char work_dir[256], src_dir[256], dest_dir[256];
    snprintf(work_dir, sizeof(work_dir), "/tmp/jpkg-local-build-%s-%d",
             recipe->name, (int)getpid());
    snprintf(src_dir,  sizeof(src_dir),  "%s/src",  work_dir);
    snprintf(dest_dir, sizeof(dest_dir), "%s/dest", work_dir);

    mkdirs(work_dir, 0755);
    mkdirs(src_dir,  0755);
    mkdirs(dest_dir, 0755);

    int rc = 0;

    /* If there's a source URL, download and extract it */
    if (recipe->source_url && recipe->source_url[0]
        && strcmp(recipe->source_url, "local") != 0) {
        const char *basename = strrchr(recipe->source_url, '/');
        basename = basename ? basename + 1 : recipe->source_url;
        char tarball[512];
        snprintf(tarball, sizeof(tarball), "%s/%s", src_dir, basename);

        char dlcmd[2048];
        snprintf(dlcmd, sizeof(dlcmd), "curl -fsSL -o '%s' '%s'",
                 tarball, recipe->source_url);
        log_info("  downloading: %s", recipe->source_url);
        if (system(dlcmd) != 0) {
            log_error("download failed");
            rc = -1;
            goto cleanup;
        }

        /* Verify SHA256 if provided */
        if (recipe->source_sha256 && recipe->source_sha256[0]) {
            char got[65];
            if (sha256_file(tarball, got) != 0 ||
                strcmp(got, recipe->source_sha256) != 0) {
                log_error("source hash mismatch (expected %s)", recipe->source_sha256);
                rc = -1;
                goto cleanup;
            }
            log_info("  source hash verified");
        }

        char extcmd[1024];
        snprintf(extcmd, sizeof(extcmd), "cd '%s' && tar xf '%s'", src_dir, tarball);
        if (system(extcmd) != 0) {
            log_error("failed to extract source");
            rc = -1;
            goto cleanup;
        }

        /* Move into a single extracted subdirectory if present */
        DIR *d = opendir(src_dir);
        if (d) {
            struct dirent *ent2;
            char only_subdir[256] = {0};
            int dir_count = 0;
            while ((ent2 = readdir(d)) != NULL) {
                if (ent2->d_name[0] == '.') continue;
                char cp[512];
                snprintf(cp, sizeof(cp), "%s/%s", src_dir, ent2->d_name);
                if (dir_exists(cp)) {
                    dir_count++;
                    if (dir_count == 1)
                        snprintf(only_subdir, sizeof(only_subdir), "%s", ent2->d_name);
                }
            }
            closedir(d);
            if (dir_count == 1 && only_subdir[0]) {
                char new_src[512];
                snprintf(new_src, sizeof(new_src), "%s/%s", src_dir, only_subdir);
                snprintf(src_dir, sizeof(src_dir), "%s", new_src);
            }
        }

        /* Apply patches if any */
        char patches_dir[512];
        snprintf(patches_dir, sizeof(patches_dir), "%s/patches", recipe_dir);
        if (dir_exists(patches_dir)) {
            DIR *pd = opendir(patches_dir);
            if (pd) {
                struct dirent *ent2;
                while ((ent2 = readdir(pd)) != NULL) {
                    const char *n = ent2->d_name;
                    size_t nlen = strlen(n);
                    bool is_patch = (nlen > 6  && strcmp(n + nlen - 6,  ".patch") == 0) ||
                                    (nlen > 5  && strcmp(n + nlen - 5,  ".diff")  == 0);
                    if (!is_patch) continue;
                    char pp[768];
                    snprintf(pp, sizeof(pp), "%s/%s", patches_dir, n);
                    log_info("  applying patch: %s", n);
                    char pcmd[1024];
                    snprintf(pcmd, sizeof(pcmd), "cd '%s' && patch -p1 < '%s'",
                             src_dir, pp);
                    if (system(pcmd) != 0) {
                        log_error("failed to apply patch: %s", n);
                        closedir(pd);
                        rc = -1;
                        goto cleanup;
                    }
                }
                closedir(pd);
            }
        }
    }

    /* Configure */
    rc = run_build_step("configure", recipe->configure_cmd, src_dir, dest_dir);
    if (rc != 0) goto cleanup;

    /* Build */
    rc = run_build_step("build", recipe->build_cmd, src_dir, dest_dir);
    if (rc != 0) goto cleanup;

    if (build_jpkg) {
        /* Install to staging, then package */
        rc = run_build_step("install", recipe->install_cmd, src_dir, dest_dir);
        if (rc != 0) goto cleanup;
        rc = create_package(recipe, dest_dir, output_dir);
    } else {
        /* Install directly to the system rootfs */
        const char *real_dest = (g_rootfs && g_rootfs[0]) ? g_rootfs : "/";
        rc = run_build_step("install", recipe->install_cmd, src_dir,
                            (char *)real_dest);
        if (rc != 0) goto cleanup;

        /* Register in the package database */
        jpkg_db_t *db = db_open();
        if (db) {
            pkg_meta_t meta = {0};
            meta.name        = recipe->name;
            meta.version     = recipe->version;
            meta.license     = recipe->license;
            meta.description = recipe->description;
            meta.arch        = recipe->arch;
            db_register(db, &meta, NULL);
            db_close(db);
            log_info("registered %s-%s in package database",
                     recipe->name, recipe->version);
        }
    }

cleanup:
    {
        char rmcmd[512];
        snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", work_dir);
        system(rmcmd);
    }
    if (needs_cleanup && recipe_tmpdir[0]) {
        char rmcmd[512];
        snprintf(rmcmd, sizeof(rmcmd), "rm -rf '%s'", recipe_tmpdir);
        system(rmcmd);
    }
    free(recipe_path);

    if (rc == 0)
        log_info("build complete: %s-%s", recipe->name, recipe->version);
    else
        log_error("build failed: %s-%s", recipe->name, recipe->version);

    recipe_free(recipe);
    return rc == 0 ? 0 : 1;
}

/* ========== Main ========== */

static void print_usage(void) {
    printf("jpkg-local - install .jpkg files and build recipes from any source\n");
    printf("MIT License - Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software\n\n");
    printf("Usage:\n");
    printf("  jpkg local install <file.jpkg|url|-> [--root <dir>]\n");
    printf("  jpkg local build   <recipe-dir|recipe.toml|url|-> [--output <dir>] [--build-jpkg]\n");
    printf("\nSources:\n");
    printf("  /path/to/file     Local file or directory\n");
    printf("  https://...       HTTP/HTTPS URL (fetched via curl)\n");
    printf("  -                 Read from stdin (pipe)\n");
    printf("\nExamples:\n");
    printf("  jpkg local install ./mypackage.jpkg\n");
    printf("  jpkg local install https://example.com/pkg-1.0-aarch64.jpkg\n");
    printf("  curl -fsSL https://example.com/recipe.toml | jpkg local build -\n");
    printf("\nOptions:\n");
    printf("  -v, --verbose   Increase verbosity\n");
    printf("  -q, --quiet     Suppress non-error output\n");
    printf("  -h, --help      Show this help\n");
}

int main(int argc, char **argv) {
    if (argc < 2) {
        print_usage();
        return 1;
    }

    int cmd_idx = 1;
    while (cmd_idx < argc && argv[cmd_idx][0] == '-') {
        if (strcmp(argv[cmd_idx], "-v") == 0 || strcmp(argv[cmd_idx], "--verbose") == 0) {
            log_set_level(LOG_DEBUG);
            cmd_idx++;
        } else if (strcmp(argv[cmd_idx], "-q") == 0 || strcmp(argv[cmd_idx], "--quiet") == 0) {
            log_set_level(LOG_ERROR);
            cmd_idx++;
        } else if (strcmp(argv[cmd_idx], "-h") == 0 || strcmp(argv[cmd_idx], "--help") == 0) {
            print_usage();
            return 0;
        } else {
            break;
        }
    }

    if (cmd_idx >= argc) {
        print_usage();
        return 1;
    }

    const char *subcmd = argv[cmd_idx];
    int sub_argc = argc - cmd_idx - 1;
    char **sub_argv = argv + cmd_idx + 1;

    if (strcmp(subcmd, "install") == 0) {
        return cmd_local_install(sub_argc, sub_argv);
    } else if (strcmp(subcmd, "build") == 0) {
        return cmd_local_build(sub_argc, sub_argv);
    } else {
        fprintf(stderr, "jpkg-local: unknown subcommand: %s\n", subcmd);
        fprintf(stderr, "Run 'jpkg-local --help' for usage.\n");
        return 1;
    }
}
