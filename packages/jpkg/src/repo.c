/*
 * jpkg - jonerix package manager
 * repo.c - Repository handling (fetch/parse INDEX.zst)
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#include "repo.h"
#include "fetch.h"
#include "sign.h"
#include "toml.h"
#include "util.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <ctype.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/utsname.h>

/* ========== Configuration ========== */

repo_config_t *repo_config_load(void) {
    repo_config_t *cfg = xcalloc(1, sizeof(repo_config_t));
    /* Detect architecture at runtime */
    struct utsname uts;
    if (uname(&uts) == 0)
        cfg->arch = xstrdup(uts.machine);
    else
        cfg->arch = xstrdup("unknown");

    /* Default cache directory */
    char cache[512];
    snprintf(cache, sizeof(cache), "%s%s", g_rootfs, JPKG_CACHE_DIR);
    cfg->cache_dir = xstrdup(cache);

    /* Load config from /etc/jpkg/repos.conf */
    char conf_path[512];
    snprintf(conf_path, sizeof(conf_path), "%s%s/repos.conf", g_rootfs, JPKG_CONFIG_DIR);

    uint8_t *data = NULL;
    ssize_t len = file_read(conf_path, &data);
    if (len > 0) {
        char *err = NULL;
        toml_doc_t *doc = toml_parse((const char *)data, &err);
        free(data);

        if (doc) {
            /* Parse architecture */
            const char *arch = toml_get_string(doc, "general.arch");
            if (arch) {
                free(cfg->arch);
                cfg->arch = xstrdup(arch);
            }

            /* Parse mirrors: look for repo.url and repo.priority keys,
             * or multiple [repo.<name>] sections */
            const char *url = toml_get_string(doc, "repo.url");
            if (url) {
                repo_mirror_t *m = xcalloc(1, sizeof(repo_mirror_t));
                m->url = xstrdup(url);
                m->priority = 100;
                m->enabled = true;
                cfg->mirrors = m;
                cfg->mirror_count = 1;
            }

            /* Parse additional mirrors from repo.mirrors array */
            const toml_array_t *mirrors = toml_get_array(doc, "repo.mirrors");
            if (mirrors) {
                for (size_t i = 0; i < mirrors->count; i++) {
                    repo_mirror_t *m = xcalloc(1, sizeof(repo_mirror_t));
                    m->url = xstrdup(mirrors->items[i]);
                    m->priority = (int)(100 + i);
                    m->enabled = true;
                    m->next = cfg->mirrors;
                    cfg->mirrors = m;
                    cfg->mirror_count++;
                }
            }

            toml_free(doc);
        } else {
            if (err) {
                log_warn("failed to parse repos.conf: %s", err);
                free(err);
            }
        }
    } else {
        free(data); /* file_read may have allocated even on failure */
    }

    /* If no mirrors configured, use default */
    if (cfg->mirror_count == 0) {
        repo_mirror_t *m = xcalloc(1, sizeof(repo_mirror_t));
        m->url = xstrdup("https://github.com/stormj-UH/jonerix/releases/download/packages");
        m->priority = 100;
        m->enabled = true;
        cfg->mirrors = m;
        cfg->mirror_count = 1;
    }

    return cfg;
}

void repo_config_free(repo_config_t *cfg) {
    if (!cfg) return;
    repo_mirror_t *m = cfg->mirrors;
    while (m) {
        repo_mirror_t *next = m->next;
        free(m->url);
        free(m);
        m = next;
    }
    free(cfg->arch);
    free(cfg->cache_dir);
    free(cfg);
}

/* ========== INDEX Parsing ========== */

/*
 * The INDEX file is TOML with repeated sections like:
 *
 * [[packages]]
 * name = "toybox"
 * version = "0.8.11"
 * ...
 *
 * But since our minimal TOML parser doesn't support arrays of tables,
 * we use a line-based format where entries are separated by blank lines:
 *
 * [toybox]
 * version = "0.8.11"
 * license = "0BSD"
 * description = "BSD-licensed replacement for BusyBox"
 * arch = "x86_64"
 * sha256 = "abc123..."
 * size = 245760
 * depends = ["musl"]
 * build-depends = ["clang", "samurai"]
 *
 * [mksh]
 * ...
 */

