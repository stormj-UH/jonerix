/*
 * jpkg - jonerix package manager
 * test_db.c - Database loading tests
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "../src/db.h"
#include "../src/util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>

static int tests_run = 0;
static int tests_passed = 0;
static int tests_failed = 0;

#define TEST(name) \
    do { \
        tests_run++; \
        printf("  %-50s ", #name); \
    } while (0)

#define PASS() \
    do { \
        tests_passed++; \
        printf("PASS\n"); \
    } while (0)

#define FAIL(msg) \
    do { \
        tests_failed++; \
        printf("FAIL: %s\n", msg); \
    } while (0)

#define ASSERT(cond, msg) \
    do { \
        if (!(cond)) { FAIL(msg); return; } \
    } while (0)

static void cleanup_tree(const char *root) {
    char cmd[1024];
    snprintf(cmd, sizeof(cmd), "rm -rf '%s'", root);
    (void)system(cmd);
}

static void test_load_legacy_metadata_without_hooks(void) {
    TEST(load_legacy_metadata_without_hooks);

    char root[128];
    snprintf(root, sizeof(root), "/tmp/jpkg-test-db-%d", (int)getpid());
    cleanup_tree(root);
    ASSERT(mkdirs(root, 0755) == 0, "mkdir root failed");
    set_rootfs(root);

    char pkg_dir[1024];
    snprintf(pkg_dir, sizeof(pkg_dir), "%s/var/db/jpkg/installed/jmake", root);
    ASSERT(mkdirs(pkg_dir, 0755) == 0, "mkdirs failed");

    const char *broken_meta =
        "[package]\n"
        "name = \"jmake\"\n"
        "version = \"1.0.0\"\n"
        "license = \"MIT\"\n"
        "description = \"Clean-room drop-in replacement for GNU Make, written in Rust\"\n"
        "arch = \"aarch64\"\n"
        "install_time = 123\n"
        "\n"
        "[depends]\n"
        "runtime = [\"musl\"]\n"
        "\n"
        "[hooks]\n"
        "post_install = \"STATE_DIR=/var/db/jpkg/state/jmake\n"
        "mkdir -p \"$STATE_DIR\"\n"
        "ln -sf jmake /bin/make\"\n";

    char meta_path[1024];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata.toml", pkg_dir);
    ASSERT(file_write(meta_path, (const uint8_t *)broken_meta, strlen(broken_meta)) == 0,
           "write metadata failed");

    char files_path[1024];
    snprintf(files_path, sizeof(files_path), "%s/files", pkg_dir);
    const char *files_text =
        "1111111111111111111111111111111111111111111111111111111111111111 100755 /bin/jmake\n";
    ASSERT(file_write(files_path, (const uint8_t *)files_text, strlen(files_text)) == 0,
           "write files failed");

    jpkg_db_t *db = db_open();
    ASSERT(db != NULL, "db_open failed");
    ASSERT(db_load(db) == 0, "db_load failed");

    db_pkg_t *pkg = db_get_package(db, "jmake");
    ASSERT(pkg != NULL, "jmake not recovered");
    ASSERT(strcmp(pkg->version, "1.0.0") == 0, "wrong version");
    ASSERT(pkg->runtime_dep_count == 1, "wrong runtime dep count");
    ASSERT(strcmp(pkg->runtime_deps[0], "musl") == 0, "wrong runtime dep");
    ASSERT(pkg->post_install == NULL, "hooks should have been dropped");

    uint8_t *rewritten = NULL;
    ssize_t len = file_read(meta_path, &rewritten);
    ASSERT(len > 0, "rewrite failed");
    ASSERT(strstr((const char *)rewritten, "[hooks]") == NULL, "hooks section still present");
    free(rewritten);

    db_close(db);
    set_rootfs("");
    cleanup_tree(root);
    PASS();
}

int main(void) {
    printf("=== Database Tests ===\n\n");

    test_load_legacy_metadata_without_hooks();

    printf("\n  %d tests: %d passed, %d failed\n",
           tests_run, tests_passed, tests_failed);

    return tests_failed > 0 ? 1 : 0;
}
