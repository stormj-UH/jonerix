/*
 * jpkg - jonerix package manager
 * test_toml.c - TOML parser tests
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "../src/toml.h"
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

/* ========== Test Cases ========== */

static void test_simple_string(void) {
    TEST(simple_string);
    const char *input = "name = \"hello\"\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(toml_get_string(doc, "name") != NULL, "key not found");
    ASSERT(strcmp(toml_get_string(doc, "name"), "hello") == 0, "wrong value");
    toml_free(doc);
    PASS();
}

static void test_integer(void) {
    TEST(integer);
    const char *input = "count = 42\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    int64_t val;
    ASSERT(toml_get_integer(doc, "count", &val), "key not found");
    ASSERT(val == 42, "wrong value");
    toml_free(doc);
    PASS();
}

static void test_negative_integer(void) {
    TEST(negative_integer);
    const char *input = "offset = -100\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    int64_t val;
    ASSERT(toml_get_integer(doc, "offset", &val), "key not found");
    ASSERT(val == -100, "wrong value");
    toml_free(doc);
    PASS();
}

static void test_boolean_true(void) {
    TEST(boolean_true);
    const char *input = "enabled = true\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    bool val;
    ASSERT(toml_get_boolean(doc, "enabled", &val), "key not found");
    ASSERT(val == true, "wrong value");
    toml_free(doc);
    PASS();
}

static void test_boolean_false(void) {
    TEST(boolean_false);
    const char *input = "disabled = false\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    bool val;
    ASSERT(toml_get_boolean(doc, "disabled", &val), "key not found");
    ASSERT(val == false, "wrong value");
    toml_free(doc);
    PASS();
}

static void test_string_array(void) {
    TEST(string_array);
    const char *input = "deps = [\"musl\", \"libressl\", \"zstd\"]\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    const toml_array_t *arr = toml_get_array(doc, "deps");
    ASSERT(arr != NULL, "array not found");
    ASSERT(arr->count == 3, "wrong count");
    ASSERT(strcmp(arr->items[0], "musl") == 0, "item 0 wrong");
    ASSERT(strcmp(arr->items[1], "libressl") == 0, "item 1 wrong");
    ASSERT(strcmp(arr->items[2], "zstd") == 0, "item 2 wrong");
    toml_free(doc);
    PASS();
}

static void test_empty_array(void) {
    TEST(empty_array);
    const char *input = "deps = []\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    const toml_array_t *arr = toml_get_array(doc, "deps");
    ASSERT(arr != NULL, "array not found");
    ASSERT(arr->count == 0, "wrong count");
    toml_free(doc);
    PASS();
}

static void test_section(void) {
    TEST(section);
    const char *input =
        "[package]\n"
        "name = \"toybox\"\n"
        "version = \"0.8.11\"\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(strcmp(toml_get_string(doc, "package.name"), "toybox") == 0, "name wrong");
    ASSERT(strcmp(toml_get_string(doc, "package.version"), "0.8.11") == 0, "version wrong");
    toml_free(doc);
    PASS();
}

static void test_multiple_sections(void) {
    TEST(multiple_sections);
    const char *input =
        "[package]\n"
        "name = \"toybox\"\n"
        "version = \"0.8.11\"\n"
        "license = \"0BSD\"\n"
        "\n"
        "[depends]\n"
        "runtime = [\"musl\"]\n"
        "build = [\"clang\", \"samurai\"]\n"
        "\n"
        "[files]\n"
        "sha256 = \"abc123\"\n"
        "size = 245760\n";

    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");

    ASSERT(strcmp(toml_get_string(doc, "package.name"), "toybox") == 0, "name");
    ASSERT(strcmp(toml_get_string(doc, "package.version"), "0.8.11") == 0, "version");
    ASSERT(strcmp(toml_get_string(doc, "package.license"), "0BSD") == 0, "license");

    const toml_array_t *rt = toml_get_array(doc, "depends.runtime");
    ASSERT(rt != NULL && rt->count == 1, "runtime deps");
    ASSERT(strcmp(rt->items[0], "musl") == 0, "runtime dep 0");

    const toml_array_t *bd = toml_get_array(doc, "depends.build");
    ASSERT(bd != NULL && bd->count == 2, "build deps");

    int64_t size;
    ASSERT(toml_get_integer(doc, "files.size", &size), "files.size");
    ASSERT(size == 245760, "wrong size");

    toml_free(doc);
    PASS();
}

