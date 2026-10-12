//! Document reads, constants, ids, and frontmatter compatibility.

use std::fs;

use tinyskills::{
    MAX_DESCRIPTION_LEN, MAX_DOCUMENT_BYTES, MAX_NAME_LEN, MAX_RESOURCE_BYTES, RESOURCE_DIRS,
    SKILL_JSON, SKILL_MD, Skill, SkillFrontmatter, WORKFLOW_MD, read_document,
};

#[test]
fn constants_are_public() {
    assert_eq!(SKILL_MD, "SKILL.md");
    assert_eq!(WORKFLOW_MD, "WORKFLOW.md");
    assert_eq!(SKILL_JSON, "skill.json");
    assert_eq!(MAX_NAME_LEN, 64);
    assert_eq!(MAX_DESCRIPTION_LEN, 1024);
    assert_eq!(MAX_RESOURCE_BYTES, 128 * 1024);
    assert_eq!(MAX_DOCUMENT_BYTES, 1024 * 1024);
    assert!(RESOURCE_DIRS.contains(&"scripts"));
}

#[test]
fn skill_id_prefers_dir_name_and_falls_back_to_name() {
    let mut skill = Skill {
        name: "Display Name".to_owned(),
        dir_name: "display-name".to_owned(),
        ..Skill::default()
    };
    assert_eq!(skill.id(), "display-name");
    skill.dir_name.clear();
    assert_eq!(skill.id(), "Display Name");
}

#[test]
fn read_document_reads_bounded_utf8_regular_files() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("WORKFLOW.md");
    fs::write(&path, "---\nname: a\n---\nbody\n")?;
    assert_eq!(read_document(&path)?, "---\nname: a\n---\nbody\n");

    assert!(read_document(&temp.path().join("missing.md")).is_err());
    assert!(
        read_document(temp.path()).is_err(),
        "directories are rejected"
    );

    let bad = temp.path().join("bad.md");
    fs::write(&bad, [0xff, 0xfe])?;
    assert!(read_document(&bad).is_err(), "invalid UTF-8 is rejected");

    let exact = temp.path().join("exact.md");
    fs::write(&exact, "a".repeat(usize::try_from(MAX_DOCUMENT_BYTES)?))?;
    assert!(read_document(&exact).is_ok());
    let big = temp.path().join("big.md");
    fs::write(&big, "a".repeat(usize::try_from(MAX_DOCUMENT_BYTES)? + 1))?;
    assert!(read_document(&big).is_err(), "oversize is rejected");
    Ok(())
}

#[cfg(unix)]
#[test]
fn read_document_rejects_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let target = temp.path().join("secret.md");
    fs::write(&target, "secret")?;
    let link = temp.path().join("WORKFLOW.md");
    std::os::unix::fs::symlink(&target, &link)?;
    assert!(read_document(&link).is_err());
    Ok(())
}

fn parse(yaml: &str) -> Result<SkillFrontmatter, serde_norway::Error> {
    serde_norway::from_str(yaml)
}

#[test]
fn allowed_tools_accepts_sequences_and_comma_strings() -> Result<(), serde_norway::Error> {
    assert_eq!(
        parse("allowed-tools:\n  - Bash\n  - Read\n")?.allowed_tools,
        ["Bash", "Read"]
    );
    assert_eq!(
        parse("allowed-tools: Bash, Read, Grep, Skill, WebFetch")?.allowed_tools,
        ["Bash", "Read", "Grep", "Skill", "WebFetch"]
    );
    Ok(())
}

#[test]
fn allowed_tools_trims_whitespace_and_drops_empty_tokens() -> Result<(), serde_norway::Error> {
    assert_eq!(
        parse(r#"allowed-tools: " Bash, , Read, ""#)?.allowed_tools,
        ["Bash", "Read"]
    );
    Ok(())
}

#[test]
fn allowed_tools_accepts_aliases_and_defaults_empty() -> Result<(), serde_norway::Error> {
    assert_eq!(parse("tools: Bash, Read")?.allowed_tools, ["Bash", "Read"]);
    assert_eq!(
        parse("allowed_tools: Bash, Read")?.allowed_tools,
        ["Bash", "Read"]
    );
    assert_eq!(parse("name: x")?.allowed_tools.len(), 0);
    Ok(())
}
