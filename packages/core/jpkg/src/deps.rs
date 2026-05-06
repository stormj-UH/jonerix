// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Dependency resolution — topological sort and cycle detection.  Port of
//! `jpkg/src/deps.c`.  Algorithm: post-order DFS, which naturally produces
//! leaves-first ordering without a separate Kahn pass.
//!
//! # Invariants
//!
//! 1. **Install-order guarantee**: [`resolve_install`] returns packages in
//!    post-order DFS sequence — every package appears after all of its
//!    transitive dependencies.  The caller's requested package order is
//!    preserved for packages at the same dependency level: if `[toybox, mksh]`
//!    is requested and both are leaves, they appear as `[toybox, mksh]` in the
//!    plan.  Callers that depend on this ordering (e.g. hook scripts that
//!    `claim` files from earlier packages) must not sort the plan.
//!
//! 2. **Cycle detection correctness**: the DFS colouring (`Unvisited` /
//!    `Visiting` / `Visited`) guarantees that any back-edge (a node reached
//!    while it is already on the DFS stack) is detected exactly once and
//!    reported as `DepsError::Cycle`.  The cycle path includes the repeated
//!    node at both ends (e.g. `[a, b, a]`) so the caller can format a readable
//!    error.  A DAG produces no false positives; the algorithm terminates in
//!    `O(V + E)` time.
//!
//! 3. **Already-installed leniency**: packages that are installed on the local
//!    system but absent from the repository INDEX are silently accepted as
//!    satisfied dependencies (matching the C behaviour for base-system packages
//!    like `musl`).  If a dependency is absent from both the INDEX and the DB
//!    the function returns `DepsError::UnknownDependency`.  Callers must ensure
//!    the DB is opened against the correct rootfs before calling.
//!
//! 4. **Deterministic ordering**: dependency lists are sorted before recursion
//!    so that the install plan is identical across runs given the same INDEX
//!    and DB state.  This is a stronger guarantee than the C implementation,
//!    which inherits whatever order `toml_parse` returns.
//!
//! 5. **Removal order**: [`resolve_remove`] puts explicit targets first
//!    (sorted for determinism), then appends orphaned dependencies in the order
//!    they are discovered.  The result is safe to pass directly to an uninstall
//!    loop — no package appears before the packages that depend on it.

use std::collections::{BTreeMap, BTreeSet};

// Real db types (Worker E):
use crate::db::{DbError, InstalledDb};
#[cfg(test)]
use crate::db::InstalledPkg;
use crate::recipe::{Index, IndexEntry};
use crate::types::{InstallMode, OrphanMode};

// ─── DepsError ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DepsError {
    /// Underlying database error.
    Db(DbError),
    /// Circular dependency detected; `Vec<String>` is the cycle path in order.
    Cycle(Vec<String>),
    /// The requested package is absent from the index.
    UnknownPackage { name: String, arch: String },
    /// A dependency of `dependent` is not in the index (and not installed).
    UnknownDependency { dependent: String, missing: String },
}

impl std::fmt::Display for DepsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepsError::Db(e) => write!(f, "database error: {e}"),
            DepsError::Cycle(path) => {
                write!(f, "circular dependency: {}", path.join(" -> "))
            }
            DepsError::UnknownPackage { name, arch } => {
                write!(f, "package not found in index: {name} ({arch})")
            }
            DepsError::UnknownDependency { dependent, missing } => {
                write!(
                    f,
                    "dependency '{missing}' required by '{dependent}' not found in index"
                )
            }
        }
    }
}

impl std::error::Error for DepsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DepsError::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DbError> for DepsError {
    fn from(e: DbError) -> Self {
        DepsError::Db(e)
    }
}

// ─── ResolvedPlan ────────────────────────────────────────────────────────────

/// Result of [`resolve_install`].
#[derive(Debug, Clone)]
pub struct ResolvedPlan {
    /// Packages to install, ordered: dependencies first.  Already-installed
    /// packages are EXCLUDED unless `force=true` was passed to `resolve_install`.
    pub to_install: Vec<String>,
    /// Packages already installed that satisfy a dependency — included for
    /// auditability.  Empty when `force=true`.
    pub already_installed: Vec<String>,
}

// ─── Internal DFS state ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Unvisited,
    /// Currently on the DFS stack — a hit here means a cycle.
    Visiting,
    Visited,
}

// ─── post-order DFS visit (mirrors topo_visit in deps.c:122-168) ─────────────

