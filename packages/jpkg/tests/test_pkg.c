/*
 * jpkg - jonerix package manager
 * test_pkg.c - Package format tests
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "../src/pkg.h"
#include "../src/util.h"
#include <stdio.h>
#include <string.h>
#include <assert.h>
#include <unistd.h>
#include <sys/utsname.h>

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

/* ========== Test Cases ========== */

static void test_magic_validation(void) {
    TEST(magic_validation);
    uint8_t good[] = {'J', 'P', 'K', 'G', 0, 1, 0, 0};
    uint8_t bad[]  = {'J', 'P', 'K', 'X', 0, 1, 0, 0};
    ASSERT(pkg_validate_magic(good, 8) == true, "valid magic rejected");
    ASSERT(pkg_validate_magic(bad, 8) == false, "invalid magic accepted");
    ASSERT(pkg_validate_magic(good, 4) == false, "short buffer accepted");
    PASS();
}

static void test_meta_from_toml(void) {
    TEST(meta_from_toml);
    const char *toml =
        "[package]\n"
        "name = \"toybox\"\n"
        "version = \"0.8.11\"\n"
        "license = \"0BSD\"\n"
        "description = \"BSD-licensed coreutils\"\n"
        "arch = \"x86_64\"\n"
        "\n"
        "[depends]\n"
        "runtime = [\"musl\"]\n"
        "build = [\"clang\", \"samurai\"]\n"
        "\n"
        "[files]\n"
        "sha256 = \"abcdef\"\n"
        "size = 245760\n";

    pkg_meta_t *meta = pkg_meta_from_toml(toml);
    ASSERT(meta != NULL, "parse failed");
    ASSERT(strcmp(meta->name, "toybox") == 0, "name");
    ASSERT(strcmp(meta->version, "0.8.11") == 0, "version");
    ASSERT(strcmp(meta->license, "0BSD") == 0, "license");
    ASSERT(strcmp(meta->description, "BSD-licensed coreutils") == 0, "description");
    ASSERT(strcmp(meta->arch, "x86_64") == 0, "arch");
    ASSERT(meta->runtime_dep_count == 1, "runtime dep count");
    ASSERT(strcmp(meta->runtime_deps[0], "musl") == 0, "runtime dep 0");
    ASSERT(meta->build_dep_count == 2, "build dep count");
    ASSERT(meta->content_size == 245760, "content size");
    ASSERT(strcmp(meta->content_sha256, "abcdef") == 0, "sha256");
    pkg_meta_free(meta);
    PASS();
}

static void test_meta_missing_name(void) {
    TEST(meta_missing_name);
    const char *toml = "[package]\nversion = \"1.0\"\n";
    pkg_meta_t *meta = pkg_meta_from_toml(toml);
    ASSERT(meta == NULL, "should fail without name");
    PASS();
}

static void test_meta_missing_version(void) {
    TEST(meta_missing_version);
    const char *toml = "[package]\nname = \"test\"\n";
    pkg_meta_t *meta = pkg_meta_from_toml(toml);
    ASSERT(meta == NULL, "should fail without version");
    PASS();
}

static void test_pkg_create_and_parse(void) {
    TEST(pkg_create_and_parse);

    const char *toml =
        "[package]\n"
        "name = \"test-pkg\"\n"
        "version = \"1.0.0\"\n"
        "license = \"MIT\"\n";

    /* Create a package with no payload */
    const char *path = "/tmp/test-jpkg-create.jpkg";
    int rc = pkg_create(path, toml, NULL, 0);
    ASSERT(rc == 0, "create failed");

    /* Parse it back */
    size_t off, len;
    pkg_meta_t *meta = pkg_parse_file(path, &off, &len);
    ASSERT(meta != NULL, "parse failed");
    ASSERT(strcmp(meta->name, "test-pkg") == 0, "name mismatch");
    ASSERT(strcmp(meta->version, "1.0.0") == 0, "version mismatch");
    ASSERT(strcmp(meta->license, "MIT") == 0, "license mismatch");
    ASSERT(len == 0, "should have no payload");

    pkg_meta_free(meta);
    unlink(path);
    PASS();
}

