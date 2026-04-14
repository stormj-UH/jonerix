/*
 * jpkg - jonerix package manager
 * util.c - Shared utilities (logging, error handling, memory, crypto)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "util.h"
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#include <fcntl.h>
#include <ctype.h>
#include <strings.h>
#include <time.h>

/* ========== Globals ========== */

log_level_t g_log_level = LOG_INFO;
const char *g_rootfs = "";

void set_rootfs(const char *prefix) {
    g_rootfs = prefix ? prefix : "";
}

/* ========== Logging ========== */

void log_set_level(log_level_t level) {
    g_log_level = level;
}

static const char *level_str(log_level_t level) {
    switch (level) {
        case LOG_DEBUG: return "debug";
        case LOG_INFO:  return "info";
        case LOG_WARN:  return "warn";
        case LOG_ERROR: return "error";
        case LOG_FATAL: return "fatal";
        default:        return "???";
    }
}

static void log_msg(log_level_t level, const char *fmt, va_list ap) {
    if (level < g_log_level) return;
    FILE *out = (level >= LOG_WARN) ? stderr : stdout;
    fprintf(out, "jpkg: %s: ", level_str(level));
    vfprintf(out, fmt, ap);
    fprintf(out, "\n");
    fflush(out);
}

void log_debug(const char *fmt, ...) {
    va_list ap; va_start(ap, fmt); log_msg(LOG_DEBUG, fmt, ap); va_end(ap);
}

void log_info(const char *fmt, ...) {
    va_list ap; va_start(ap, fmt); log_msg(LOG_INFO, fmt, ap); va_end(ap);
}

void log_warn(const char *fmt, ...) {
    va_list ap; va_start(ap, fmt); log_msg(LOG_WARN, fmt, ap); va_end(ap);
}

void log_error(const char *fmt, ...) {
    va_list ap; va_start(ap, fmt); log_msg(LOG_ERROR, fmt, ap); va_end(ap);
}

void log_fatal(const char *fmt, ...) {
    va_list ap; va_start(ap, fmt); log_msg(LOG_FATAL, fmt, ap); va_end(ap);
    exit(1);
}

/* ========== Safe Memory ========== */

void *xmalloc(size_t size) {
    void *p = malloc(size);
    if (!p && size > 0) log_fatal("out of memory (malloc %zu)", size);
    return p;
}

void *xcalloc(size_t nmemb, size_t size) {
    void *p = calloc(nmemb, size);
    if (!p && nmemb > 0 && size > 0)
        log_fatal("out of memory (calloc %zu*%zu)", nmemb, size);
    return p;
}

void *xrealloc(void *ptr, size_t size) {
    void *p = realloc(ptr, size);
    if (!p && size > 0) log_fatal("out of memory (realloc %zu)", size);
    return p;
}

char *xstrdup(const char *s) {
    if (!s) return NULL;
    char *p = strdup(s);
    if (!p) log_fatal("out of memory (strdup)");
    return p;
}

char *xstrndup(const char *s, size_t n) {
    if (!s) return NULL;
    size_t len = strlen(s);
    if (len > n) len = n;
    char *p = xmalloc(len + 1);
    memcpy(p, s, len);
    p[len] = '\0';
    return p;
}

/* ========== String Utilities ========== */

char *str_trim(char *s) {
    if (!s) return NULL;
    while (isspace((unsigned char)*s)) s++;
    if (*s == '\0') return s;
    char *end = s + strlen(s) - 1;
    while (end > s && isspace((unsigned char)*end)) end--;
    end[1] = '\0';
    return s;
}

bool str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return false;
    return strncmp(s, prefix, strlen(prefix)) == 0;
}

bool str_ends_with(const char *s, const char *suffix) {
    if (!s || !suffix) return false;
    size_t slen = strlen(s);
    size_t suflen = strlen(suffix);
    if (suflen > slen) return false;
    return strcmp(s + slen - suflen, suffix) == 0;
}

bool str_contains(const char *haystack, const char *needle) {
    if (!haystack || !needle) return false;
    return strstr(haystack, needle) != NULL;
}