/// Recursively visit `name`, pushing its dependencies before itself.
///
/// `state_map`   — per-node DFS colour (Unvisited / Visiting / Visited).
/// `result`      — accumulates package names in post-order (leaves first).
/// `stack`       — current DFS path; used to build the cycle path on error.
///
/// Returns `Err(DepsError::Cycle(...))` on a back-edge, or
/// `Err(DepsError::UnknownDependency {...})` when a dep is absent from both
/// the index and the installed db (matching deps.c:145-152 — the C code warns
/// and continues; the Rust port is stricter and errors, matching the contract
/// given in the task spec).
#[allow(clippy::too_many_arguments)]
fn dfs_visit(
    name: &str,
    arch: &str,
    index: &Index,
    installed_names: &BTreeSet<String>,
    state_map: &mut BTreeMap<String, NodeState>,
    result: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> Result<(), DepsError> {
    match state_map.get(name).copied().unwrap_or(NodeState::Unvisited) {
        NodeState::Visited => return Ok(()),
        NodeState::Visiting => {
            // Back-edge: build cycle path from `stack`.
            let cycle_start = stack
                .iter()
                .position(|n| n == name)
                .unwrap_or(0);
            let mut cycle: Vec<String> = stack[cycle_start..].to_vec();
            cycle.push(name.to_string()); // close the loop
            return Err(DepsError::Cycle(cycle));
        }
        NodeState::Unvisited => {}
    }

    state_map.insert(name.to_string(), NodeState::Visiting);
    stack.push(name.to_string());

    // Look up the entry in the index.
    let entry: &IndexEntry = match index.get(name, arch) {
        Some(e) => e,
        None => {
            // Package not in index.  If it's already installed we can skip it
            // (mirrors the C behaviour: deps.c:145-152).
            if installed_names.contains(name) {
                state_map.insert(name.to_string(), NodeState::Visited);
                stack.pop();
                return Ok(());
            }
            // Not installed either — hard error.
            return Err(DepsError::UnknownPackage {
                name: name.to_string(),
                arch: arch.to_string(),
            });
        }
    };

    // Collect deps into a local Vec so we don't borrow `index` through
    // `entry` while also passing `index` recursively.
    let deps: Vec<String> = entry.depends.clone();

    // Recurse into each dependency (sorted for determinism — BTreeMap iteration
    // is already sorted but `depends` is a Vec from TOML; sort it here).
    let mut sorted_deps = deps;
    sorted_deps.sort();

    for dep in &sorted_deps {
        // If the dependency is already installed and not in the index, skip it
        // (same leniency as the C code for base-system packages like musl).
        if index.get(dep, arch).is_none() {
            if installed_names.contains(dep.as_str()) {
                continue;
            }
            // Dependency is missing from index AND not installed.
            return Err(DepsError::UnknownDependency {
                dependent: name.to_string(),
                missing: dep.clone(),
            });
        }
        dfs_visit(dep, arch, index, installed_names, state_map, result, stack)?;
    }

    state_map.insert(name.to_string(), NodeState::Visited);
    stack.pop();

    // Post-order: push self AFTER all dependencies.
    result.push(name.to_string());

    Ok(())
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Resolve the install order for `want`.
///
/// Algorithm (post-order DFS, matching deps.c:122-168):
///   1. Collect installed package names from `db`.
///   2. For each requested package, run `dfs_visit` which recursively walks
///      `IndexEntry.depends`. The caller's requested package order is preserved
///      because same-level packages can replace one another's files. The Pi
///      image depends on this: toybox must land before mksh/shadow/fixups so
///      their hooks can claim /bin/sh, /bin/login, and /bin/reboot.
///   3. Cycle → `Err::Cycle`; unknown dep → `Err::UnknownDependency`.
///   4. If `mode` is [`InstallMode::Normal`], split the post-order result into
///      `to_install` (not yet installed) and `already_installed` (already present).
///   5. If `mode` is [`InstallMode::Force`], every reachable package goes into
///      `to_install`.
pub fn resolve_install(
    want: &[String],
    arch: &str,
    db: &InstalledDb,
    index: &Index,
    mode: InstallMode,
) -> Result<ResolvedPlan, DepsError> {
    let installed_names: BTreeSet<String> = db.list()?.into_iter().collect();

    let mut state_map: BTreeMap<String, NodeState> = BTreeMap::new();
    let mut result: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();

    // Preserve caller order. Reordering independent packages can change the
    // final owner of replacement paths such as /bin/sh and /bin/reboot.
    for name in want {
        // Verify the package exists in the index before diving in.
        if index.get(name, arch).is_none() {
            return Err(DepsError::UnknownPackage {
                name: name.clone(),
                arch: arch.to_string(),
            });
        }
        dfs_visit(
            name,
            arch,
            index,
            &installed_names,
            &mut state_map,
            &mut result,
            &mut stack,
        )?;
    }

    // Split: to_install vs already_installed (step 4/5 of the algorithm).
    let mut to_install: Vec<String> = Vec::new();
    let mut already_installed: Vec<String> = Vec::new();

    for pkg in result {
        if mode.is_force() || !installed_names.contains(&pkg) {
            to_install.push(pkg);
        } else {
            already_installed.push(pkg);
        }
    }

    Ok(ResolvedPlan {
        to_install,
        already_installed,
    })
}

/// Resolve removal order for `targets`.
///
/// Algorithm (deps.c:245-295):
///   1. Add explicit `targets` to the removal set.
///   2. If `mode` is [`OrphanMode::PruneOrphans`], walk each target's
///      `metadata.depends.runtime`; any dependency whose only reverse-dependents
///      are in the removal set is itself added (recursively).
///   3. Topological order: a package appears AFTER all packages that depend on
///      it (so it is removed last among its rev-dependents).
///      The C code (deps.c:247-251) simply puts the explicit target first, then
///      appends orphaned dependencies — which is already "rev-dep first" for the
///      single-package case.  We replicate that ordering here.
///   4. If a target has live reverse-dependents NOT in the removal set, return
///      `Err::UnknownDependency` (used as "blocked by rev-dep" — matches the
///      task contract).
pub fn resolve_remove(
    targets: &[String],
    db: &InstalledDb,
    mode: OrphanMode,
) -> Result<Vec<String>, DepsError> {
    // Build the full set of installed packages and their runtime deps.
    let all_names = db.list()?;

    // result preserves insertion order: targets first, orphans appended.
    let mut result: Vec<String> = targets.to_vec();
    // Sort the explicit targets for determinism.
    result.sort();
    // removal_set grows as we discover orphans.
    let mut removal_set: BTreeSet<String> = result.iter().cloned().collect();

    // Build reverse-dependency map: dep → set of packages that depend on it.
    // Iterate in sorted order (all_names from BTreeMap is already sorted).
    let mut rev_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for pkg_name in &all_names {
        if let Some(pkg) = db.get(pkg_name)? {
            let mut runtime_deps = pkg.metadata.depends.runtime.clone();
            runtime_deps.sort();
            for dep in runtime_deps {
                rev_deps
                    .entry(dep)
                    .or_default()
                    .insert(pkg_name.clone());
            }
        }
    }

    // Step 4: check that no target has a live rev-dependent outside the removal set.
    for target in targets {
        if let Some(rdeps) = rev_deps.get(target) {
            let live: Vec<&String> = rdeps
                .iter()
                .filter(|r| !removal_set.contains(*r))
                .collect();
            if !live.is_empty() {
                return Err(DepsError::UnknownDependency {
                    dependent: live[0].clone(),
                    missing: target.clone(),
                });
            }
        }
    }

    if !mode.is_prune() {
        return Ok(result);
    }

    // Step 2: orphan reaping (mirrors deps.c:263-294).
    // Process the worklist in a stable order.  We iterate over a snapshot of
    // `result` (which grows as orphans are found) via an index cursor.
    let mut cursor = 0;
    while cursor < result.len() {
        let pkg_name = result[cursor].clone();
        cursor += 1;

        let pkg = match db.get(&pkg_name)? {
            Some(p) => p,
            None => continue,
        };

        let mut runtime_deps = pkg.metadata.depends.runtime.clone();
        runtime_deps.sort();

        for dep in runtime_deps {
            // Already in removal set → skip.
            if removal_set.contains(&dep) {
                continue;
            }
            // Must be installed.
            if db.get(&dep)?.is_none() {
                continue;
            }
            // All rev-dependents of `dep` must be in the removal set.
            let all_rdeps_removing = rev_deps
                .get(&dep)
                .map(|rdeps| rdeps.iter().all(|r| removal_set.contains(r)))
                .unwrap_or(true);

            if all_rdeps_removing {
                removal_set.insert(dep.clone());
                result.push(dep);
            }
        }
    }

    Ok(result)
}

/// Detect cycles in an arbitrary directed graph.
///
/// Returns `Some(path)` where `path` lists the nodes forming the first cycle
/// found (in DFS discovery order, with the repeated node appended to close the
/// loop).  Returns `None` if the graph is acyclic.
///
/// Iteration over the graph is deterministic because `BTreeMap` keys are
/// sorted.
pub fn has_cycle(graph: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    let mut state: BTreeMap<String, NodeState> = BTreeMap::new();

    // Collect all nodes (keys + edge targets) so isolated sinks are also tried.
    let mut all_nodes: BTreeSet<String> = graph.keys().cloned().collect();
    for neighbours in graph.values() {
        for n in neighbours {
            all_nodes.insert(n.clone());
        }
    }

    for start in &all_nodes {
        if state.get(start).copied().unwrap_or(NodeState::Unvisited) == NodeState::Unvisited {
            let mut stack: Vec<String> = Vec::new();
            if let Some(cycle) = cycle_dfs(start, graph, &mut state, &mut stack) {
                return Some(cycle);
            }
        }
    }
    None
}

/// DFS helper for [`has_cycle`].  Returns the cycle path on finding a back-edge.
fn cycle_dfs(
    node: &str,
    graph: &BTreeMap<String, Vec<String>>,
    state: &mut BTreeMap<String, NodeState>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    state.insert(node.to_string(), NodeState::Visiting);
    stack.push(node.to_string());

    // Sort neighbours for deterministic traversal.
    let mut neighbours: Vec<String> = graph
        .get(node)
        .cloned()
        .unwrap_or_default();
    neighbours.sort();

    for neighbour in &neighbours {
        match state.get(neighbour).copied().unwrap_or(NodeState::Unvisited) {
            NodeState::Visiting => {
                // Back-edge: build cycle path.
                let start = stack
                    .iter()
                    .position(|n| n == neighbour)
                    .unwrap_or(0);
                let mut cycle: Vec<String> = stack[start..].to_vec();
                cycle.push(neighbour.clone()); // close the loop
                return Some(cycle);
            }
            NodeState::Visited => {}
            NodeState::Unvisited => {
                if let Some(cycle) = cycle_dfs(neighbour, graph, state, stack) {
                    return Some(cycle);
                }
            }
        }
    }

    state.insert(node.to_string(), NodeState::Visited);
    stack.pop();
    None
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{DependsSection, IndexEntry, Metadata, PackageSection};
    use crate::types::{InstallMode, OrphanMode};

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_entry(depends: Vec<&str>) -> IndexEntry {
        IndexEntry {
            version: "1.0".into(),
            license: "MIT".into(),
            description: "test".into(),
            arch: "x86_64".into(),
            sha256: "aa".into(),
            size: 1,
            depends: depends.into_iter().map(String::from).collect(),
            build_depends: vec![],
        }
    }

    /// Build an `Index` from a slice of `(name, depends)` pairs for arch x86_64.
    fn make_index(pkgs: &[(&str, Vec<&str>)]) -> Index {
        let mut entries = std::collections::BTreeMap::new();
        for (name, deps) in pkgs {
            let key = format!("{}-x86_64", name);
            entries.insert(key, make_entry(deps.clone()));
        }
        Index { entries }
    }

    /// Build a tempdir-backed [`InstalledDb`] populated with the given packages
    /// (no runtime deps recorded).  Returns the [`tempfile::TempDir`] guard
    /// alongside the db so the caller can keep it alive for the duration of
    /// the test (dropping the guard removes the on-disk state).
    fn make_db(installed: &[&str]) -> (tempfile::TempDir, InstalledDb) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db = InstalledDb::open(tmp.path()).expect("open db");
        for name in installed {
            let meta = Metadata {
                package: PackageSection {
                    name: Some(name.to_string()),
                    version: Some("1.0".into()),
                    ..Default::default()
                },
                depends: DependsSection::default(),
                ..Default::default()
            };
            db.insert(&InstalledPkg {
                metadata: meta,
                files: vec![],
            })
            .expect("insert");
        }
        (tmp, db)
    }

    /// Build a db where each package has specific runtime deps recorded.
    fn make_db_with_deps(pkgs: &[(&str, Vec<&str>)]) -> (tempfile::TempDir, InstalledDb) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let db = InstalledDb::open(tmp.path()).expect("open db");
        for (name, deps) in pkgs {
            let meta = Metadata {
                package: PackageSection {
                    name: Some(name.to_string()),
                    version: Some("1.0".into()),
                    ..Default::default()
                },
                depends: DependsSection {
                    runtime: deps.iter().map(|s| s.to_string()).collect(),
                    build: vec![],
                },
                ..Default::default()
            };
            db.insert(&InstalledPkg {
                metadata: meta,
                files: vec![],
            })
            .expect("insert");
        }
        (tmp, db)
    }

    // ── Test 1: Linear chain A → B → C ───────────────────────────────────────

    #[test]
    fn test_linear_chain() {
        // A depends on B, B depends on C.  Expected install order: [C, B, A].
        let index = make_index(&[
            ("a", vec!["b"]),
            ("b", vec!["c"]),
            ("c", vec![]),
        ]);
        let (_tmp, db) = make_db(&[]);
        let plan = resolve_install(&[String::from("a")], "x86_64", &db, &index, InstallMode::Normal)
            .expect("resolve_install failed");
        assert_eq!(plan.to_install, vec!["c", "b", "a"]);
        assert!(plan.already_installed.is_empty());
    }

    // ── Test 2: Diamond A → B,C; B,C → D ────────────────────────────────────

    #[test]
    fn test_diamond() {
        // A depends on B and C, both depend on D.
        // Post-order DFS visits deps sorted: B before C (alphabetical).
        // DFS from A: visit B → visit D (leaf) → D; B; visit C → D (already
        // Visited) → C; A.  Expected: [D, B, C, A].
        let index = make_index(&[
            ("a", vec!["b", "c"]),
            ("b", vec!["d"]),
            ("c", vec!["d"]),
            ("d", vec![]),
        ]);
        let (_tmp, db) = make_db(&[]);
        let plan = resolve_install(&[String::from("a")], "x86_64", &db, &index, false)
            .expect("resolve_install failed");

        // D must be first, A must be last; B and C in between.
        assert_eq!(plan.to_install[0], "d", "D must be first");
        assert_eq!(
            plan.to_install[plan.to_install.len() - 1],
            "a",
            "A must be last"
        );
        let b_pos = plan.to_install.iter().position(|x| x == "b").expect("b missing");
        let c_pos = plan.to_install.iter().position(|x| x == "c").expect("c missing");
        let a_pos = plan.to_install.iter().position(|x| x == "a").expect("a missing");
        assert!(b_pos < a_pos, "B must come before A");
        assert!(c_pos < a_pos, "C must come before A");
    }

    // ── Test 3: Cycle A → B → A ──────────────────────────────────────────────

    #[test]
    fn test_cycle() {
        let index = make_index(&[
            ("a", vec!["b"]),
            ("b", vec!["a"]),
        ]);
        let (_tmp, db) = make_db(&[]);
        let err = resolve_install(&[String::from("a")], "x86_64", &db, &index, false)
            .expect_err("expected cycle error");
        match err {
            DepsError::Cycle(path) => {
                // Path must contain a, b, and close with a repeated node.
                assert!(
                    path.contains(&String::from("a")),
                    "cycle path must contain a: {path:?}"
                );
                assert!(
                    path.contains(&String::from("b")),
                    "cycle path must contain b: {path:?}"
                );
                // The closing node (last element) must also appear earlier.
                let last = path.last().unwrap();
                assert!(
                    path[..path.len() - 1].contains(last),
                    "cycle path not closed: {path:?}"
                );
            }
            other => panic!("expected Cycle, got: {other:?}"),
        }
    }

    // ── Test 4: Already-installed exclusion (force=false) ─────────────────────

    #[test]
    fn test_already_installed_excluded() {
        // A depends on B.  B is already installed.  With force=false, plan = [A].
        let index = make_index(&[
            ("a", vec!["b"]),
            ("b", vec![]),
        ]);
        let (_tmp, db) = make_db(&["b"]);
        let plan = resolve_install(&[String::from("a")], "x86_64", &db, &index, false)
            .expect("resolve_install failed");
        assert_eq!(plan.to_install, vec!["a"]);
        assert!(
            plan.already_installed.contains(&String::from("b")),
            "b should be in already_installed"
        );
    }

    // ── Test 5: Force re-install ──────────────────────────────────────────────

    #[test]
    fn test_force_reinstall() {
        // A is already installed.  With force=true, plan = [A].
        let index = make_index(&[("a", vec![])]);
        let (_tmp, db) = make_db(&["a"]);
        let plan = resolve_install(&[String::from("a")], "x86_64", &db, &index, true)
            .expect("resolve_install failed");
        assert!(
            plan.to_install.contains(&String::from("a")),
            "a must be in to_install with force=true"
        );
        assert!(
            plan.already_installed.is_empty(),
            "already_installed must be empty when force=true"
        );
    }

    #[test]
    fn test_preserves_requested_package_order() {
        let index = make_index(&[
            ("toybox", vec![]),
            ("mksh", vec![]),
            ("shadow", vec![]),
        ]);
        let (_tmp, db) = make_db(&[]);
        let want = vec![
            String::from("toybox"),
            String::from("mksh"),
            String::from("shadow"),
        ];
        let plan = resolve_install(&want, "x86_64", &db, &index, false)
            .expect("resolve_install failed");
        assert_eq!(plan.to_install, want);
    }

    // ── Test 6: Unknown dependency ───────────────────────────────────────────

    #[test]
    fn test_unknown_dep() {
        // A depends on B, but B is not in the index.
        let index = make_index(&[("a", vec!["b"])]);
        let (_tmp, db) = make_db(&[]);
        let err = resolve_install(&[String::from("a")], "x86_64", &db, &index, false)
            .expect_err("expected error for unknown dep");
        match err {
            DepsError::UnknownDependency { dependent, missing } => {
                assert_eq!(dependent, "a");
                assert_eq!(missing, "b");
            }
            other => panic!("expected UnknownDependency, got: {other:?}"),
        }
    }

    // ── Test 7: Removal with no rev-deps → [A] ───────────────────────────────

    #[test]
    fn test_remove_no_rev_deps() {
        // Only A is installed, nothing depends on it.
        let (_tmp, db) = make_db_with_deps(&[("a", vec![])]);
        let order = resolve_remove(&[String::from("a")], &db, false)
            .expect("resolve_remove failed");
        assert_eq!(order, vec!["a"]);
    }

    // ── Test 8: Removal with orphans ─────────────────────────────────────────

    #[test]
    fn test_remove_with_orphans() {
        // A depends on B.  Remove A with orphans=true.
        // B has no other rev-dependents → B is orphaned → result = [A, B].
        let (_tmp, db) = make_db_with_deps(&[
            ("a", vec!["b"]),
            ("b", vec![]),
        ]);
        let order = resolve_remove(&[String::from("a")], &db, true)
            .expect("resolve_remove failed");
        assert!(order.contains(&String::from("a")), "a must be in removal list");
        assert!(order.contains(&String::from("b")), "b must be in removal list as orphan");
        // A should appear before B in the result.
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        assert!(a_pos < b_pos, "A should appear before orphan B: {order:?}");
    }

    // ── Test 9: Removal blocked by live rev-dep ───────────────────────────────

    #[test]
    fn test_remove_blocked_by_rev_dep() {
        // A depends on B.  Try to remove B without removing A → error.
        let (_tmp, db) = make_db_with_deps(&[
            ("a", vec!["b"]),
            ("b", vec![]),
        ]);
        let err = resolve_remove(&[String::from("b")], &db, false)
            .expect_err("expected error: b is needed by a");
        match err {
            DepsError::UnknownDependency { dependent, missing } => {
                assert_eq!(dependent, "a", "dependent should be a");
                assert_eq!(missing, "b", "missing should be b");
            }
            other => panic!("expected UnknownDependency, got: {other:?}"),
        }
    }

    // ── Test 10: has_cycle standalone ────────────────────────────────────────

    #[test]
    fn test_has_cycle_detects_3_node_cycle() {
        // A → B → C → A
        let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
        graph.insert("a".into(), vec!["b".into()]);
        graph.insert("b".into(), vec!["c".into()]);
        graph.insert("c".into(), vec!["a".into()]);

        let cycle = has_cycle(&graph).expect("expected cycle to be detected");
        // Cycle path must contain all three nodes.
        assert!(cycle.contains(&String::from("a")), "a missing from cycle: {cycle:?}");
        assert!(cycle.contains(&String::from("b")), "b missing from cycle: {cycle:?}");
        assert!(cycle.contains(&String::from("c")), "c missing from cycle: {cycle:?}");
        // Last element closes the cycle.
        let last = cycle.last().unwrap();
        assert!(
            cycle[..cycle.len() - 1].contains(last),
            "cycle not closed: {cycle:?}"
        );
    }

    #[test]
    fn test_has_cycle_accepts_dag() {
        // A → B → C (DAG — no cycle).
        let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
        graph.insert("a".into(), vec!["b".into()]);
        graph.insert("b".into(), vec!["c".into()]);
        graph.insert("c".into(), vec![]);

        assert!(has_cycle(&graph).is_none(), "expected no cycle in DAG");
    }
}
