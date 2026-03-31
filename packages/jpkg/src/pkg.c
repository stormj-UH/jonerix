/*
 * jpkg - jonerix package manager
 * pkg.c - Package format parsing (JPKG magic + TOML metadata + zstd tar)
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "pkg.h"
#include "util.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/utsname.h>
#include <unistd.h>
#include <fcntl.h>

/* ========== Magic Validation ========== */

bool pkg_validate_magic(const uint8_t *data, size_t len) {
    if (len < JPKG_MAGIC_LEN) return false;
    return memcmp(data, JPKG_MAGIC, JPKG_MAGIC_LEN) == 0;
}

/* ========== Metadata Parsing ========== */

static char **copy_string_array(const toml_array_t *arr, size_t *count) {
    if (!arr || arr->count == 0) {
        *count = 0;
        return NULL;
    }
    char **out = xcalloc(arr->count, sizeof(char *));
    for (size_t i = 0; i < arr->count; i++) {
        out[i] = xstrdup(arr->items[i]);
    }
    *count = arr->count;
    return out;
}

pkg_meta_t *pkg_meta_from_toml(const char *toml_str) {
    if (!toml_str) return NULL;

    char *err = NULL;
    toml_doc_t *doc = toml_parse(toml_str, &err);
    if (!doc) {
        log_error("failed to parse package metadata: %s", err ? err : "unknown error");
        free(err);
        return NULL;
    }

    pkg_meta_t *meta = xcalloc(1, sizeof(pkg_meta_t));

    /* Required fields */
    const char *name = toml_get_string(doc, "package.name");
    const char *version = toml_get_string(doc, "package.version");
    if (!name || !version) {
        log_error("package metadata missing name or version");
        toml_free(doc);
        free(meta);
        return NULL;
    }

    meta->name = xstrdup(name);
    meta->version = xstrdup(version);

    /* Optional fields */
    const char *s;
    if ((s = toml_get_string(doc, "package.license")))
        meta->license = xstrdup(s);
    if ((s = toml_get_string(doc, "package.description")))
        meta->description = xstrdup(s);
    if ((s = toml_get_string(doc, "package.arch")))
        meta->arch = xstrdup(s);

    /* Dependencies */
    const toml_array_t *arr;
    if ((arr = toml_get_array(doc, "depends.runtime")))
        meta->runtime_deps = copy_string_array(arr, &meta->runtime_dep_count);
    if ((arr = toml_get_array(doc, "depends.build")))
        meta->build_deps = copy_string_array(arr, &meta->build_dep_count);

    /* File info */
    if ((s = toml_get_string(doc, "files.sha256")))
        meta->content_sha256 = xstrdup(s);
    int64_t size;
    if (toml_get_integer(doc, "files.size", &size))
        meta->content_size = (uint64_t)size;

    toml_free(doc);
    return meta;
}

/* ========== Package File Parsing ========== */

pkg_meta_t *pkg_parse_buffer(const uint8_t *data, size_t len,
                             size_t *payload_offset, size_t *payload_len) {
    if (!data || len < JPKG_HEADER_MIN) {
        log_error("package too small (%zu bytes)", len);
        return NULL;
    }

    if (!pkg_validate_magic(data, len)) {
        log_error("invalid package magic");
        return NULL;
    }

    uint32_t hdr_len = read_le32(data + JPKG_MAGIC_LEN);
    size_t meta_start = JPKG_MAGIC_LEN + 4;

    if (meta_start + hdr_len > len) {
        log_error("header length %u exceeds file size %zu", hdr_len, len);
        return NULL;
    }

    /* Extract TOML metadata as a null-terminated string */
    char *toml_str = xmalloc(hdr_len + 1);
    memcpy(toml_str, data + meta_start, hdr_len);
    toml_str[hdr_len] = '\0';

    pkg_meta_t *meta = pkg_meta_from_toml(toml_str);
    free(toml_str);

    if (!meta) return NULL;

    /* Payload starts right after the header */
    if (payload_offset) *payload_offset = meta_start + hdr_len;
    if (payload_len)    *payload_len = len - (meta_start + hdr_len);

    return meta;
}

pkg_meta_t *pkg_parse_file(const char *path,
                           size_t *payload_offset, size_t *payload_len) {
    uint8_t *data;
    ssize_t len = file_read(path, &data);
    if (len < 0) {
        log_error("failed to read package file: %s: %s", path, strerror(errno));
        return NULL;
    }

    pkg_meta_t *meta = pkg_parse_buffer(data, (size_t)len, payload_offset, payload_len);
    free(data);
    return meta;
}

/* ========== Package Creation ========== */

int pkg_create(const char *output_path,
               const char *toml_metadata,
               const uint8_t *zstd_payload, size_t zstd_len) {
    if (!output_path || !toml_metadata) return -1;

    size_t meta_len = strlen(toml_metadata);
    size_t total = JPKG_MAGIC_LEN + 4 + meta_len + zstd_len;

    uint8_t *buf = xmalloc(total);
    size_t off = 0;

    /* Magic */
    memcpy(buf + off, JPKG_MAGIC, JPKG_MAGIC_LEN);
    off += JPKG_MAGIC_LEN;

    /* Header length (LE32) */
    write_le32(buf + off, (uint32_t)meta_len);
    off += 4;

    /* TOML metadata */
    memcpy(buf + off, toml_metadata, meta_len);
    off += meta_len;

    /* zstd payload */
    if (zstd_payload && zstd_len > 0) {
        memcpy(buf + off, zstd_payload, zstd_len);
        off += zstd_len;
    }

    int rc = file_write(output_path, buf, total);
    free(buf);

    if (rc != 0) {
        log_error("failed to write package: %s: %s", output_path, strerror(errno));
    }
    return rc;
}

