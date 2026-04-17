/*
 * jpkg - jonerix package manager
 * util.h - Shared utilities (logging, error handling, memory)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#ifndef JPKG_UTIL_H
#define JPKG_UTIL_H

#define JPKG_VERSION "1.0.6"

#include <stddef.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/types.h>

/* Logging levels */
typedef enum {
    LOG_DEBUG = 0,
    LOG_INFO  = 1,
    LOG_WARN  = 2,
    LOG_ERROR = 3,
    LOG_FATAL = 4
} log_level_t;

/* Global verbosity level */
extern log_level_t g_log_level;

/* Set log level */
void log_set_level(log_level_t level);

/* Logging functions */
void log_debug(const char *fmt, ...);
void log_info(const char *fmt, ...);
void log_warn(const char *fmt, ...);
void log_error(const char *fmt, ...);
void log_fatal(const char *fmt, ...);  /* calls exit(1) */

/* Safe memory allocation (fatal on failure) */
void *xmalloc(size_t size);
void *xcalloc(size_t nmemb, size_t size);
void *xrealloc(void *ptr, size_t size);
char *xstrdup(const char *s);
char *xstrndup(const char *s, size_t n);

/* String utilities */
char *str_trim(char *s);
bool str_starts_with(const char *s, const char *prefix);
bool str_ends_with(const char *s, const char *suffix);
bool str_contains(const char *haystack, const char *needle);
char *str_replace(const char *s, const char *old, const char *new_str);

/* Path utilities */
char *path_join(const char *dir, const char *name);
int mkdirs(const char *path, mode_t mode);
bool file_exists(const char *path);
bool dir_exists(const char *path);
ssize_t file_read(const char *path, uint8_t **out_buf);
int file_write(const char *path, const uint8_t *data, size_t len);
int file_copy(const char *src, const char *dst);

/* SHA256 */
void sha256_hash(const uint8_t *data, size_t len, uint8_t out[32]);
void sha256_hex(const uint8_t hash[32], char out[65]);
int sha256_file(const char *path, char out[65]);

/* LE byte reading */
static inline uint32_t read_le32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static inline void write_le32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v & 0xff);
    p[1] = (uint8_t)((v >> 8) & 0xff);
    p[2] = (uint8_t)((v >> 16) & 0xff);
    p[3] = (uint8_t)((v >> 24) & 0xff);
}

/* Version comparison: returns <0, 0, >0 like strcmp */
int version_compare(const char *v1, const char *v2);

/* Permitted licenses */
bool license_is_permissive(const char *license);

typedef enum {
    TREE_AUDIT_OK = 0,
    TREE_AUDIT_ROOT_DOT_ZERO,
    TREE_AUDIT_LIB64_PATH,
    TREE_AUDIT_LIB64_REFERENCE,
    TREE_AUDIT_SBIN_PATH
} tree_audit_result_t;

tree_audit_result_t audit_layout_tree(const char *root, char *problem_path,
                                      size_t problem_path_len);
const char *audit_layout_result_string(tree_audit_result_t result);

/* Root filesystem prefix (for testing, normally "") */
extern const char *g_rootfs;
void set_rootfs(const char *prefix);

/* Database path */
#define JPKG_DB_DIR     "/var/db/jpkg"
#define JPKG_CACHE_DIR  "/var/cache/jpkg"
#define JPKG_CONFIG_DIR "/etc/jpkg"
#define JPKG_KEY_DIR    "/etc/jpkg/keys"

#endif /* JPKG_UTIL_H */