static repo_entry_t *parse_index_entry(toml_doc_t *doc, const char *name) {
    repo_entry_t *e = xcalloc(1, sizeof(repo_entry_t));
    e->name = xstrdup(name);

    char key[256];

    snprintf(key, sizeof(key), "%s.version", name);
    const char *s = toml_get_string(doc, key);
    e->version = xstrdup(s ? s : "0");

    snprintf(key, sizeof(key), "%s.license", name);
    s = toml_get_string(doc, key);
    e->license = s ? xstrdup(s) : xstrdup("unknown");

    snprintf(key, sizeof(key), "%s.description", name);
    s = toml_get_string(doc, key);
    e->description = s ? xstrdup(s) : xstrdup("");

    snprintf(key, sizeof(key), "%s.arch", name);
    s = toml_get_string(doc, key);
    e->arch = s ? xstrdup(s) : xstrdup("x86_64");

    snprintf(key, sizeof(key), "%s.sha256", name);
    s = toml_get_string(doc, key);
    e->sha256 = s ? xstrdup(s) : xstrdup("");

    snprintf(key, sizeof(key), "%s.size", name);
    int64_t size;
    if (toml_get_integer(doc, key, &size))
        e->size = (uint64_t)size;

    snprintf(key, sizeof(key), "%s.depends", name);
    const toml_array_t *arr = toml_get_array(doc, key);
    if (arr && arr->count > 0) {
        e->runtime_deps = xcalloc(arr->count, sizeof(char *));
        e->runtime_dep_count = arr->count;
        for (size_t i = 0; i < arr->count; i++)
            e->runtime_deps[i] = xstrdup(arr->items[i]);
    }

    snprintf(key, sizeof(key), "%s.build-depends", name);
    arr = toml_get_array(doc, key);
    if (arr && arr->count > 0) {
        e->build_deps = xcalloc(arr->count, sizeof(char *));
        e->build_dep_count = arr->count;
        for (size_t i = 0; i < arr->count; i++)
            e->build_deps[i] = xstrdup(arr->items[i]);
    }

    return e;
}

static void entry_free(repo_entry_t *e) {
    if (!e) return;
    free(e->name);
    free(e->version);
    free(e->license);
    free(e->description);
    free(e->arch);
    free(e->sha256);
    for (size_t i = 0; i < e->runtime_dep_count; i++) free(e->runtime_deps[i]);
    free(e->runtime_deps);
    for (size_t i = 0; i < e->build_dep_count; i++) free(e->build_deps[i]);
    free(e->build_deps);
    free(e);
}

repo_index_t *repo_index_load(void) {
    char index_path[512];
    snprintf(index_path, sizeof(index_path), "%s%s/INDEX", g_rootfs, JPKG_CACHE_DIR);

    uint8_t *data;
    ssize_t len = file_read(index_path, &data);
    if (len <= 0) {
        log_error("no cached INDEX found. Run 'jpkg update' first.");
        return NULL;
    }

    char *err = NULL;
    toml_doc_t *doc = toml_parse((const char *)data, &err);
    free(data);

    if (!doc) {
        log_error("failed to parse INDEX: %s", err ? err : "unknown error");
        free(err);
        return NULL;
    }

    repo_index_t *idx = xcalloc(1, sizeof(repo_index_t));

    /* Extract timestamp if present */
    const char *ts = toml_get_string(doc, "meta.timestamp");
    if (ts) idx->timestamp = xstrdup(ts);

    /* Walk through all values to collect unique section names (=package names) */
    /* We track seen sections by scanning through the value keys */
    size_t seen_cap = 128;
    size_t seen_count = 0;
    char **seen = xcalloc(seen_cap, sizeof(char *));

    for (toml_value_t *v = doc->head; v; v = v->next) {
        /* Extract section name from key (before the dot) */
        const char *dot = strchr(v->key, '.');
        if (!dot) continue;

        size_t slen = (size_t)(dot - v->key);
        /* Skip "meta" section */
        if (slen == 4 && memcmp(v->key, "meta", 4) == 0) continue;

        /* Check if we've already seen this section */
        bool found = false;
        for (size_t i = 0; i < seen_count; i++) {
            if (strlen(seen[i]) == slen && memcmp(seen[i], v->key, slen) == 0) {
                found = true;
                break;
            }
        }
        if (found) continue;

        if (seen_count >= seen_cap) {
            seen_cap *= 2;
            seen = xrealloc(seen, seen_cap * sizeof(char *));
        }
        seen[seen_count] = xstrndup(v->key, slen);
        seen_count++;
    }

    /* Now parse each package section */
    for (size_t i = 0; i < seen_count; i++) {
        repo_entry_t *e = parse_index_entry(doc, seen[i]);
        e->next = idx->entries;
        idx->entries = e;
        idx->entry_count++;
        free(seen[i]);
    }
    free(seen);
    toml_free(doc);

    log_debug("loaded INDEX with %zu packages", idx->entry_count);
    return idx;
}