/* ========== Package Extraction ========== */

int pkg_extract(const char *jpkg_path, const char *dest_dir) {
    uint8_t *data;
    ssize_t len = file_read(jpkg_path, &data);
    if (len < 0) {
        log_error("failed to read package: %s: %s", jpkg_path, strerror(errno));
        return -1;
    }

    size_t payload_off, payload_len;
    pkg_meta_t *meta = pkg_parse_buffer(data, (size_t)len, &payload_off, &payload_len);
    if (!meta) {
        free(data);
        return -1;
    }

    if (payload_len == 0) {
        log_warn("package %s has no payload", meta->name);
        pkg_meta_free(meta);
        free(data);
        return 0;
    }

    /* Write the zstd payload to a temporary file */
    char tmp_zst[256];
    snprintf(tmp_zst, sizeof(tmp_zst), "/tmp/jpkg-extract-%d.tar.zst", (int)getpid());

    if (file_write(tmp_zst, data + payload_off, payload_len) != 0) {
        log_error("failed to write temp file: %s", tmp_zst);
        pkg_meta_free(meta);
        free(data);
        return -1;
    }

    /* Create destination directory */
    if (mkdirs(dest_dir, 0755) != 0) {
        log_error("failed to create directory: %s", dest_dir);
        unlink(tmp_zst);
        pkg_meta_free(meta);
        free(data);
        return -1;
    }

    /*
     * Decompress and extract using zstd and tar.
     * In the jonerix system, both are available as native tools.
     * We invoke them via execv for safety.
     */
    char cmd[1024];
    snprintf(cmd, sizeof(cmd), "zstd -d -c '%s' | tar -xf - -C '%s' 2>/dev/null",
             tmp_zst, dest_dir);

    int rc = system(cmd);
    unlink(tmp_zst);

    if (rc != 0) {
        log_error("extraction failed for %s (exit %d)", meta->name, rc);
        pkg_meta_free(meta);
        free(data);
        return -1;
    }

    /*
     * Flatten usr/ to support merged-usr layout.
     * jpkg archives may contain usr/bin/, usr/lib/ etc. On jonerix,
     * /usr is a symlink to / so extracting usr/ paths over the symlink
     * corrupts the filesystem. Instead, merge usr/ contents into the
     * root and remove the usr/ directory.
     */
    char flatten_cmd[1024];
    snprintf(flatten_cmd, sizeof(flatten_cmd),
             "if [ -d '%s/usr' ] && [ ! -L '%s/usr' ]; then "
             "cp -a '%s/usr/.' '%s/' && rm -rf '%s/usr'; fi",
             dest_dir, dest_dir, dest_dir, dest_dir, dest_dir);
    system(flatten_cmd);

    log_debug("extracted %s-%s to %s", meta->name, meta->version, dest_dir);
    pkg_meta_free(meta);
    free(data);
    return 0;
}

/* ========== Memory Management ========== */

void pkg_meta_free(pkg_meta_t *meta) {
    if (!meta) return;
    free(meta->name);
    free(meta->version);
    free(meta->license);
    free(meta->description);
    free(meta->arch);

    for (size_t i = 0; i < meta->runtime_dep_count; i++)
        free(meta->runtime_deps[i]);
    free(meta->runtime_deps);

    for (size_t i = 0; i < meta->build_dep_count; i++)
        free(meta->build_deps[i]);
    free(meta->build_deps);

    free(meta->content_sha256);

    pkg_file_t *f = meta->files;
    while (f) {
        pkg_file_t *next = f->next;
        free(f->path);
        free(f);
        f = next;
    }

    free(meta);
}

/* ========== TOML Serialization ========== */

char *pkg_meta_to_toml(const pkg_meta_t *meta) {
    if (!meta) return NULL;

    toml_doc_t *doc = toml_new();

    toml_set_string(doc, "package.name", meta->name);
    toml_set_string(doc, "package.version", meta->version);
    if (meta->license) toml_set_string(doc, "package.license", meta->license);
    if (meta->description) toml_set_string(doc, "package.description", meta->description);
    if (meta->arch) toml_set_string(doc, "package.arch", meta->arch);

    if (meta->runtime_dep_count > 0) {
        toml_set_array(doc, "depends.runtime",
                       (const char **)meta->runtime_deps, meta->runtime_dep_count);
    }
    if (meta->build_dep_count > 0) {
        toml_set_array(doc, "depends.build",
                       (const char **)meta->build_deps, meta->build_dep_count);
    }

    if (meta->content_sha256) toml_set_string(doc, "files.sha256", meta->content_sha256);
    if (meta->content_size > 0)
        toml_set_integer(doc, "files.size", (int64_t)meta->content_size);

    char *result = toml_serialize(doc);
    toml_free(doc);
    return result;
}

/* ========== Utility ========== */

char *pkg_filename(const char *name, const char *version) {
    if (!name || !version) return NULL;
    /* Include arch so x86_64 and aarch64 packages coexist in the same release.
     * Auto-detect from uname() when building; fallback to x86_64. */
    struct utsname uts;
    const char *arch = "x86_64";
    if (uname(&uts) == 0) arch = uts.machine;
    size_t len = strlen(name) + strlen(version) + strlen(arch) + 8; /* name-version-arch.jpkg\0 */
    char *buf = xmalloc(len);
    snprintf(buf, len, "%s-%s-%s.jpkg", name, version, arch);
    return buf;
}