char *str_replace(const char *s, const char *old, const char *new_str) {
    if (!s || !old || !new_str) return NULL;
    size_t old_len = strlen(old);
    size_t new_len = strlen(new_str);
    if (old_len == 0) return xstrdup(s);

    /* Count occurrences */
    size_t count = 0;
    const char *p = s;
    while ((p = strstr(p, old)) != NULL) {
        count++;
        p += old_len;
    }

    size_t result_len = strlen(s) + count * (new_len - old_len);
    char *result = xmalloc(result_len + 1);
    char *w = result;
    p = s;
    const char *q;
    while ((q = strstr(p, old)) != NULL) {
        size_t chunk = (size_t)(q - p);
        memcpy(w, p, chunk);
        w += chunk;
        memcpy(w, new_str, new_len);
        w += new_len;
        p = q + old_len;
    }
    strcpy(w, p);
    return result;
}

/* ========== Path Utilities ========== */

char *path_join(const char *dir, const char *name) {
    if (!dir || !name) return NULL;
    size_t dlen = strlen(dir);
    size_t nlen = strlen(name);
    bool need_sep = (dlen > 0 && dir[dlen-1] != '/' && nlen > 0 && name[0] != '/');
    char *path = xmalloc(dlen + nlen + (need_sep ? 2 : 1));
    memcpy(path, dir, dlen);
    if (need_sep) path[dlen++] = '/';
    memcpy(path + dlen, name, nlen);
    path[dlen + nlen] = '\0';
    return path;
}

int mkdirs(const char *path, mode_t mode) {
    if (!path) return -1;
    char *tmp = xstrdup(path);
    size_t len = strlen(tmp);

    /* Remove trailing slash */
    if (len > 1 && tmp[len - 1] == '/') tmp[len - 1] = '\0';

    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            if (mkdir(tmp, mode) != 0 && errno != EEXIST) {
                free(tmp);
                return -1;
            }
            *p = '/';
        }
    }
    int rc = mkdir(tmp, mode);
    free(tmp);
    return (rc == 0 || errno == EEXIST) ? 0 : -1;
}

bool file_exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0 && S_ISREG(st.st_mode);
}

bool dir_exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0 && S_ISDIR(st.st_mode);
}

ssize_t file_read(const char *path, uint8_t **out_buf) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;

    struct stat st;
    if (fstat(fd, &st) != 0) { close(fd); return -1; }

    size_t size = (size_t)st.st_size;
    uint8_t *buf = xmalloc(size + 1);
    size_t total = 0;
    while (total < size) {
        ssize_t n = read(fd, buf + total, size - total);
        if (n < 0) {
            if (errno == EINTR) continue;
            free(buf);
            close(fd);
            return -1;
        }
        if (n == 0) break;
        total += (size_t)n;
    }
    buf[total] = '\0';
    close(fd);
    *out_buf = buf;
    return (ssize_t)total;
}

int file_write(const char *path, const uint8_t *data, size_t len) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) return -1;

    size_t written = 0;
    while (written < len) {
        ssize_t n = write(fd, data + written, len - written);
        if (n < 0) {
            if (errno == EINTR) continue;
            close(fd);
            return -1;
        }
        written += (size_t)n;
    }
    close(fd);
    return 0;
}

int file_copy(const char *src, const char *dst) {
    uint8_t *buf;
    ssize_t len = file_read(src, &buf);
    if (len < 0) return -1;
    int rc = file_write(dst, buf, (size_t)len);
    free(buf);
    return rc;
}

/* ========== SHA-256 ========== */

/*
 * Minimal SHA-256 implementation.
 * Based on the public domain implementation by Brad Conte.
 */

typedef struct {
    uint8_t  data[64];
    uint32_t datalen;
    uint64_t bitlen;
    uint32_t state[8];
} sha256_ctx;

static const uint32_t sha256_k[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
};

