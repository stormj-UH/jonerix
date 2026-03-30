/*
 * jpkg - jonerix package manager
 * cmd_build.c - jpkg build / build-world: build packages from source recipes
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "pkg.h"
#include "repo.h"
#include "fetch.h"
#include "db.h"
#include "toml.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <sys/utsname.h>
#include <dirent.h>

/* Build recipe is a directory containing:
 *   recipe.toml   - package metadata + build instructions
 *   patches/      - optional patches
 */

typedef struct build_recipe {
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

static build_recipe_t *load_recipe(const char *recipe_dir) {
    char recipe_path[512];
    snprintf(recipe_path, sizeof(recipe_path), "%s/recipe.toml", recipe_dir);

    uint8_t *data;
    ssize_t len = file_read(recipe_path, &data);
    if (len <= 0) {
        /* Try Makefile-style recipe */
        snprintf(recipe_path, sizeof(recipe_path), "%s/Makefile", recipe_dir);
        if (!file_exists(recipe_path)) {
            log_error("no recipe.toml or Makefile found in %s", recipe_dir);
            return NULL;
        }
        /* For Makefile-based recipes, create a minimal recipe */
        build_recipe_t *r = xcalloc(1, sizeof(build_recipe_t));
        /* Extract package name from directory name */
        const char *base = strrchr(recipe_dir, '/');
        r->name = xstrdup(base ? base + 1 : recipe_dir);
        r->version = xstrdup("0.0.0");
        r->license = xstrdup("unknown");
        r->build_cmd = xstrdup("make");
        r->install_cmd = xstrdup("make install DESTDIR=$DESTDIR");
        return r;
    }

    char *err = NULL;
    toml_doc_t *doc = toml_parse((const char *)data, &err);
    free(data);

    if (!doc) {
        log_error("failed to parse recipe: %s", err ? err : "?");
        free(err);
        return NULL;
    }

    build_recipe_t *r = xcalloc(1, sizeof(build_recipe_t));

    const char *s;
    if ((s = toml_get_string(doc, "package.name")))
        r->name = xstrdup(s);
    if ((s = toml_get_string(doc, "package.version")))
        r->version = xstrdup(s);
    if ((s = toml_get_string(doc, "package.license")))
        r->license = xstrdup(s);
    if ((s = toml_get_string(doc, "package.description")))
        r->description = xstrdup(s);
    if ((s = toml_get_string(doc, "package.arch")))
        r->arch = xstrdup(s);

    if ((s = toml_get_string(doc, "source.url")))
        r->source_url = xstrdup(s);
    if ((s = toml_get_string(doc, "source.sha256")))
        r->source_sha256 = xstrdup(s);

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

    if ((s = toml_get_string(doc, "build.configure")))
        r->configure_cmd = xstrdup(s);
    if ((s = toml_get_string(doc, "build.build")))
        r->build_cmd = xstrdup(s);
    if ((s = toml_get_string(doc, "build.install")))
        r->install_cmd = xstrdup(s);

    toml_free(doc);

    /* Validate required fields */
    if (!r->name || !r->version) {
        log_error("recipe missing required fields (name, version)");
        recipe_free(r);
        return NULL;
    }

    /* License gate: reject GPL packages */
    if (r->license && !license_is_permissive(r->license)) {
        log_error("BLOCKED: %s is licensed under %s - not permitted in jonerix",
                  r->name, r->license);
        recipe_free(r);
        return NULL;
    }

    return r;
}

static int run_build_step(const char *step_name, const char *cmd,
                          const char *work_dir, const char *dest_dir) {
    if (!cmd || cmd[0] == '\0') {
        log_debug("skipping %s (no command)", step_name);
        return 0;
    }

    log_info("  %s: %s", step_name, cmd);

    /* Set up environment.
     * Use C_INCLUDE_PATH / LIBRARY_PATH instead of -I/-L in CFLAGS/LDFLAGS.
     * This is additive — recipes that set their own CFLAGS won't lose the
     * include paths. Only add /include (jonerix merged-usr), not /usr/include
     * which may contain Alpine fortify wrapper headers that cause circular
     * #include_next failures. */
    char env_cc[64] = "CC=clang";
    char env_ld[64] = "LD=ld.lld";
    char env_ar[64] = "AR=llvm-ar";
    char env_nm[64] = "NM=llvm-nm";
    char env_ranlib[64] = "RANLIB=llvm-ranlib";
    char env_cflags[256];
    snprintf(env_cflags, sizeof(env_cflags),
             "CFLAGS=-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2");
    char env_ldflags[256];
    snprintf(env_ldflags, sizeof(env_ldflags),
             "LDFLAGS=-Wl,-z,relro,-z,now -pie");
    char env_destdir[512];
    snprintf(env_destdir, sizeof(env_destdir), "DESTDIR=%s", dest_dir);
    char env_cinclude[128] = "C_INCLUDE_PATH=/include";
    char env_libpath[128] = "LIBRARY_PATH=/lib";

    /* Build the full command with environment and working directory */
    char full_cmd[4096];
    snprintf(full_cmd, sizeof(full_cmd),
             "cd '%s' && export %s && export %s && export %s && export %s && "
             "export %s && export '%s' && export '%s' && "
             "export '%s' && export %s && export %s && %s",
             work_dir, env_cc, env_ld, env_ar, env_nm, env_ranlib,
             env_cflags, env_ldflags, env_destdir,
             env_cinclude, env_libpath, cmd);

    int rc = system(full_cmd);
    if (rc != 0) {
        log_error("%s failed (exit %d)", step_name, WEXITSTATUS(rc));
        return -1;
    }

    return 0;
}

static int fetch_source(const build_recipe_t *recipe, const char *work_dir) {
    if (!recipe->source_url) {
        log_debug("no source URL, assuming local build");
        return 0;
    }

    char tarball[512];
    const char *basename = strrchr(recipe->source_url, '/');
    basename = basename ? basename + 1 : recipe->source_url;
    snprintf(tarball, sizeof(tarball), "%s/%s", work_dir, basename);

    /* Download */
    char cmd[2048];
    snprintf(cmd, sizeof(cmd), "curl -fsSL -o '%s' '%s'", tarball, recipe->source_url);
    log_info("  downloading source: %s", recipe->source_url);
    if (system(cmd) != 0) {
        log_error("failed to download source");
        return -1;
    }

    /* Verify SHA256 if provided */
    if (recipe->source_sha256 && recipe->source_sha256[0]) {
        char hash[65];
        if (sha256_file(tarball, hash) != 0) {
            log_error("failed to hash source tarball");
            return -1;
        }
        if (strcmp(hash, recipe->source_sha256) != 0) {
            log_error("source hash mismatch:");
            log_error("  expected: %s", recipe->source_sha256);
            log_error("  actual:   %s", hash);
            return -1;
        }
        log_info("  source hash verified");
    }

    /* Extract */
    snprintf(cmd, sizeof(cmd), "cd '%s' && tar xf '%s'", work_dir, tarball);
    if (system(cmd) != 0) {
        log_error("failed to extract source");
        return -1;
    }

    return 0;
}

static int apply_patches(const char *recipe_dir, const char *src_dir) {
    char patches_dir[512];
    snprintf(patches_dir, sizeof(patches_dir), "%s/patches", recipe_dir);

    if (!dir_exists(patches_dir)) return 0;

    DIR *dir = opendir(patches_dir);
    if (!dir) return 0;

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (!str_ends_with(ent->d_name, ".patch") &&
            !str_ends_with(ent->d_name, ".diff"))
            continue;

        char patch_path[768];
        snprintf(patch_path, sizeof(patch_path), "%s/%s", patches_dir, ent->d_name);

        log_info("  applying patch: %s", ent->d_name);

        char cmd[1024];
        snprintf(cmd, sizeof(cmd), "cd '%s' && patch -p1 < '%s'", src_dir, patch_path);
        if (system(cmd) != 0) {
            log_error("failed to apply patch: %s", ent->d_name);
            closedir(dir);
            return -1;
        }
    }

    closedir(dir);
    return 0;
}

