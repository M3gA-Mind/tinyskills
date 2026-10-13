#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::fs;

fn write_skill(root: &Path, dir: &str, name: &str) {
    let d = root.join(dir);
    fs::create_dir_all(&d).expect("mkdir");
    fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {name} skill\n---\n\nbody\n"),
    )
    .expect("write");
}

fn query<'a>(
    home: &'a Path,
    ws: &'a Path,
    trusted: bool,
    kinds: &'a [RootKind],
) -> LayoutQuery<'a> {
    LayoutQuery {
        home: Some(home),
        workspace: Some(ws),
        trusted,
        kinds,
        ..Default::default()
    }
}

fn names(skills: &[Skill]) -> Vec<String> {
    let mut n: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    n.sort();
    n
}

#[test]
fn roots_are_ordered_with_workflows_last() {
    let roots = user_roots(Path::new("/h"));
    assert_eq!(roots.len(), 3);
    assert_eq!(roots[2].1, RootKind::Workflow);
    assert!(roots[0].0.ends_with(".openhuman/skills"));
    assert_eq!(project_roots(Path::new("/w")).len(), 3);
}

#[test]
fn untrusted_workspace_contributes_no_project_roots_but_legacy_still_scans() {
    let home = tempfile::tempdir().expect("home");
    let ws = tempfile::tempdir().expect("ws");
    write_skill(&ws.path().join(".openhuman/skills"), "proj", "proj");
    write_skill(&ws.path().join("skills"), "old", "old");
    write_skill(&home.path().join(".openhuman/skills"), "mine", "mine");

    let untrusted = discover_layout(&query(home.path(), ws.path(), false, ALL_ROOT_KINDS));
    assert_eq!(names(&untrusted), ["mine", "old"]);

    let trusted = discover_layout(&query(home.path(), ws.path(), true, ALL_ROOT_KINDS));
    assert_eq!(names(&trusted), ["mine", "old", "proj"]);
}

#[test]
fn workflow_kinds_exclude_skill_roots_and_legacy() {
    let home = tempfile::tempdir().expect("home");
    let ws = tempfile::tempdir().expect("ws");
    write_skill(&home.path().join(".openhuman/skills"), "cap", "cap");
    write_skill(&home.path().join(".openhuman/workflows"), "auto", "auto");
    write_skill(&ws.path().join("skills"), "old", "old");

    let found = discover_layout(&query(home.path(), ws.path(), true, WORKFLOW_ROOT_KINDS));
    assert_eq!(names(&found), ["auto"]);
}

#[test]
fn project_shadows_user_and_builtin_loses_to_everything() {
    let home = tempfile::tempdir().expect("home");
    let ws = tempfile::tempdir().expect("ws");
    write_skill(&home.path().join(".openhuman/skills"), "dup", "dup");
    write_skill(&ws.path().join(".openhuman/skills"), "dup", "dup");
    let builtin = Skill {
        name: "dup".into(),
        dir_name: "dup".into(),
        scope: SkillScope::Builtin,
        ..Default::default()
    };
    let mut q = query(home.path(), ws.path(), true, ALL_ROOT_KINDS);
    q.builtin = vec![builtin];

    let found = discover_layout(&q);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, SkillScope::Project);
}

#[test]
fn extra_user_roots_are_scanned_by_kind() {
    let home = tempfile::tempdir().expect("home");
    let extra = tempfile::tempdir().expect("extra");
    write_skill(extra.path(), "agent-skill", "agent-skill");
    let mut q = LayoutQuery {
        home: Some(home.path()),
        kinds: ALL_ROOT_KINDS,
        ..Default::default()
    };
    q.extra_user_roots = vec![(extra.path().to_path_buf(), RootKind::Skill)];
    assert_eq!(names(&discover_layout(&q)), ["agent-skill"]);
    q.kinds = WORKFLOW_ROOT_KINDS;
    assert!(discover_layout(&q).is_empty());
}

#[test]
fn legacy_scaffold_roots_depend_on_scope() {
    let home = Path::new("/h");
    let ws = Path::new("/w");
    assert_eq!(
        legacy_scaffold_roots(Some(home), ws, SkillScope::User).len(),
        2
    );
    assert_eq!(
        legacy_scaffold_roots(None, ws, SkillScope::User),
        [] as [std::path::PathBuf; 0]
    );
    assert_eq!(
        legacy_scaffold_roots(None, ws, SkillScope::Project).len(),
        2
    );
    assert_eq!(
        legacy_scaffold_roots(Some(home), ws, SkillScope::Builtin),
        [] as [std::path::PathBuf; 0]
    );
}