#define ROTR(x,n)  (((x) >> (n)) | ((x) << (32 - (n))))
#define CH(x,y,z)  (((x) & (y)) ^ (~(x) & (z)))
#define MAJ(x,y,z) (((x) & (y)) ^ ((x) & (z)) ^ ((y) & (z)))
#define EP0(x)     (ROTR(x,2) ^ ROTR(x,13) ^ ROTR(x,22))
#define EP1(x)     (ROTR(x,6) ^ ROTR(x,11) ^ ROTR(x,25))
#define SIG0(x)    (ROTR(x,7) ^ ROTR(x,18) ^ ((x) >> 3))
#define SIG1(x)    (ROTR(x,17) ^ ROTR(x,19) ^ ((x) >> 10))

static void sha256_transform(sha256_ctx *ctx, const uint8_t data[64]) {
    uint32_t a, b, c, d, e, f, g, h, t1, t2, m[64];
    int i;

    for (i = 0; i < 16; i++) {
        m[i] = ((uint32_t)data[i*4] << 24) | ((uint32_t)data[i*4+1] << 16) |
               ((uint32_t)data[i*4+2] << 8) | ((uint32_t)data[i*4+3]);
    }
    for (i = 16; i < 64; i++) {
        m[i] = SIG1(m[i-2]) + m[i-7] + SIG0(m[i-15]) + m[i-16];
    }

    a = ctx->state[0]; b = ctx->state[1]; c = ctx->state[2]; d = ctx->state[3];
    e = ctx->state[4]; f = ctx->state[5]; g = ctx->state[6]; h = ctx->state[7];

    for (i = 0; i < 64; i++) {
        t1 = h + EP1(e) + CH(e,f,g) + sha256_k[i] + m[i];
        t2 = EP0(a) + MAJ(a,b,c);
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }

    ctx->state[0] += a; ctx->state[1] += b; ctx->state[2] += c; ctx->state[3] += d;
    ctx->state[4] += e; ctx->state[5] += f; ctx->state[6] += g; ctx->state[7] += h;
}

static void sha256_init(sha256_ctx *ctx) {
    ctx->datalen = 0;
    ctx->bitlen = 0;
    ctx->state[0] = 0x6a09e667; ctx->state[1] = 0xbb67ae85;
    ctx->state[2] = 0x3c6ef372; ctx->state[3] = 0xa54ff53a;
    ctx->state[4] = 0x510e527f; ctx->state[5] = 0x9b05688c;
    ctx->state[6] = 0x1f83d9ab; ctx->state[7] = 0x5be0cd19;
}

static void sha256_update(sha256_ctx *ctx, const uint8_t *data, size_t len) {
    for (size_t i = 0; i < len; i++) {
        ctx->data[ctx->datalen] = data[i];
        ctx->datalen++;
        if (ctx->datalen == 64) {
            sha256_transform(ctx, ctx->data);
            ctx->bitlen += 512;
            ctx->datalen = 0;
        }
    }
}

static void sha256_final(sha256_ctx *ctx, uint8_t hash[32]) {
    uint32_t i = ctx->datalen;

    if (i < 56) {
        ctx->data[i++] = 0x80;
        while (i < 56) ctx->data[i++] = 0x00;
    } else {
        ctx->data[i++] = 0x80;
        while (i < 64) ctx->data[i++] = 0x00;
        sha256_transform(ctx, ctx->data);
        memset(ctx->data, 0, 56);
    }

    ctx->bitlen += (uint64_t)ctx->datalen * 8;
    ctx->data[63] = (uint8_t)(ctx->bitlen);
    ctx->data[62] = (uint8_t)(ctx->bitlen >> 8);
    ctx->data[61] = (uint8_t)(ctx->bitlen >> 16);
    ctx->data[60] = (uint8_t)(ctx->bitlen >> 24);
    ctx->data[59] = (uint8_t)(ctx->bitlen >> 32);
    ctx->data[58] = (uint8_t)(ctx->bitlen >> 40);
    ctx->data[57] = (uint8_t)(ctx->bitlen >> 48);
    ctx->data[56] = (uint8_t)(ctx->bitlen >> 56);
    sha256_transform(ctx, ctx->data);

    /* Big endian output */
    for (i = 0; i < 4; i++) {
        hash[i]      = (uint8_t)((ctx->state[0] >> (24 - i * 8)) & 0xff);
        hash[i + 4]  = (uint8_t)((ctx->state[1] >> (24 - i * 8)) & 0xff);
        hash[i + 8]  = (uint8_t)((ctx->state[2] >> (24 - i * 8)) & 0xff);
        hash[i + 12] = (uint8_t)((ctx->state[3] >> (24 - i * 8)) & 0xff);
        hash[i + 16] = (uint8_t)((ctx->state[4] >> (24 - i * 8)) & 0xff);
        hash[i + 20] = (uint8_t)((ctx->state[5] >> (24 - i * 8)) & 0xff);
        hash[i + 24] = (uint8_t)((ctx->state[6] >> (24 - i * 8)) & 0xff);
        hash[i + 28] = (uint8_t)((ctx->state[7] >> (24 - i * 8)) & 0xff);
    }
}

