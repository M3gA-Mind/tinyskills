use super::*;

fn resource(path: &str, bytes: &[u8]) -> BundleFile {
    BundleFile {
        path: path.into(),
        bytes: bytes.into(),
    }
}

fn bundle(files: Vec<BundleFile>) -> Result<UserBundle, BundleContentError> {
    UserBundle::new(
        BundleDocument::Skill,
        "original\r\n".into(),
        files,
        BundleLimits::default(),
    )
}

#[test]
fn unsafe_resource_names_are_rejected_without_normalizing_source_bytes() {
    for path in [
        "",
        "/absolute",
        "../escape",
        "scripts/../escape",
        "scripts//tool",
        "./tool",
        "scripts/.",
        "scripts/",
        "scripts\\tool",
        "C:/drive",
        "bad\0name",
        "aux",
        "COM1.txt",
        "refs/trailing.",
        "refs/trailing ",
        "refs/a:b",
    ] {
        assert_eq!(
            bundle(vec![resource(path, b"private")]).unwrap_err(),
            BundleContentError::InvalidPath,
            "{path:?}"
        );
    }
}

#[test]
fn duplicate_primary_case_aliases_and_file_directory_conflicts_are_rejected() {
    for files in [
        vec![resource("SKILL.md", b"duplicate")],
        vec![resource("skill.MD", b"alias")],
        vec![resource("assets/a", b"a"), resource("assets/A", b"b")],
    ] {
        assert_eq!(
            bundle(files).unwrap_err(),
            BundleContentError::DuplicatePath
        );
    }
    for files in [
        vec![resource("refs/a", b"a"), resource("refs/a/deep", b"b")],
        vec![resource("refs/A/deep", b"b"), resource("refs/a", b"a")],
    ] {
        assert_eq!(bundle(files).unwrap_err(), BundleContentError::PathConflict);
    }
}

#[test]
fn publication_budgets_include_primary_file_and_utf8_bytes() {
    let limits = BundleLimits {
        max_files: 1,
        ..BundleLimits::default()
    };
    assert_eq!(
        UserBundle::new(
            BundleDocument::Skill,
            String::new(),
            vec![resource("a", b"")],
            limits
        )
        .unwrap_err(),
        BundleContentError::Limit("file count")
    );
    let limits = BundleLimits {
        max_file_bytes: 1,
        ..BundleLimits::default()
    };
    assert_eq!(
        UserBundle::new(BundleDocument::Skill, "é".into(), Vec::new(), limits).unwrap_err(),
        BundleContentError::Limit("file bytes")
    );
    let limits = BundleLimits {
        max_total_bytes: 1,
        ..BundleLimits::default()
    };
    assert_eq!(
        UserBundle::new(
            BundleDocument::Skill,
            "a".into(),
            vec![resource("a", b"b")],
            limits
        )
        .unwrap_err(),
        BundleContentError::Limit("total bytes")
    );
    let limits = BundleLimits {
        max_path_bytes: 7,
        ..BundleLimits::default()
    };
    assert_eq!(
        UserBundle::new(BundleDocument::Skill, String::new(), Vec::new(), limits).unwrap_err(),
        BundleContentError::Limit("path bytes")
    );
}

#[test]
fn manifest_and_digest_are_order_independent_and_cover_bytes_names_and_primary() {
    let a = bundle(vec![
        resource("refs/z", b"z"),
        resource("assets/a", &[0, 255]),
    ])
    .unwrap();
    let b = bundle(vec![
        resource("assets/a", &[0, 255]),
        resource("refs/z", b"z"),
    ])
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.digest().len(), 64);
    let manifest = a.manifest();
    assert_eq!(
        manifest
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["SKILL.md", "assets/a", "refs/z"]
    );
    assert_eq!(manifest[1].bytes, 2);
    assert_eq!(manifest[1].digest.len(), 64);
    let changed = bundle(vec![
        resource("refs/z", b"different"),
        resource("assets/a", &[0, 255]),
    ])
    .unwrap();
    assert_ne!(a.digest(), changed.digest());
    let renamed = bundle(vec![
        resource("refs/other", b"z"),
        resource("assets/a", &[0, 255]),
    ])
    .unwrap();
    assert_ne!(a.digest(), renamed.digest());
    let workflow = UserBundle::new(
        BundleDocument::Workflow,
        "original\r\n".into(),
        vec![resource("refs/z", b"z"), resource("assets/a", &[0, 255])],
        BundleLimits::default(),
    )
    .unwrap();
    assert_ne!(a.digest(), workflow.digest());
}

#[test]
fn resources_preserve_binary_empty_and_original_document_and_allow_both_document_names() {
    let stored = bundle(vec![
        resource("refs/empty", b""),
        resource("assets/binary", &[0, 255]),
        resource("WORKFLOW.md", b"workflow"),
    ])
    .unwrap();
    assert_eq!(stored.file("SKILL.md"), Some(b"original\r\n".as_slice()));
    assert_eq!(stored.file("refs/empty"), Some(b"".as_slice()));
    assert_eq!(stored.file("refs/missing"), None);
    assert_eq!(stored.file("assets/binary"), Some([0, 255].as_slice()));
    assert_eq!(stored.file("WORKFLOW.md"), Some(b"workflow".as_slice()));
}

#[test]
fn primary_selection_changes_digest_even_when_all_stored_files_match() {
    let skill = UserBundle::new(
        BundleDocument::Skill,
        "skill".into(),
        vec![resource("WORKFLOW.md", b"workflow")],
        BundleLimits::default(),
    )
    .unwrap();
    let workflow = UserBundle::new(
        BundleDocument::Workflow,
        "workflow".into(),
        vec![resource("SKILL.md", b"skill")],
        BundleLimits::default(),
    )
    .unwrap();
    assert_eq!(skill.files(), workflow.files());
    assert_ne!(skill.digest(), workflow.digest());
}

#[test]
fn length_delimiters_and_file_hashes_preserve_unambiguous_original_bytes() {
    let first = bundle(vec![resource("ab", b"c")]).unwrap();
    let second = bundle(vec![resource("a", b"bc")]).unwrap();
    assert_ne!(first.digest(), second.digest());
    let empty = UserBundle::new(
        BundleDocument::Skill,
        String::new(),
        Vec::new(),
        BundleLimits {
            max_files: 1,
            max_file_bytes: 0,
            max_total_bytes: 0,
            max_path_bytes: 8,
        },
    )
    .unwrap();
    assert_eq!(empty.file("SKILL.md"), Some(b"".as_slice()));
    assert_eq!(
        empty.manifest()[0].digest,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
