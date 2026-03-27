/*
 * jpkg - jonerix package manager
 * repo.h - Repository handling
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#ifndef JPKG_REPO_H
#define JPKG_REPO_H

#include "pkg.h"
#include <stdbool.h>
#include <stddef.h>

/* Repository mirror */
typedef struct repo_mirror {
    char *url;           /* e.g., "https://pkg.jonerix.org/v1/x86_64" */
    int priority;
    bool enabled;
    struct repo_mirror *next;
} repo_mirror_t;

/* Repository configuration */
typedef struct repo_config {
    repo_mirror_t *mirrors;
    size_t mirror_count;
    char *arch;          /* e.g., "x86_64" */
    char *cache_dir;     /* e.g., "/var/cache/jpkg" */
} repo_config_t;

/* An entry in the INDEX */
typedef struct repo_entry {
    char *name;
    char *version;
    char *license;
    char *description;
    char *arch;
    char *sha256;          /* hash of the .jpkg file */
    uint64_t size;
    char **runtime_deps;
    size_t runtime_dep_count;
    char **build_deps;
    size_t build_dep_count;
    struct repo_entry *next;
} repo_entry_t;

/* Parsed repository index */
typedef struct repo_index {
    repo_entry_t *entries;
    size_t entry_count;
    char *timestamp;      /* when the index was last generated */
} repo_index_t;

/* Load repository configuration from /etc/jpkg/repos.conf */
repo_config_t *repo_config_load(void);

/* Free repository configuration */
void repo_config_free(repo_config_t *cfg);

/* Fetch INDEX.zst from mirrors, verify signature, decompress & parse */
int repo_update(const repo_config_t *cfg);

/* Load the locally cached INDEX */
repo_index_t *repo_index_load(void);

/* Free a repository index */
void repo_index_free(repo_index_t *idx);

/* Find a package entry by name */
repo_entry_t *repo_find_package(const repo_index_t *idx, const char *name);

/* Search packages by query (substring match in name + description) */
repo_entry_t **repo_search(const repo_index_t *idx, const char *query,
                           size_t *result_count);

/* Free search results (but not the entries themselves) */
void repo_search_free(repo_entry_t **results);

/* Download a .jpkg file into cache, return the local path (caller frees) */
char *repo_fetch_package(const repo_config_t *cfg, const repo_entry_t *entry);

#endif /* JPKG_REPO_H */
