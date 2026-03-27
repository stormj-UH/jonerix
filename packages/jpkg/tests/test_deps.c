/*
 * jpkg - jonerix package manager
 * test_deps.c - Dependency resolution tests
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "../src/deps.h"
#include "../src/repo.h"
#include "../src/db.h"
#include "../src/util.h"
#include <stdio.h>
#include <string.h>
#include <assert.h>

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

/* ========== Test Fixtures ========== */

/*
 * Build a test repository index:
 *   musl (no deps)
 *   zstd -> musl
 *   libressl -> musl
 *   curl -> musl, libressl
 *   toybox -> musl
 *   jpkg -> musl, libressl, zstd
 */

static repo_entry_t *make_entry(const char *name, const char *version,
                                const char **deps, size_t dep_count) {
    repo_entry_t *e = xcalloc(1, sizeof(repo_entry_t));
    e->name = xstrdup(name);
    e->version = xstrdup(version);
    e->license = xstrdup("MIT");
    e->description = xstrdup("");
    e->arch = xstrdup("x86_64");
    e->sha256 = xstrdup("");

    if (dep_count > 0) {
        e->runtime_deps = xcalloc(dep_count, sizeof(char *));
        e->runtime_dep_count = dep_count;
        for (size_t i = 0; i < dep_count; i++)
            e->runtime_deps[i] = xstrdup(deps[i]);
    }

    return e;
}

static repo_index_t *build_test_index(void) {
    repo_index_t *idx = xcalloc(1, sizeof(repo_index_t));

    /* Build entries in reverse order (linked list prepends) */
    const char *jpkg_deps[] = {"musl", "libressl", "zstd"};
    repo_entry_t *jpkg = make_entry("jpkg", "1.0.0", jpkg_deps, 3);

    const char *curl_deps[] = {"musl", "libressl"};
    repo_entry_t *curl = make_entry("curl", "8.5.0", curl_deps, 2);

    const char *toybox_deps[] = {"musl"};
    repo_entry_t *toybox = make_entry("toybox", "0.8.11", toybox_deps, 1);

    const char *libressl_deps[] = {"musl"};
    repo_entry_t *libressl = make_entry("libressl", "3.9.0", libressl_deps, 1);

    const char *zstd_deps[] = {"musl"};
    repo_entry_t *zstd = make_entry("zstd", "1.5.5", zstd_deps, 1);

    repo_entry_t *musl = make_entry("musl", "1.2.5", NULL, 0);

    /* Chain them */
    musl->next = zstd;
    zstd->next = libressl;
    libressl->next = toybox;
    toybox->next = curl;
    curl->next = jpkg;

    idx->entries = musl;
    idx->entry_count = 6;

    return idx;
}

static void free_test_index(repo_index_t *idx) {
    repo_entry_t *e = idx->entries;
    while (e) {
        repo_entry_t *next = e->next;
        free(e->name);
        free(e->version);
        free(e->license);
        free(e->description);
        free(e->arch);
        free(e->sha256);
        for (size_t i = 0; i < e->runtime_dep_count; i++) free(e->runtime_deps[i]);
        free(e->runtime_deps);
        free(e);
        e = next;
    }
    free(idx);
}

/* ========== Test Cases ========== */

static void test_resolve_no_deps(void) {
    TEST(resolve_no_deps);
    repo_index_t *idx = build_test_index();
    dep_list_t *list = deps_resolve(idx, NULL, "musl", true);
    ASSERT(list != NULL, "resolve failed");
    ASSERT(list->count == 1, "should be 1 package");
    ASSERT(strcmp(list->packages[0], "musl") == 0, "should be musl");
    dep_list_free(list);
    free_test_index(idx);
    PASS();
}

static void test_resolve_single_dep(void) {
    TEST(resolve_single_dep);
    repo_index_t *idx = build_test_index();
    dep_list_t *list = deps_resolve(idx, NULL, "zstd", true);
    ASSERT(list != NULL, "resolve failed");
    ASSERT(list->count == 2, "should be 2 packages");
    /* musl should come before zstd (dependency first) */
    ASSERT(strcmp(list->packages[0], "musl") == 0, "first should be musl");
    ASSERT(strcmp(list->packages[1], "zstd") == 0, "second should be zstd");
    dep_list_free(list);
    free_test_index(idx);
    PASS();
}

static void test_resolve_multi_dep(void) {
    TEST(resolve_multi_dep);
    repo_index_t *idx = build_test_index();
    dep_list_t *list = deps_resolve(idx, NULL, "curl", true);
    ASSERT(list != NULL, "resolve failed");
    /* curl -> musl, libressl. libressl -> musl. So: musl, libressl, curl */
    ASSERT(list->count == 3, "should be 3 packages");
    /* musl must come first */
    ASSERT(strcmp(list->packages[0], "musl") == 0, "first should be musl");
    /* curl must come last */
    ASSERT(strcmp(list->packages[list->count - 1], "curl") == 0, "last should be curl");
    dep_list_free(list);
    free_test_index(idx);
    PASS();
}

