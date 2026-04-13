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
#include <fcntl.h>

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

static int verify_count_cb(const char *path, const char *expected,
                            const char *actual, void *ctx) {
    (void)path; (void)expected; (void)actual;
    int *count = (int *)ctx;
    (*count)++;
    return 0;
}

static void test_verify_files_regular(void) {
    TEST(verify_files_regular);

    char root[128];
    snprintf(root, sizeof(root), "/tmp/jpkg-test-verify-%d", (int)getpid());
    cleanup_tree(root);
    ASSERT(mkdirs(root, 0755) == 0, "mkdir root failed");
    set_rootfs(root);

    /* Set up DB */
    char pkg_dir[1024];
    snprintf(pkg_dir, sizeof(pkg_dir), "%s/var/db/jpkg/installed/testpkg", root);
    ASSERT(mkdirs(pkg_dir, 0755) == 0, "mkdirs failed");

    const char *meta =
        "[package]\n"
        "name = \"testpkg\"\n"
        "version = \"1.0\"\n"
        "license = \"MIT\"\n";
    char meta_path[1024];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata.toml", pkg_dir);
    ASSERT(file_write(meta_path, (const uint8_t *)meta, strlen(meta)) == 0, "write meta");

    /* Create the real file on disk */
    char bin_dir[1024];
    snprintf(bin_dir, sizeof(bin_dir), "%s/bin", root);
    ASSERT(mkdirs(bin_dir, 0755) == 0, "mkdirs bin");
    char file_path[1024];
    snprintf(file_path, sizeof(file_path), "%s/bin/testbin", root);
    const char *content = "#!/bin/sh\necho hello\n";
    ASSERT(file_write(file_path, (const uint8_t *)content, strlen(content)) == 0, "write file");

    /* Compute its sha256 */
    char sha256[65];
    ASSERT(sha256_file(file_path, sha256) == 0, "sha256 failed");

    /* Write files list with correct sha256 */
    char files_line[256];
    snprintf(files_line, sizeof(files_line), "%s 100755 /bin/testbin\n", sha256);
    char files_path[1024];
    snprintf(files_path, sizeof(files_path), "%s/files", pkg_dir);
    ASSERT(file_write(files_path, (const uint8_t *)files_line, strlen(files_line)) == 0,
           "write files");

    jpkg_db_t *db = db_open();
    ASSERT(db != NULL, "db_open failed");
    ASSERT(db_load(db) == 0, "db_load failed");

    /* Verify: should be 0 mismatches */
    int mismatch_count = 0;
    int mismatches = db_verify_files(db, "testpkg", verify_count_cb, &mismatch_count);
    ASSERT(mismatches == 0, "correctly installed file should verify OK");
    ASSERT(mismatch_count == 0, "no mismatches expected");

    db_close(db);
    set_rootfs("");
    cleanup_tree(root);
    PASS();
}

static void test_verify_files_symlink(void) {
    TEST(verify_files_symlink);

    char root[128];
    snprintf(root, sizeof(root), "/tmp/jpkg-test-verifysym-%d", (int)getpid());
    cleanup_tree(root);
    ASSERT(mkdirs(root, 0755) == 0, "mkdir root failed");
    set_rootfs(root);

    /* Set up DB */
    char pkg_dir[1024];
    snprintf(pkg_dir, sizeof(pkg_dir), "%s/var/db/jpkg/installed/sympkg", root);
    ASSERT(mkdirs(pkg_dir, 0755) == 0, "mkdirs failed");

    const char *meta =
        "[package]\n"
        "name = \"sympkg\"\n"
        "version = \"1.0\"\n"
        "license = \"MIT\"\n";
    char meta_path[1024];
    snprintf(meta_path, sizeof(meta_path), "%s/metadata.toml", pkg_dir);
    ASSERT(file_write(meta_path, (const uint8_t *)meta, strlen(meta)) == 0, "write meta");

    /* Create the real files: a regular file and a symlink pointing to it */
    char lib_dir[1024];
    snprintf(lib_dir, sizeof(lib_dir), "%s/lib", root);
    ASSERT(mkdirs(lib_dir, 0755) == 0, "mkdirs lib");

    char real_file[1024];
    snprintf(real_file, sizeof(real_file), "%s/lib/libfoo.so.1", root);
    const char *content = "fake library content";
    ASSERT(file_write(real_file, (const uint8_t *)content, strlen(content)) == 0,
           "write real file");

    char sym_path[1024];
    snprintf(sym_path, sizeof(sym_path), "%s/lib/libfoo.so", root);
    ASSERT(symlink("libfoo.so.1", sym_path) == 0, "symlink failed");

    /* Compute sha256 of the real file */
    char sha256[65];
    ASSERT(sha256_file(real_file, sha256) == 0, "sha256 failed");

    /* Write files list: regular file + symlink entry */
    char files_text[512];
    snprintf(files_text, sizeof(files_text),
             "%s 100644 /lib/libfoo.so.1\n"
             "0000000000000000000000000000000000000000000000000000000000000000"
             " 120777 /lib/libfoo.so -> libfoo.so.1\n",
             sha256);
    char files_path[1024];
    snprintf(files_path, sizeof(files_path), "%s/files", pkg_dir);
    ASSERT(file_write(files_path, (const uint8_t *)files_text, strlen(files_text)) == 0,
           "write files");

    jpkg_db_t *db = db_open();
    ASSERT(db != NULL, "db_open failed");
    ASSERT(db_load(db) == 0, "db_load failed");

    /* Verify: both regular file and symlink should pass */
    int mismatch_count = 0;
    int mismatches = db_verify_files(db, "sympkg", verify_count_cb, &mismatch_count);
    ASSERT(mismatches == 0, "correctly installed symlink should verify OK");
    ASSERT(mismatch_count == 0, "no mismatches expected for symlink");

    db_close(db);
    set_rootfs("");
    cleanup_tree(root);
    PASS();
}

int main(void) {
    printf("=== Database Tests ===\n\n");

    test_load_legacy_metadata_without_hooks();
    test_verify_files_regular();
    test_verify_files_symlink();

    printf("\n  %d tests: %d passed, %d failed\n",
           tests_run, tests_passed, tests_failed);

    return tests_failed > 0 ? 1 : 0;
}
