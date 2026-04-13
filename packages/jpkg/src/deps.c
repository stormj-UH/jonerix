/*
 * jpkg - jonerix package manager
 * deps.c - Dependency resolution (topological sort)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "deps.h"
#include "util.h"
#include <string.h>
#include <stdio.h>

/* ========== Internal Data Structures ========== */

/* Node state for DFS-based topological sort */
typedef enum {
    NODE_UNVISITED = 0,
    NODE_VISITING,   /* currently on the DFS stack (cycle detection) */
    NODE_VISITED
} node_state_t;

typedef struct dep_node {
    const char *name;
    node_state_t state;
    const repo_entry_t *entry;
} dep_node_t;

typedef struct dep_graph {
    dep_node_t *nodes;
    size_t count;
    size_t capacity;
} dep_graph_t;

/* ========== Graph Operations ========== */

static dep_graph_t *graph_new(void) {
    dep_graph_t *g = xcalloc(1, sizeof(dep_graph_t));
    g->capacity = 64;
    g->nodes = xcalloc(g->capacity, sizeof(dep_node_t));
    return g;
}

static void graph_free(dep_graph_t *g) {
    if (!g) return;
    free(g->nodes);
    free(g);
}

static dep_node_t *graph_find(dep_graph_t *g, const char *name) {
    for (size_t i = 0; i < g->count; i++) {
        if (strcmp(g->nodes[i].name, name) == 0)
            return &g->nodes[i];
    }
    return NULL;
}

static dep_node_t *graph_add(dep_graph_t *g, const char *name,
                             const repo_entry_t *entry) {
    dep_node_t *existing = graph_find(g, name);
    if (existing) return existing;

    if (g->count >= g->capacity) {
        g->capacity *= 2;
        g->nodes = xrealloc(g->nodes, g->capacity * sizeof(dep_node_t));
    }

    dep_node_t *n = &g->nodes[g->count++];
    n->name = name;
    n->state = NODE_UNVISITED;
    n->entry = entry;
    return n;
}

/* ========== Dependency List ========== */

static dep_list_t *dep_list_new(void) {
    dep_list_t *list = xcalloc(1, sizeof(dep_list_t));
    list->capacity = 32;
    list->packages = xcalloc(list->capacity, sizeof(char *));
    return list;
}

static void dep_list_append(dep_list_t *list, const char *name) {
    /* Check for duplicates */
    for (size_t i = 0; i < list->count; i++) {
        if (strcmp(list->packages[i], name) == 0) return;
    }

    if (list->count >= list->capacity) {
        list->capacity *= 2;
        list->packages = xrealloc(list->packages, list->capacity * sizeof(char *));
    }
    list->packages[list->count++] = xstrdup(name);
}

static bool should_include_node(const dep_node_t *node, const jpkg_db_t *db, bool force) {
    if (force || !node)
        return true;
    if (!db)
        return true;

    db_pkg_t *installed = db_get_package(db, node->name);
    if (!installed)
        return true;

    /* Package is installed — don't include in install list.
     * Use 'jpkg upgrade' to update to a newer version. */
    return false;
}

void dep_list_free(dep_list_t *list) {
    if (!list) return;
    for (size_t i = 0; i < list->count; i++)
        free(list->packages[i]);
    free(list->packages);
    free(list);
}

/* ========== Topological Sort (DFS) ========== */

static int topo_visit(dep_graph_t *g, dep_node_t *node,
                      const repo_index_t *idx, const jpkg_db_t *db,
                      dep_list_t *result, bool force) {
    if (node->state == NODE_VISITED) return 0;

    if (node->state == NODE_VISITING) {
        log_error("circular dependency detected involving: %s", node->name);
        return -1;
    }

    node->state = NODE_VISITING;

    /* Process dependencies first */
    if (node->entry) {
        for (size_t i = 0; i < node->entry->runtime_dep_count; i++) {
            const char *dep_name = node->entry->runtime_deps[i];

            /* Find dependency in index */
            repo_entry_t *dep_entry = repo_find_package(idx, dep_name);
            if (!dep_entry) {
                /* Not in index — if already installed in jpkg db, skip it.
                 * Otherwise warn and continue: it may be a base system package
                 * (e.g. musl on Alpine) not tracked by our repository. */
                if (db && db_is_installed(db, dep_name)) {
                    continue;
                }
                log_warn("dependency '%s' (required by %s) not found in index"
                         " — assuming provided by the base system",
                         dep_name, node->name);
                continue;
            }

            dep_node_t *dep_node = graph_add(g, dep_entry->name, dep_entry);
            int rc = topo_visit(g, dep_node, idx, db, result, force);
            if (rc != 0) return rc;
        }
    }

    node->state = NODE_VISITED;

    /* Add to result if missing, forced, or the repo has a newer version. */
    if (should_include_node(node, db, force)) {
        dep_list_append(result, node->name);
    }

    return 0;
}

