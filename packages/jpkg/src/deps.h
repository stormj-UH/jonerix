/*
 * jpkg - jonerix package manager
 * deps.h - Dependency resolution (topological sort)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#ifndef JPKG_DEPS_H
#define JPKG_DEPS_H

#include "repo.h"
#include "db.h"
#include <stdbool.h>
#include <stddef.h>

/* Dependency resolution result */
typedef struct dep_list {
    char **packages;      /* package names in install order */
    size_t count;
    size_t capacity;
} dep_list_t;

/* Resolve dependencies for a package.
 * Returns an ordered list of packages to install (deps first).
 * Already-installed packages are excluded unless force=true.
 * Returns NULL on error (circular deps, missing package, etc). */
dep_list_t *deps_resolve(const repo_index_t *idx, const jpkg_db_t *db,
                         const char *package, bool force);

/* Resolve dependencies for multiple packages */
dep_list_t *deps_resolve_multi(const repo_index_t *idx, const jpkg_db_t *db,
                               const char **packages, size_t count, bool force);

/* Check for circular dependencies. Returns true if a cycle exists. */
bool deps_has_cycle(const repo_index_t *idx, const char *package);

/* Get the install order for removing a package (reverse deps first).
 * Returns list of packages that would be orphaned. */
dep_list_t *deps_removal_order(const jpkg_db_t *db, const char *package,
                               bool remove_orphans);

/* Free a dependency list */
void dep_list_free(dep_list_t *list);

/* Print the dependency tree (for debugging) */
void deps_print_tree(const repo_index_t *idx, const char *package, int depth);

#endif /* JPKG_DEPS_H */