void sha256_hash(const uint8_t *data, size_t len, uint8_t out[32]) {
    sha256_ctx ctx;
    sha256_init(&ctx);
    sha256_update(&ctx, data, len);
    sha256_final(&ctx, out);
}

void sha256_hex(const uint8_t hash[32], char out[65]) {
    static const char hex[] = "0123456789abcdef";
    for (int i = 0; i < 32; i++) {
        out[i*2]     = hex[(hash[i] >> 4) & 0xf];
        out[i*2 + 1] = hex[hash[i] & 0xf];
    }
    out[64] = '\0';
}

int sha256_file(const char *path, char out[65]) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;

    sha256_ctx ctx;
    sha256_init(&ctx);

    uint8_t buf[8192];
    ssize_t n;
    while ((n = read(fd, buf, sizeof(buf))) > 0) {
        sha256_update(&ctx, buf, (size_t)n);
    }
    close(fd);
    if (n < 0) return -1;

    uint8_t hash[32];
    sha256_final(&ctx, hash);
    sha256_hex(hash, out);
    return 0;
}

#undef ROTR
#undef CH
#undef MAJ
#undef EP0
#undef EP1
#undef SIG0
#undef SIG1

/* ========== Version Comparison ========== */

/*
 * Compare version strings like "1.2.3" vs "1.2.4".
 * Handles numeric and alphabetic segments.
 */
int version_compare(const char *v1, const char *v2) {
    if (!v1 && !v2) return 0;
    if (!v1) return -1;
    if (!v2) return 1;

    const char *p1 = v1, *p2 = v2;

    while (*p1 || *p2) {
        /* Skip leading separators */
        while (*p1 && !isalnum((unsigned char)*p1)) p1++;
        while (*p2 && !isalnum((unsigned char)*p2)) p2++;

        if (!*p1 && !*p2) break;
        if (!*p1) return -1;
        if (!*p2) return 1;

        /* Both are digits: compare numerically */
        if (isdigit((unsigned char)*p1) && isdigit((unsigned char)*p2)) {
            /* Skip leading zeros */
            while (*p1 == '0' && isdigit((unsigned char)*(p1+1))) p1++;
            while (*p2 == '0' && isdigit((unsigned char)*(p2+1))) p2++;

            const char *s1 = p1, *s2 = p2;
            while (isdigit((unsigned char)*p1)) p1++;
            while (isdigit((unsigned char)*p2)) p2++;
            size_t len1 = (size_t)(p1 - s1);
            size_t len2 = (size_t)(p2 - s2);

            if (len1 != len2) return (len1 > len2) ? 1 : -1;
            int cmp = memcmp(s1, s2, len1);
            if (cmp != 0) return cmp;
        }
        /* Both are alpha: compare lexically */
        else if (isalpha((unsigned char)*p1) && isalpha((unsigned char)*p2)) {
            while (isalpha((unsigned char)*p1) && isalpha((unsigned char)*p2)) {
                if (*p1 != *p2) return (*p1 > *p2) ? 1 : -1;
                p1++; p2++;
            }
            if (isalpha((unsigned char)*p1)) return 1;
            if (isalpha((unsigned char)*p2)) return -1;
        }
        /* Digit vs alpha: digits win (considered higher) */
        else {
            return isdigit((unsigned char)*p1) ? 1 : -1;
        }
    }
    return 0;
}