static void test_comments(void) {
    TEST(comments);
    const char *input =
        "# This is a comment\n"
        "name = \"test\" # inline comment\n"
        "# Another comment\n"
        "value = 42\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(strcmp(toml_get_string(doc, "name"), "test") == 0, "name wrong");
    int64_t val;
    ASSERT(toml_get_integer(doc, "value", &val) && val == 42, "value wrong");
    toml_free(doc);
    PASS();
}

static void test_escape_sequences(void) {
    TEST(escape_sequences);
    const char *input = "msg = \"hello\\nworld\\t!\"\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(strcmp(toml_get_string(doc, "msg"), "hello\nworld\t!") == 0, "escapes wrong");
    toml_free(doc);
    PASS();
}

static void test_multiline_array(void) {
    TEST(multiline_array);
    const char *input =
        "deps = [\n"
        "    \"musl\",\n"
        "    \"libressl\",\n"
        "    \"zstd\",\n"
        "]\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    const toml_array_t *arr = toml_get_array(doc, "deps");
    ASSERT(arr != NULL, "array not found");
    ASSERT(arr->count == 3, "wrong count");
    toml_free(doc);
    PASS();
}

static void test_has_key(void) {
    TEST(has_key);
    const char *input = "name = \"test\"\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(toml_has_key(doc, "name") == true, "should have key");
    ASSERT(toml_has_key(doc, "missing") == false, "should not have key");
    toml_free(doc);
    PASS();
}

static void test_serialization(void) {
    TEST(serialization);
    toml_doc_t *doc = toml_new();
    toml_set_string(doc, "package.name", "test");
    toml_set_string(doc, "package.version", "1.0");
    toml_set_integer(doc, "files.size", 1024);

    const char *items[] = {"dep1", "dep2"};
    toml_set_array(doc, "depends.runtime", items, 2);

    char *s = toml_serialize(doc);
    ASSERT(s != NULL, "serialize failed");
    ASSERT(strstr(s, "name") != NULL, "missing name");
    ASSERT(strstr(s, "test") != NULL, "missing test");
    ASSERT(strstr(s, "1024") != NULL, "missing size");

    /* Re-parse the serialized output */
    char *err = NULL;
    toml_doc_t *doc2 = toml_parse(s, &err);
    ASSERT(doc2 != NULL, err ? err : "re-parse failed");
    ASSERT(strcmp(toml_get_string(doc2, "package.name"), "test") == 0, "roundtrip name");

    free(s);
    toml_free(doc);
    toml_free(doc2);
    PASS();
}

static void test_empty_input(void) {
    TEST(empty_input);
    char *err = NULL;
    toml_doc_t *doc = toml_parse("", &err);
    ASSERT(doc != NULL, "should parse empty");
    ASSERT(doc->count == 0, "should have no values");
    toml_free(doc);
    PASS();
}

static void test_dotted_section(void) {
    TEST(dotted_section);
    const char *input =
        "[a.b]\n"
        "key = \"val\"\n";
    char *err = NULL;
    toml_doc_t *doc = toml_parse(input, &err);
    ASSERT(doc != NULL, err ? err : "parse failed");
    ASSERT(strcmp(toml_get_string(doc, "a.b.key"), "val") == 0, "dotted key wrong");
    toml_free(doc);
    PASS();
}

static void test_update_value(void) {
    TEST(update_value);
    toml_doc_t *doc = toml_new();
    toml_set_string(doc, "key", "first");
    ASSERT(strcmp(toml_get_string(doc, "key"), "first") == 0, "initial value");
    toml_set_string(doc, "key", "second");
    ASSERT(strcmp(toml_get_string(doc, "key"), "second") == 0, "updated value");
    toml_free(doc);
    PASS();
}

/* ========== Main ========== */

int main(void) {
    printf("=== TOML Parser Tests ===\n\n");

    test_simple_string();
    test_integer();
    test_negative_integer();
    test_boolean_true();
    test_boolean_false();
    test_string_array();
    test_empty_array();
    test_section();
    test_multiple_sections();
    test_comments();
    test_escape_sequences();
    test_multiline_array();
    test_has_key();
    test_serialization();
    test_empty_input();
    test_dotted_section();
    test_update_value();

    printf("\n  %d tests: %d passed, %d failed\n",
           tests_run, tests_passed, tests_failed);

    return tests_failed > 0 ? 1 : 0;
}