void repo_index_free(repo_index_t *idx) {
    if (!idx) return;
    free(idx->timestamp);
    repo_entry_t *e = idx->entries;
    while (e) {
        repo_entry_t *next = e->next;
        entry_free(e);
        e = next;
    }
    free(idx);
}

/* ========== Repository Update ========== */

int repo_update(const repo_config_t *cfg) {
    if (!cfg || !cfg->mirrors) {
        log_error("no repository mirrors configured");
        return -1;
    }

    /* Ensure cache directory exists */
    mkdirs(cfg->cache_dir, 0755);

    char index_path[512];
    snprintf(index_path, sizeof(index_path), "%s/INDEX.zst", cfg->cache_dir);
    char sig_path[512];
    snprintf(sig_path, sizeof(sig_path), "%s/INDEX.zst.sig", cfg->cache_dir);
    char out_path[512];
    snprintf(out_path, sizeof(out_path), "%s/INDEX", cfg->cache_dir);

    /* Try each mirror */
    bool fetched = false;
    for (repo_mirror_t *m = cfg->mirrors; m; m = m->next) {
        if (!m->enabled) continue;

        char url[2048];
        snprintf(url, sizeof(url), "%s/INDEX.zst", m->url);

        log_info("fetching INDEX from %s", m->url);

        if (fetch_to_file(url, index_path) == 0) {
            /* Also fetch signature */
            snprintf(url, sizeof(url), "%s/INDEX.zst.sig", m->url);
            if (fetch_to_file(url, sig_path) != 0) {
                log_warn("failed to fetch INDEX signature from %s", m->url);
                /* Continue - verification will fail if keys are loaded */
            }
            fetched = true;
            break;
        }
        log_warn("mirror %s failed, trying next...", m->url);
    }

    if (!fetched) {
        log_error("failed to fetch INDEX from any mirror");
        return -1;
    }

    /* Check that the downloaded file actually exists and has content */
    struct stat st;
    if (stat(index_path, &st) != 0 || st.st_size == 0) {
        log_error("downloaded INDEX.zst is empty or missing");
        return -1;
    }

    /* Decompress: zstd -d INDEX.zst > INDEX */
    char cmd[1024];
    snprintf(cmd, sizeof(cmd), "zstd -d -f -o '%s' '%s' 2>/dev/null", out_path, index_path);
    int rc = system(cmd);
    if (rc != 0) {
        /* Maybe it's not actually compressed — try copying as-is */
        log_debug("zstd decompress failed, assuming plain text INDEX");
        uint8_t *raw_data = NULL;
        ssize_t raw_len = file_read(index_path, &raw_data);
        if (raw_len <= 0 || !raw_data) {
            log_error("failed to read downloaded INDEX");
            free(raw_data);
            return -1;
        }
        if (file_write(out_path, raw_data, (size_t)raw_len) != 0) {
            log_error("failed to write INDEX");
            free(raw_data);
            return -1;
        }
        free(raw_data);
    }

    /* Optionally verify signature */
    uint8_t *sig_data = NULL;
    ssize_t sig_len = file_read(sig_path, &sig_data);
    if (sig_len > 0 && sig_data) {
        uint8_t *idx_data = NULL;
        ssize_t idx_len = file_read(index_path, &idx_data);
        if (idx_len > 0 && idx_data) {
            if (!sign_verify_detached(idx_data, (size_t)idx_len,
                                      sig_data, (size_t)sig_len)) {
                log_warn("INDEX signature verification failed (continuing anyway)");
            } else {
                log_info("INDEX signature verified");
            }
            free(idx_data);
        }
        free(sig_data);
    }
    log_info("package index updated successfully");
    return 0;
}