static void test_pkg_create_with_payload(void) {
    TEST(pkg_create_with_payload);

    const char *toml =
        "[package]\n"
        "name = \"payload-test\"\n"
        "version = \"2.0\"\n";

    const uint8_t payload[] = "fake zstd payload data for testing";
    size_t plen = sizeof(payload) - 1;

    const char *path = "/tmp/test-jpkg-payload.jpkg";
    int rc = pkg_create(path, toml, payload, plen);
    ASSERT(rc == 0, "create failed");

    /* Parse and verify payload offset/length */
    size_t off, len;
    pkg_meta_t *meta = pkg_parse_file(path, &off, &len);
    ASSERT(meta != NULL, "parse failed");
    ASSERT(len == plen, "payload length mismatch");

    /* Verify payload content */
    uint8_t *data;
    ssize_t flen = file_read(path, &data);
    ASSERT(flen > 0, "read failed");
    ASSERT(memcmp(data + off, payload, plen) == 0, "payload content mismatch");

    free(data);
    pkg_meta_free(meta);
    unlink(path);
    PASS();
}

static void test_pkg_filename(void) {
    TEST(pkg_filename);
    char *fn = pkg_filename("toybox", "0.8.11");
    ASSERT(fn != NULL, "NULL filename");
    struct utsname uts;
    const char *arch = "x86_64";
    if (uname(&uts) == 0) arch = uts.machine;
    char expected[128];
    snprintf(expected, sizeof(expected), "toybox-0.8.11-%s.jpkg", arch);
    ASSERT(strcmp(fn, expected) == 0, "wrong filename");
    free(fn);
    PASS();
}

static void test_meta_to_toml(void) {
    TEST(meta_to_toml);

    pkg_meta_t *meta = xcalloc(1, sizeof(pkg_meta_t));
    meta->name = xstrdup("test");
    meta->version = xstrdup("1.0");
    meta->license = xstrdup("MIT");
    meta->description = xstrdup("Test package");
    meta->arch = xstrdup("x86_64");

    char *toml = pkg_meta_to_toml(meta);
    ASSERT(toml != NULL, "serialize failed");
    ASSERT(strstr(toml, "test") != NULL, "name missing");
    ASSERT(strstr(toml, "1.0") != NULL, "version missing");
    ASSERT(strstr(toml, "MIT") != NULL, "license missing");

    /* Parse it back */
    pkg_meta_t *meta2 = pkg_meta_from_toml(toml);
    ASSERT(meta2 != NULL, "re-parse failed");
    ASSERT(strcmp(meta2->name, "test") == 0, "roundtrip name");

    free(toml);
    pkg_meta_free(meta);
    pkg_meta_free(meta2);
    PASS();
}

static void test_parse_buffer_too_small(void) {
    TEST(parse_buffer_too_small);
    uint8_t data[4] = {0};
    pkg_meta_t *meta = pkg_parse_buffer(data, 4, NULL, NULL);
    ASSERT(meta == NULL, "should fail on small buffer");
    PASS();
}

static void test_parse_bad_magic(void) {
    TEST(parse_bad_magic);
    uint8_t data[16] = {'N', 'O', 'T', 'J', 'P', 'K', 'G', 0, 0, 0, 0, 0, 0, 0, 0, 0};
    pkg_meta_t *meta = pkg_parse_buffer(data, 16, NULL, NULL);
    ASSERT(meta == NULL, "should fail on bad magic");
    PASS();
}

/* ========== Main ========== */

int main(void) {
    printf("=== Package Format Tests ===\n\n");

    test_magic_validation();
    test_meta_from_toml();
    test_meta_missing_name();
    test_meta_missing_version();
    test_pkg_create_and_parse();
    test_pkg_create_with_payload();
    test_pkg_filename();
    test_meta_to_toml();
    test_parse_buffer_too_small();
    test_parse_bad_magic();

    printf("\n  %d tests: %d passed, %d failed\n",
           tests_run, tests_passed, tests_failed);

    return tests_failed > 0 ? 1 : 0;
}