/* ========== License Checking ========== */

static const char *permissive_licenses[] = {
    "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Apache-2.0",
    "0BSD", "CC0", "CC0-1.0", "Unlicense", "curl",
    "MirOS", "OpenSSL", "SSLeay", "zlib", "Zlib",
    "public domain", "Public-Domain",
    "BSD-2-Clause-Patent", "PSF-2.0", "BSL-1.0", "Artistic-2.0",
    "Ruby", "MPL-2.0",
    NULL
};

bool license_is_permissive(const char *license) {
    if (!license) return false;

    /* Exact match */
    for (int i = 0; permissive_licenses[i]; i++) {
        if (strcasecmp(license, permissive_licenses[i]) == 0)
            return true;
    }

    /* SPDX OR: "MIT OR Apache-2.0" — permissive if any component is */
    const char *or_pos = strstr(license, " OR ");
    if (or_pos) {
        char left[128];
        size_t llen = (size_t)(or_pos - license);
        if (llen < sizeof(left)) {
            memcpy(left, license, llen);
            left[llen] = '\0';
            if (license_is_permissive(left)) return true;
        }
        return license_is_permissive(or_pos + 4);
    }

    /* SPDX AND: "MIT AND GPL-2.0" — permissive only if all components are */
    const char *and_pos = strstr(license, " AND ");
    if (and_pos) {
        char left[128];
        size_t llen = (size_t)(and_pos - license);
        if (llen < sizeof(left)) {
            memcpy(left, license, llen);
            left[llen] = '\0';
            if (!license_is_permissive(left)) return false;
        }
        return license_is_permissive(and_pos + 5);
    }

    return false;
}

static bool audit_path_is_doc_payload(const char *rel_path) {
    if (!rel_path || !rel_path[0]) return false;

    return str_starts_with(rel_path, "share/man/") ||
           strcmp(rel_path, "share/man") == 0 ||
           str_starts_with(rel_path, "share/doc/") ||
           strcmp(rel_path, "share/doc") == 0 ||
           str_starts_with(rel_path, "share/info/") ||
           strcmp(rel_path, "share/info") == 0 ||
           str_starts_with(rel_path, "man/") ||
           strcmp(rel_path, "man") == 0 ||
           str_starts_with(rel_path, "doc/") ||
           strcmp(rel_path, "doc") == 0 ||
           str_starts_with(rel_path, "info/") ||
           strcmp(rel_path, "info") == 0;
}

static bool audit_buffer_is_elf(const uint8_t *buf, size_t len) {
    return len >= 4 && buf[0] == 0x7f && buf[1] == 'E' &&
           buf[2] == 'L' && buf[3] == 'F';
}

static bool audit_buffer_is_text(const uint8_t *buf, size_t len) {
    for (size_t i = 0; i < len; i++) {
        if (buf[i] == '\0') return false;
    }
    return true;
}

static bool audit_file_contains_string(const char *path, const char *needle) {
    size_t needle_len = strlen(needle);
    if (needle_len == 0) return false;

    int fd = open(path, O_RDONLY);
    if (fd < 0) return false;

    size_t buf_size = 8192 + needle_len;
    uint8_t *buf = xmalloc(buf_size);
    size_t carry = 0;

    while (1) {
        ssize_t n = read(fd, buf + carry, buf_size - carry);
        if (n < 0) {
            if (errno == EINTR) continue;
            close(fd);
            free(buf);
            return false;
        }
        if (n == 0) break;

        size_t total = carry + (size_t)n;
        for (size_t i = 0; i + needle_len <= total; i++) {
            if (memcmp(buf + i, needle, needle_len) == 0) {
                close(fd);
                free(buf);
                return true;
            }
        }

        carry = needle_len > 1 ? needle_len - 1 : 0;
        if (carry > total) carry = total;
        if (carry > 0)
            memmove(buf, buf + total - carry, carry);
    }

    close(fd);
    free(buf);
    return false;
}