static int create_package(const build_recipe_t *recipe, const char *dest_dir,
                          const char *output_dir) {
    /* Build TOML metadata */
    toml_doc_t *doc = toml_new();
    toml_set_string(doc, "package.name", recipe->name);
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
                       (const char **)recipe->runtime_deps, recipe->runtime_dep_count);
    if (recipe->build_dep_count > 0)
        toml_set_array(doc, "depends.build",
                       (const char **)recipe->build_deps, recipe->build_dep_count);

    char *toml_str = toml_serialize(doc);
    toml_free(doc);

    /* Flatten usr/ in dest_dir before packaging.
     * Many build systems install to usr/bin/, usr/lib/ etc. even with
     * --prefix=/. On jonerix merged-usr layout, these must be at
     * /bin/, /lib/ directly. Flatten here so ALL jpkg archives are
     * consistent regardless of build system behavior. */
    char flatten[2048];
    snprintf(flatten, sizeof(flatten),
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cp -a '%s/usr/.' '%s/' && rm -rf '%s/usr'; fi",
             dest_dir, dest_dir, dest_dir, dest_dir, dest_dir);
    system(flatten);

    /* Create zstd-compressed tarball of dest_dir contents */
    char tar_path[512];
    snprintf(tar_path, sizeof(tar_path), "/tmp/jpkg-build-%s-%d.tar.zst",
             recipe->name, (int)getpid());

    char cmd[2048];
    snprintf(cmd, sizeof(cmd), "cd '%s' && tar cf - . | zstd -c > '%s'",
             dest_dir, tar_path);
    if (system(cmd) != 0) {
        log_error("failed to create package tarball");
        free(toml_str);
        return -1;
    }

    /* Read zstd payload */
    uint8_t *payload;
    ssize_t payload_len = file_read(tar_path, &payload);
    unlink(tar_path);

    if (payload_len < 0) {
        log_error("failed to read package payload");
        free(toml_str);
        return -1;
    }

    /* Compute payload hash */
    uint8_t hash[32];
    sha256_hash(payload, (size_t)payload_len, hash);
    char hash_hex[65];
    sha256_hex(hash, hash_hex);

    /* Add hash and size to metadata */
    /* We need to re-serialize with the hash - quick append */
    size_t toml_len = strlen(toml_str);
    char extra[256];
    int elen = snprintf(extra, sizeof(extra),
                        "\n[files]\nsha256 = \"%s\"\nsize = %zd\n",
                        hash_hex, payload_len);
    toml_str = xrealloc(toml_str, toml_len + (size_t)elen + 1);
    memcpy(toml_str + toml_len, extra, (size_t)elen + 1);

    /* Create output directory */
    mkdirs(output_dir, 0755);

    /* Output path */
    char *filename = pkg_filename(recipe->name, recipe->version);
    char *output_path = path_join(output_dir, filename);

    /* Create the .jpkg file */
    int rc = pkg_create(output_path, toml_str, payload, (size_t)payload_len);

    if (rc == 0) {
        log_info("package created: %s", output_path);

        /* Show package size */
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

/*
 * Fetch a recipe.toml from the package repository for a given package name.
 * Downloads from <mirror>/recipes/<name>/recipe.toml
 * Returns a temporary directory containing the recipe, or NULL on failure.
 */
static char *fetch_remote_recipe(const char *pkg_name) {
    repo_config_t *cfg = repo_config_load();
    if (!cfg || !cfg->mirrors) {
        log_error("no repository mirrors configured");
        repo_config_free(cfg);
        return NULL;
    }

    /* Create temp directory for the recipe */
    char *recipe_dir = xstrdup("/tmp/jpkg-recipe-XXXXXX");
    if (!mkdtemp(recipe_dir)) {
        log_error("failed to create temp directory");
        free(recipe_dir);
        repo_config_free(cfg);
        return NULL;
    }

    char recipe_path[512];
    snprintf(recipe_path, sizeof(recipe_path), "%s/recipe.toml", recipe_dir);

    /* Try each mirror — look for recipes/<name>/recipe.toml */
    bool fetched = false;
    for (repo_mirror_t *m = cfg->mirrors; m && !fetched; m = m->next) {
        if (!m->enabled) continue;

        char url[2048];
        /* Try GitHub raw content URL pattern for stormj-UH/jonerix */
        snprintf(url, sizeof(url),
                 "https://raw.githubusercontent.com/stormj-UH/jonerix/master/"
                 "packages/core/%s/recipe.toml", pkg_name);

        log_info("fetching recipe for %s...", pkg_name);
        if (fetch_to_file(url, recipe_path) == 0) {
            fetched = true;
        } else {
            /* Fallback: try <mirror>/recipes/<name>.toml */
            snprintf(url, sizeof(url), "%s/recipes/%s.toml", m->url, pkg_name);
            if (fetch_to_file(url, recipe_path) == 0) {
                fetched = true;
            }
        }
    }

    repo_config_free(cfg);

    if (!fetched) {
        log_error("no recipe found for '%s' in any mirror", pkg_name);
        char cmd[512];
        snprintf(cmd, sizeof(cmd), "rm -rf '%s'", recipe_dir);
        system(cmd);
        free(recipe_dir);
        return NULL;
    }

    log_info("recipe downloaded for %s", pkg_name);
    return recipe_dir;
}

static bool tool_in_path(const char *name) {
    static const char *dirs[] = {"/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin", NULL};
    for (int i = 0; dirs[i]; i++) {
        char p[512];
        snprintf(p, sizeof(p), "%s/%s", dirs[i], name);
        if (access(p, X_OK) == 0) return true;
    }
    return false;
}

int cmd_build(int argc, char **argv) {
    if (argc < 1) {
        fprintf(stderr, "usage: jpkg build <recipe-dir-or-package-name> [--build-jpkg] [--output <dir>]\n");
        return 1;
    }

    const char *arg = argv[0];
    const char *output_dir = ".";
    bool build_jpkg = false;
    char *fetched_recipe_dir = NULL;

    /* Parse options */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--build-jpkg") == 0) {
            build_jpkg = true;
        } else if ((strcmp(argv[i], "--output") == 0 || strcmp(argv[i], "-o") == 0) &&
            i + 1 < argc) {
            output_dir = argv[++i];
        }
    }

    const char *recipe_dir;

    if (dir_exists(arg)) {
        /* Argument is a local directory */
        recipe_dir = arg;
    } else {
        /* Argument is a package name — fetch recipe from repo */
        fetched_recipe_dir = fetch_remote_recipe(arg);
        if (!fetched_recipe_dir) return 1;
        recipe_dir = fetched_recipe_dir;
    }

    /* Load recipe */
    build_recipe_t *recipe = load_recipe(recipe_dir);
    if (!recipe) {
        if (fetched_recipe_dir) {
            char cmd[512];
            snprintf(cmd, sizeof(cmd), "rm -rf '%s'", fetched_recipe_dir);
            system(cmd);
            free(fetched_recipe_dir);
        }
        return 1;
    }

    log_info("building %s-%s...", recipe->name, recipe->version);

    /* Pre-flight: check that required build tools are available.
     * Only require clang for recipes that actually use make/configure.
     * Custom recipes (e.g. Rust) download their own toolchain. */
    {
        bool needs_cc = (recipe->configure_cmd && recipe->configure_cmd[0]) ||
                        (recipe->build_cmd &&
                            (strstr(recipe->build_cmd, "make") ||
                             strstr(recipe->build_cmd, "cmake") ||
                             strstr(recipe->build_cmd, "./configure")));
        if (needs_cc && !tool_in_path("clang")) {
            log_error("build requires 'clang' — install it with: jpkg install llvm");
            recipe_free(recipe);
            if (fetched_recipe_dir) {
                char cmd[512];
                snprintf(cmd, sizeof(cmd), "rm -rf '%s'", fetched_recipe_dir);
                system(cmd);
                free(fetched_recipe_dir);
            }
            return 1;
        }

        /* Warn if declared build dependencies are not installed.
         * We check the jpkg db and PATH (for tool packages like cmake).
         * Library packages (ncurses, openssl) may be installed by the base OS
         * without a matching binary, so this is a warning, not a hard error. */
        jpkg_db_t *db_chk = db_open();
        for (size_t i = 0; i < recipe->build_dep_count; i++) {
            const char *dep = recipe->build_deps[i];
            bool installed = (db_chk && db_is_installed(db_chk, dep)) ||
                             tool_in_path(dep);
            if (!installed) {
                log_warn("build dependency '%s' not found via jpkg"
                         " — install it with: jpkg install %s", dep, dep);
            }
        }
        if (db_chk) db_close(db_chk);
    }

    /* Create working directories */
    char work_dir[256], src_dir[256], dest_dir[256];
    snprintf(work_dir, sizeof(work_dir), "/tmp/jpkg-build-%s-%d",
             recipe->name, (int)getpid());
    snprintf(src_dir, sizeof(src_dir), "%s/src", work_dir);
    snprintf(dest_dir, sizeof(dest_dir), "%s/dest", work_dir);

    mkdirs(work_dir, 0755);
    mkdirs(src_dir, 0755);
    mkdirs(dest_dir, 0755);

    int rc;

    /* Step 1: Fetch source */
    rc = fetch_source(recipe, src_dir);
    if (rc != 0) goto cleanup;

    /* Step 2: Find the actual source directory (tarball may extract to a subdir) */
    /* Look for a single subdirectory inside src_dir that contains the source */
    {
        DIR *d = opendir(src_dir);
        if (d) {
            struct dirent *ent;
            char only_subdir[256] = {0};
            int dir_count = 0;
            while ((ent = readdir(d)) != NULL) {
                if (ent->d_name[0] == '.') continue;
                char check_path[512];
                snprintf(check_path, sizeof(check_path), "%s/%s", src_dir, ent->d_name);
                if (dir_exists(check_path)) {
                    dir_count++;
                    if (dir_count == 1)
                        strncpy(only_subdir, ent->d_name, sizeof(only_subdir) - 1);
                }
            }
            closedir(d);
            if (dir_count == 1 && only_subdir[0]) {
                char new_src[512];
                snprintf(new_src, sizeof(new_src), "%s/%s", src_dir, only_subdir);
                strncpy(src_dir, new_src, sizeof(src_dir) - 1);
                src_dir[sizeof(src_dir) - 1] = '\0';
                log_info("  source directory: %s", src_dir);
            }
        }
    }

    /* Step 3: Apply patches */
    rc = apply_patches(recipe_dir, src_dir);
    if (rc != 0) goto cleanup;

    /* Step 4: Configure */
    rc = run_build_step("configure", recipe->configure_cmd, src_dir, dest_dir);
    if (rc != 0) goto cleanup;

    /* Step 5: Build */
    rc = run_build_step("build", recipe->build_cmd, src_dir, dest_dir);
    if (rc != 0) goto cleanup;

    if (build_jpkg) {
        /* Step 6: Install to staging directory */
        rc = run_build_step("install", recipe->install_cmd, src_dir, dest_dir);
        if (rc != 0) goto cleanup;

        /* Step 7: Create .jpkg package */
        rc = create_package(recipe, dest_dir, output_dir);
    } else {
        /* Step 6: Install directly to the system rootfs */
        char real_dest[512];
        snprintf(real_dest, sizeof(real_dest), "%s", g_rootfs[0] ? g_rootfs : "");
        rc = run_build_step("install", recipe->install_cmd, src_dir,
                            real_dest[0] ? real_dest : "/");
        if (rc != 0) goto cleanup;

        /* Register in the package database */
        {
            pkg_meta_t meta = {0};
            meta.name = recipe->name;
            meta.version = recipe->version;
            meta.license = recipe->license;
            meta.description = recipe->description;
            meta.arch = recipe->arch;
            jpkg_db_t *db = db_open();
            if (db) {
                db_register(db, &meta, NULL);
                db_close(db);
                log_info("registered %s-%s in package database",
                         recipe->name, recipe->version);
            }
        }
    }