static void test_resolve_deep_deps(void) {
    TEST(resolve_deep_deps);
    repo_index_t *idx = build_test_index();
    dep_list_t *list = deps_resolve(idx, NULL, "jpkg", true);
    ASSERT(list != NULL, "resolve failed");
    /* jpkg -> musl, libressl, zstd. All depend on musl. */
    /* Should get: musl, libressl, zstd, jpkg (or musl, zstd, libressl, jpkg) */
    ASSERT(list->count == 4, "should be 4 packages");
    /* musl must come first */
    ASSERT(strcmp(list->packages[0], "musl") == 0, "first should be musl");
    /* jpkg must come last */
    ASSERT(strcmp(list->packages[3], "jpkg") == 0, "last should be jpkg");
    dep_list_free(list);
    free_test_index(idx);
    PASS();
}

static void test_resolve_not_found(void) {
    TEST(resolve_not_found);
    repo_index_t *idx = build_test_index();
    dep_list_t *list = deps_resolve(idx, NULL, "nonexistent", true);
    ASSERT(list == NULL, "should fail for missing package");
    free_test_index(idx);
    PASS();
}

static void test_resolve_multi_packages(void) {
    TEST(resolve_multi_packages);
    repo_index_t *idx = build_test_index();
    const char *pkgs[] = {"toybox", "zstd"};
    dep_list_t *list = deps_resolve_multi(idx, NULL, pkgs, 2, true);
    ASSERT(list != NULL, "resolve failed");
    /* Both need musl, plus toybox and zstd = 3 packages */
    ASSERT(list->count == 3, "should be 3 packages");
    /* musl first */
    ASSERT(strcmp(list->packages[0], "musl") == 0, "first should be musl");
    dep_list_free(list);
    free_test_index(idx);
    PASS();
}

static void test_no_cycle(void) {
    TEST(no_cycle);
    repo_index_t *idx = build_test_index();
    ASSERT(deps_has_cycle(idx, "jpkg") == false, "no cycle should exist");
    ASSERT(deps_has_cycle(idx, "musl") == false, "no cycle should exist");
    free_test_index(idx);
    PASS();
}

static void test_version_compare(void) {
    TEST(version_compare);
    ASSERT(version_compare("1.0", "1.0") == 0, "equal");
    ASSERT(version_compare("1.0", "2.0") < 0, "less");
    ASSERT(version_compare("2.0", "1.0") > 0, "greater");
    ASSERT(version_compare("1.2.3", "1.2.4") < 0, "patch less");
    ASSERT(version_compare("1.10", "1.9") > 0, "numeric compare");
    ASSERT(version_compare("0.8.11", "0.8.9") > 0, "multi-digit");
    ASSERT(version_compare("1.0.0", "1.0.0") == 0, "three-part equal");
    PASS();
}

static void test_license_check(void) {
    TEST(license_check);
    ASSERT(license_is_permissive("MIT") == true, "MIT");
    ASSERT(license_is_permissive("BSD-2-Clause") == true, "BSD-2");
    ASSERT(license_is_permissive("BSD-3-Clause") == true, "BSD-3");
    ASSERT(license_is_permissive("ISC") == true, "ISC");
    ASSERT(license_is_permissive("Apache-2.0") == true, "Apache");
    ASSERT(license_is_permissive("0BSD") == true, "0BSD");
    ASSERT(license_is_permissive("CC0") == true, "CC0");
    ASSERT(license_is_permissive("public domain") == true, "PD");
    ASSERT(license_is_permissive("GPL-2.0") == false, "GPL");
    ASSERT(license_is_permissive("LGPL-2.1") == false, "LGPL");
    ASSERT(license_is_permissive("AGPL-3.0") == false, "AGPL");
    PASS();
}

/* ========== Main ========== */

int main(void) {
    printf("=== Dependency Resolution Tests ===\n\n");

    /* Suppress log output during tests */
    log_set_level(LOG_ERROR);

    test_resolve_no_deps();
    test_resolve_single_dep();
    test_resolve_multi_dep();
    test_resolve_deep_deps();
    test_resolve_not_found();
    test_resolve_multi_packages();
    test_no_cycle();
    test_version_compare();
    test_license_check();

    printf("\n  %d tests: %d passed, %d failed\n",
           tests_run, tests_passed, tests_failed);

    return tests_failed > 0 ? 1 : 0;
}