/* ========== Public API ========== */

dep_list_t *deps_resolve(const repo_index_t *idx, const jpkg_db_t *db,
                         const char *package, bool force) {
    if (!idx || !package) return NULL;

    repo_entry_t *entry = repo_find_package(idx, package);
    if (!entry) {
        log_error("package not found in repository: %s", package);
        return NULL;
    }

    dep_graph_t *g = graph_new();
    dep_list_t *result = dep_list_new();

    dep_node_t *root = graph_add(g, entry->name, entry);
    int rc = topo_visit(g, root, idx, db, result, force);

    graph_free(g);

    if (rc != 0) {
        dep_list_free(result);
        return NULL;
    }

    return result;
}

dep_list_t *deps_resolve_multi(const repo_index_t *idx, const jpkg_db_t *db,
                               const char **packages, size_t count, bool force) {
    if (!idx || !packages || count == 0) return NULL;

    dep_graph_t *g = graph_new();
    dep_list_t *result = dep_list_new();

    for (size_t i = 0; i < count; i++) {
        repo_entry_t *entry = repo_find_package(idx, packages[i]);
        if (!entry) {
            log_error("package not found: %s", packages[i]);
            dep_list_free(result);
            graph_free(g);
            return NULL;
        }

        dep_node_t *node = graph_add(g, entry->name, entry);
        int rc = topo_visit(g, node, idx, db, result, force);
        if (rc != 0) {
            dep_list_free(result);
            graph_free(g);
            return NULL;
        }
    }

    graph_free(g);
    return result;
}

bool deps_has_cycle(const repo_index_t *idx, const char *package) {
    if (!idx || !package) return false;

    repo_entry_t *entry = repo_find_package(idx, package);
    if (!entry) return false;

    dep_graph_t *g = graph_new();
    dep_list_t *result = dep_list_new();

    dep_node_t *root = graph_add(g, entry->name, entry);
    int rc = topo_visit(g, root, idx, NULL, result, true);

    graph_free(g);
    dep_list_free(result);

    return rc != 0;
}

dep_list_t *deps_removal_order(const jpkg_db_t *db, const char *package,
                               bool remove_orphans) {
    if (!db || !package) return NULL;

    dep_list_t *result = dep_list_new();

    /* First, add the package itself */
    dep_list_append(result, package);

    if (!remove_orphans) return result;

    /*
     * Find orphaned dependencies: packages that were dependencies of the
     * removed package and are not required by any other installed package.
     */
    db_pkg_t *pkg = db_get_package(db, package);
    if (!pkg) return result;

    for (size_t i = 0; i < pkg->runtime_dep_count; i++) {
        const char *dep_name = pkg->runtime_deps[i];

        /* Check if any other installed package needs this dependency */
        bool needed = false;
        for (db_pkg_t *p = db->packages; p; p = p->next) {
            if (strcmp(p->name, package) == 0) continue;
            /* Also skip packages we're already removing */
            bool removing = false;
            for (size_t j = 0; j < result->count; j++) {
                if (strcmp(result->packages[j], p->name) == 0) {
                    removing = true;
                    break;
                }
            }
            if (removing) continue;

            for (size_t j = 0; j < p->runtime_dep_count; j++) {
                if (strcmp(p->runtime_deps[j], dep_name) == 0) {
                    needed = true;
                    break;
                }
            }
            if (needed) break;
        }

        if (!needed && db_is_installed(db, dep_name)) {
            dep_list_append(result, dep_name);
        }
    }

    return result;
}

void deps_print_tree(const repo_index_t *idx, const char *package, int depth) {
    if (!idx || !package) return;
    if (depth > 20) { /* Prevent infinite recursion */
        printf("%*s...\n", depth * 2, "");
        return;
    }

    repo_entry_t *entry = repo_find_package(idx, package);
    if (!entry) {
        printf("%*s%s (not found)\n", depth * 2, "", package);
        return;
    }

    printf("%*s%s-%s\n", depth * 2, "", entry->name, entry->version);

    for (size_t i = 0; i < entry->runtime_dep_count; i++) {
        deps_print_tree(idx, entry->runtime_deps[i], depth + 1);
    }
}