cleanup:
    /* Clean up working directory */
    {
        char cmd[512];
        snprintf(cmd, sizeof(cmd), "rm -rf '%s'", work_dir);
        system(cmd);
    }

    if (rc == 0) {
        log_info("build complete: %s-%s", recipe->name, recipe->version);
    } else {
        log_error("build failed: %s-%s", recipe->name, recipe->version);
    }

    /* Clean up fetched recipe directory */
    if (fetched_recipe_dir) {
        char cmd2[512];
        snprintf(cmd2, sizeof(cmd2), "rm -rf '%s'", fetched_recipe_dir);
        system(cmd2);
        free(fetched_recipe_dir);
    }

    recipe_free(recipe);
    return rc == 0 ? 0 : 1;
}

/* ========== build-world ========== */

/*
 * Build-world rebuilds the entire jonerix system from source.
 * It looks for recipes in /packages/core/ (or a specified directory).
 */

/* Build order for the core system (dependencies first) */
static const char *build_world_order[] = {
    "musl",
    "zstd",
    "lz4",
    "libressl",
    "toybox",
    "mksh",
    "samurai",
    "llvm",
    "openrc",
    "dropbear",
    "curl",
    "dhcpcd",
    "unbound",
    "doas",
    "socklog",
    "snooze",
    "mandoc",
    "ifupdown-ng",
    "pigz",
    "nvi",
    "jpkg",
    NULL
};