/* ========== Package Lookup ========== */

repo_entry_t *repo_find_package(const repo_index_t *idx, const char *name) {
    if (!idx || !name) return NULL;

    /* Try arch-qualified key first: "name-arch" (e.g. "rust-aarch64"). */
    struct utsname uts;
    if (uname(&uts) == 0) {
        char arch_key[256];
        snprintf(arch_key, sizeof(arch_key), "%s-%s", name, uts.machine);
        for (repo_entry_t *e = idx->entries; e; e = e->next) {
            if (strcmp(e->name, arch_key) == 0) return e;
        }
    }

    /* Fall back to unqualified name for backward-compat. */
    for (repo_entry_t *e = idx->entries; e; e = e->next) {
        if (strcmp(e->name, name) == 0) return e;
    }
    return NULL;
}

repo_entry_t **repo_search(const repo_index_t *idx, const char *query,
                           size_t *result_count) {
    if (!idx || !query || !result_count) return NULL;

    size_t cap = 32;
    repo_entry_t **results = xcalloc(cap, sizeof(repo_entry_t *));
    *result_count = 0;

    /* Case-insensitive substring search */
    char *lower_query = xstrdup(query);
    for (char *p = lower_query; *p; p++) *p = (char)tolower((unsigned char)*p);

    for (repo_entry_t *e = idx->entries; e; e = e->next) {
        char *lower_name = xstrdup(e->name);
        for (char *p = lower_name; *p; p++) *p = (char)tolower((unsigned char)*p);

        char *lower_desc = xstrdup(e->description ? e->description : "");
        for (char *p = lower_desc; *p; p++) *p = (char)tolower((unsigned char)*p);

        if (strstr(lower_name, lower_query) || strstr(lower_desc, lower_query)) {
            if (*result_count >= cap) {
                cap *= 2;
                results = xrealloc(results, cap * sizeof(repo_entry_t *));
            }
            results[*result_count] = e;
            (*result_count)++;
        }

        free(lower_name);
        free(lower_desc);
    }

    free(lower_query);
    return results;
}

void repo_search_free(repo_entry_t **results) {
    free(results);
}

/* ========== Package Download ========== */

char *repo_fetch_package(const repo_config_t *cfg, const repo_entry_t *entry) {
    if (!cfg || !entry || !cfg->mirrors) return NULL;

    /* Package filename: name-version.jpkg */
    char *filename = pkg_filename(entry->name, entry->version);
    char *local_path = path_join(cfg->cache_dir, filename);

    /* Check if already cached */
    if (file_exists(local_path)) {
        /* Verify hash if available */
        if (entry->sha256 && entry->sha256[0]) {
            char hash[65];
            if (sha256_file(local_path, hash) == 0 &&
                strcmp(hash, entry->sha256) == 0) {
                log_debug("using cached %s", filename);
                free(filename);
                return local_path;
            }
            log_debug("cached %s has wrong hash, re-downloading", filename);
        } else {
            free(filename);
            return local_path;
        }
    }

    /* Download from mirrors */
    mkdirs(cfg->cache_dir, 0755);

    for (repo_mirror_t *m = cfg->mirrors; m; m = m->next) {
        if (!m->enabled) continue;

        char url[2048];
        snprintf(url, sizeof(url), "%s/%s", m->url, filename);

        log_info("downloading %s", filename);
        if (fetch_to_file(url, local_path) == 0) {
            /* Verify hash */
            if (entry->sha256 && entry->sha256[0]) {
                char hash[65];
                if (sha256_file(local_path, hash) == 0 &&
                    strcmp(hash, entry->sha256) == 0) {
                    log_debug("hash verified for %s", filename);
                } else {
                    log_error("hash mismatch for %s", filename);
                    unlink(local_path);
                    continue;
                }
            }
            free(filename);
            return local_path;
        }
    }

    log_error("failed to download %s from any mirror", filename);
    free(filename);
    free(local_path);
    return NULL;
}
