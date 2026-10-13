//! The on-disk root layout a host scans: which directories exist per scope, and
//! the multi-root discovery that walks them.
//!
//! Roots come in two kinds. [`RootKind::Skill`] roots (`.openhuman/skills`,
//! `.agents/skills`, the legacy `<workspace>/skills`) hold capability skills;
//! [`RootKind::Workflow`] roots (`.openhuman/workflows`) hold task automations.
//! Both hold the same bundle format.

use std::path::{Path, PathBuf};

use crate::discovery::{CollisionPolicy, TieBreak, resolve_collisions_with, scan_root};
use crate::model::{Skill, SkillScope};

/// Which root category a bundle is discovered under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RootKind {
    /// Capability skills.
    Skill,
    /// Task automations.
    Workflow,
}

/// Every root kind: the full discovery surface.
pub const ALL_ROOT_KINDS: &[RootKind] = &[RootKind::Skill, RootKind::Workflow];
/// Automation roots only.
pub const WORKFLOW_ROOT_KINDS: &[RootKind] = &[RootKind::Workflow];

/// User-scope roots under `home`. `workflows/` is scanned last so a same-named
/// bundle there wins over a pre-rename `skills/` one.
#[must_use]
pub fn user_roots(home: &Path) -> Vec<(PathBuf, RootKind)> {
    roots_under(home)
}

/// Project-scope roots under `workspace`, in the same order as [`user_roots`].
#[must_use]
pub fn project_roots(workspace: &Path) -> Vec<(PathBuf, RootKind)> {
    roots_under(workspace)
}

fn roots_under(base: &Path) -> Vec<(PathBuf, RootKind)> {
    vec![
        (base.join(".openhuman").join("skills"), RootKind::Skill),
        (base.join(".agents").join("skills"), RootKind::Skill),
        (
            base.join(".openhuman").join("workflows"),
            RootKind::Workflow,
        ),
    ]
}

/// The pre-rename `<root>/skills` roots an edit may still find a bundle under:
/// the user's and the project's non-primary skill roots. Other scopes have no
/// writable legacy location.
#[must_use]
pub fn legacy_scaffold_roots(
    home: Option<&Path>,
    workspace: &Path,
    scope: SkillScope,
) -> Vec<PathBuf> {
    let skills_roots = |base: &Path| {
        vec![
            base.join(".openhuman").join("skills"),
            base.join(".agents").join("skills"),
        ]
    };
    match scope {
        SkillScope::User => home.map(skills_roots).unwrap_or_default(),
        SkillScope::Project => skills_roots(workspace),
        _ => Vec::new(),
    }
}

/// What [`discover_layout`] walks.
#[derive(Debug, Clone, Default)]
pub struct LayoutQuery<'a> {
    /// The user's home directory, when known.
    pub home: Option<&'a Path>,
    /// The active workspace, when there is one.
    pub workspace: Option<&'a Path>,
    /// Whether the workspace is trusted; untrusted workspaces contribute no
    /// project-scope roots.
    pub trusted: bool,
    /// Root kinds to scan; empty scans nothing.
    pub kinds: &'a [RootKind],
    /// Builtin bundles the host already loaded, registered first at the lowest
    /// precedence. Only included when `kinds` has [`RootKind::Skill`].
    pub builtin: Vec<Skill>,
    /// Extra user-scope roots (for example a per-agent home), scanned after the
    /// user's own.
    pub extra_user_roots: Vec<(PathBuf, RootKind)>,
    /// Noun for the id in shadowing warnings (`skill` when empty).
    pub id_noun: String,
}

/// Walk the layout and resolve cross-scope collisions.
///
/// Scan order is builtin, user, extra user, project, legacy; among equal
/// precedence the last root to register a name wins. `Profile` bundles are not
/// a filesystem scope and are dropped.
#[must_use]
pub fn discover_layout(query: &LayoutQuery<'_>) -> Vec<Skill> {
    let wants = |kind: RootKind| query.kinds.contains(&kind);
    let mut discovered: Vec<Skill> = Vec::new();

    if wants(RootKind::Skill) {
        discovered.extend(query.builtin.iter().cloned());
    }
    if let Some(home) = query.home {
        for (root, kind) in user_roots(home) {
            if wants(kind) {
                discovered.extend(scan_root(&root, SkillScope::User));
            }
        }
    }
    for (root, kind) in &query.extra_user_roots {
        if wants(*kind) {
            discovered.extend(scan_root(root, SkillScope::User));
        }
    }
    if let Some(workspace) = query.workspace {
        if query.trusted {
            for (root, kind) in project_roots(workspace) {
                if wants(kind) {
                    discovered.extend(scan_root(&root, SkillScope::Project));
                }
            }
        }
        if wants(RootKind::Skill) {
            discovered.extend(scan_root(&workspace.join("skills"), SkillScope::Legacy));
        }
    }

    resolve_collisions_with(
        discovered,
        &CollisionPolicy {
            tie_break: TieBreak::LastWins,
            excluded_scopes: vec![SkillScope::Profile],
            id_noun: if query.id_noun.is_empty() {
                "skill".into()
            } else {
                query.id_noun.clone()
            },
        },
    )
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