int cmd_build_world(int argc, char **argv) {
    const char *recipes_dir = "packages/core";
    const char *output_dir = "output/packages";

    /* Parse options */
    for (int i = 0; i < argc; i++) {
        if ((strcmp(argv[i], "--recipes") == 0 || strcmp(argv[i], "-r") == 0) &&
            i + 1 < argc) {
            recipes_dir = argv[++i];
        } else if ((strcmp(argv[i], "--output") == 0 || strcmp(argv[i], "-o") == 0) &&
                   i + 1 < argc) {
            output_dir = argv[++i];
        }
    }

    log_info("building world from %s...", recipes_dir);
    log_info("output directory: %s", output_dir);

    mkdirs(output_dir, 0755);

    int total = 0, success = 0, failed = 0;

    for (int i = 0; build_world_order[i]; i++) {
        char recipe_path[512];
        snprintf(recipe_path, sizeof(recipe_path), "%s/%s",
                 recipes_dir, build_world_order[i]);

        if (!dir_exists(recipe_path)) {
            log_warn("recipe not found: %s (skipping)", recipe_path);
            continue;
        }

        total++;
        log_info("=== Building %s (%d of ...) ===", build_world_order[i], total);

        char *build_argv[4];
        build_argv[0] = recipe_path;
        build_argv[1] = (char *)"--output";
        build_argv[2] = (char *)output_dir;

        int rc = cmd_build(3, build_argv);
        if (rc == 0) {
            success++;
        } else {
            failed++;
            log_error("FAILED: %s", build_world_order[i]);
        }
    }

    printf("\n=== Build World Summary ===\n");
    printf("  Total:   %d\n", total);
    printf("  Success: %d\n", success);
    printf("  Failed:  %d\n", failed);

    return failed > 0 ? 1 : 0;
}