static tree_audit_result_t audit_layout_tree_recursive(const char *root,
                                                       const char *rel_path,
                                                       char *problem_path,
                                                       size_t problem_path_len) {
    char *scan_dir = rel_path[0] ? path_join(root, rel_path) : xstrdup(root);
    DIR *dir = opendir(scan_dir);
    if (!dir) {
        free(scan_dir);
        return TREE_AUDIT_OK;
    }

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0)
            continue;

        char *child_rel = rel_path[0] ? path_join(rel_path, ent->d_name)
                                      : xstrdup(ent->d_name);
        char *child_full = path_join(root, child_rel);

        struct stat st;
        if (lstat(child_full, &st) != 0) {
            free(child_rel);
            free(child_full);
            continue;
        }

        if (strncmp(child_rel, "lib64", 5) == 0 &&
            (child_rel[5] == '\0' || child_rel[5] == '/')) {
            snprintf(problem_path, problem_path_len, "/%s", child_rel);
            free(child_rel);
            free(child_full);
            closedir(dir);
            free(scan_dir);
            return TREE_AUDIT_LIB64_PATH;
        }

        if (!strchr(child_rel, '/') && str_ends_with(child_rel, ".0")) {
            snprintf(problem_path, problem_path_len, "/%s", child_rel);
            free(child_rel);
            free(child_full);
            closedir(dir);
            free(scan_dir);
            return TREE_AUDIT_ROOT_DOT_ZERO;
        }

        if (S_ISLNK(st.st_mode)) {
            char target[1024];
            ssize_t tlen = readlink(child_full, target, sizeof(target) - 1);
            if (tlen > 0) {
                target[tlen] = '\0';
                if (strstr(target, "/lib64") != NULL) {
                    snprintf(problem_path, problem_path_len, "/%s -> %s",
                             child_rel, target);
                    free(child_rel);
                    free(child_full);
                    closedir(dir);
                    free(scan_dir);
                    return TREE_AUDIT_LIB64_REFERENCE;
                }
            }
        } else if (S_ISREG(st.st_mode) && !audit_path_is_doc_payload(child_rel)) {
            uint8_t head[256];
            int fd = open(child_full, O_RDONLY);
            ssize_t n = -1;
            if (fd >= 0) {
                n = read(fd, head, sizeof(head));
                close(fd);
            }
            if (n > 0 &&
                (audit_buffer_is_elf(head, (size_t)n) ||
                 audit_buffer_is_text(head, (size_t)n)) &&
                audit_file_contains_string(child_full, "/lib64")) {
                snprintf(problem_path, problem_path_len, "/%s", child_rel);
                free(child_rel);
                free(child_full);
                closedir(dir);
                free(scan_dir);
                return TREE_AUDIT_LIB64_REFERENCE;
            }
        }

        if (S_ISDIR(st.st_mode)) {
            tree_audit_result_t rc = audit_layout_tree_recursive(root, child_rel,
                                                                 problem_path,
                                                                 problem_path_len);
            if (rc != TREE_AUDIT_OK) {
                free(child_rel);
                free(child_full);
                closedir(dir);
                free(scan_dir);
                return rc;
            }
        }

        free(child_rel);
        free(child_full);
    }

    closedir(dir);
    free(scan_dir);
    return TREE_AUDIT_OK;
}

tree_audit_result_t audit_layout_tree(const char *root, char *problem_path,
                                      size_t problem_path_len) {
    if (problem_path && problem_path_len > 0)
        problem_path[0] = '\0';
    if (!root || !dir_exists(root))
        return TREE_AUDIT_OK;
    return audit_layout_tree_recursive(root, "", problem_path, problem_path_len);
}

const char *audit_layout_result_string(tree_audit_result_t result) {
    switch (result) {
        case TREE_AUDIT_OK:
            return "ok";
        case TREE_AUDIT_ROOT_DOT_ZERO:
            return "root-level *.0 payload";
        case TREE_AUDIT_LIB64_PATH:
            return "staged /lib64 payload";
        case TREE_AUDIT_LIB64_REFERENCE:
            return "embedded /lib64 reference";
        case TREE_AUDIT_SBIN_PATH:
            return "staged /sbin payload (use /bin)";
        default:
            return "unknown layout audit failure";
    }
}
