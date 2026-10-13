use super::*;

use cap_std::ambient_authority;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn open(path: &Path) -> std::io::Result<Dir> {
    Dir::open_ambient_dir(path, ambient_authority())
}

fn doc(dir_name: &str, document: &str) -> MaterializeEntry {
    MaterializeEntry {
        dir_name: dir_name.to_string(),
        source: MaterializeSource::Document(document.to_string()),
    }
}

fn bundle(dir_name: &str, src: &Path) -> Result<MaterializeEntry, std::io::Error> {
    Ok(MaterializeEntry {
        dir_name: dir_name.to_string(),
        source: MaterializeSource::Dir(Arc::new(open(src)?)),
    })
}

fn refused(
    parent: &Path,
    root_name: &str,
    entries: &[MaterializeEntry],
) -> Result<MaterializeError, Box<dyn std::error::Error>> {
    match materialize_tree(&open(parent)?, root_name, entries) {
        Ok(report) => Err(format!("expected a refusal, wrote {report:?}").into()),
        Err(error) => Ok(error),
    }
}

fn deep_source(temp: &Path, levels: usize) -> Result<PathBuf, std::io::Error> {
    let src = temp.join("src");
    let mut deep = src.clone();
    for level in 0..levels {
        deep = deep.join(format!("d{level}"));
    }
    std::fs::create_dir_all(&deep)?;
    Ok(src)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[test]
fn a_source_nested_past_the_limit_is_refused_and_the_old_tree_survives() -> TestResult {
    let temp = tempfile::tempdir()?;
    let src = deep_source(temp.path(), MAX_MATERIALIZE_DEPTH + 1)?;
    materialize_tree(&open(temp.path())?, "out", &[doc("keep", "old")])?;

    let error = refused(temp.path(), "out", &[bundle("deep", &src)?])?;

    let mut over = PathBuf::from("deep");
    for level in 0..=MAX_MATERIALIZE_DEPTH {
        over = over.join(format!("d{level}"));
    }
    assert!(
        matches!(&error, MaterializeError::TooDeep { path, max } if *path == over && *max == MAX_MATERIALIZE_DEPTH),
        "{error:?}"
    );
    assert!(error.to_string().contains("directories deep"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("out/keep/SKILL.md"))?,
        "old"
    );
    assert!(!temp.path().join(".out.materializing").exists());
    Ok(())
}

#[test]
fn a_source_at_the_limit_is_copied() -> TestResult {
    let temp = tempfile::tempdir()?;
    let src = deep_source(temp.path(), MAX_MATERIALIZE_DEPTH)?;
    let mut leaf = src.clone();
    for level in 0..MAX_MATERIALIZE_DEPTH {
        leaf = leaf.join(format!("d{level}"));
    }
    std::fs::write(leaf.join("leaf.md"), "leaf")?;

    let report = materialize_tree(&open(temp.path())?, "out", &[bundle("deep", &src)?])?;
    assert_eq!(report.files, 1);
    Ok(())
}

#[test]
fn an_unsafe_root_name_is_refused() -> TestResult {
    let temp = tempfile::tempdir()?;
    for bad in ["", ".", "..", "a/b", "a\\b", "has space"] {
        let error = refused(temp.path(), bad, &[])?;
        assert!(
            matches!(&error, MaterializeError::UnsafeRootName { root_name } if root_name == bad),
            "{bad:?}: {error:?}"
        );
        assert!(error.to_string().contains("not a safe skill tree name"));
    }
    Ok(())
}

#[test]
fn a_regular_file_at_the_root_is_replaced_by_the_tree() -> TestResult {
    let temp = tempfile::tempdir()?;
    std::fs::write(temp.path().join("skills"), "not a dir")?;

    materialize_tree(&open(temp.path())?, "skills", &[doc("a", "x")])?;
    assert_eq!(
        std::fs::read_to_string(temp.path().join("skills/a/SKILL.md"))?,
        "x"
    );
    Ok(())
}

#[test]
fn a_symlinked_root_is_unlinked_and_its_target_left_alone() -> TestResult {
    let temp = tempfile::tempdir()?;
    let target = temp.path().join("target");
    std::fs::create_dir_all(&target)?;
    std::fs::write(target.join("keep.md"), "keep")?;
    symlink(&target, &temp.path().join("skills"))?;

    materialize_tree(&open(temp.path())?, "skills", &[doc("a", "x")])?;

    assert!(
        !std::fs::symlink_metadata(temp.path().join("skills"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(target.join("keep.md"))?, "keep");
    assert!(!target.join("a").exists());
    Ok(())
}

#[test]
fn a_stale_staging_entry_is_removed_not_followed() -> TestResult {
    let temp = tempfile::tempdir()?;
    let target = temp.path().join("target");
    std::fs::create_dir_all(&target)?;
    std::fs::write(target.join("keep.md"), "keep")?;
    symlink(&target, &temp.path().join(".skills.materializing"))?;

    materialize_tree(&open(temp.path())?, "skills", &[doc("a", "x")])?;

    assert_eq!(std::fs::read_to_string(target.join("keep.md"))?, "keep");
    assert!(!target.join("a").exists());
    assert!(!temp.path().join(".skills.materializing").exists());
    assert!(temp.path().join("skills/a/SKILL.md").is_file());
    Ok(())
}

#[test]
fn a_source_inside_the_old_tree_is_read_before_the_old_tree_is_removed() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("skills");
    std::fs::create_dir_all(root.join("inner"))?;
    std::fs::write(root.join("inner/SKILL.md"), "keep")?;
    let entry = bundle("copy", &root.join("inner"))?;

    let report = materialize_tree(&open(temp.path())?, "skills", &[entry])?;

    assert_eq!(report.files, 1);
    assert_eq!(std::fs::read_to_string(root.join("copy/SKILL.md"))?, "keep");
    assert!(!root.join("inner").exists());
    Ok(())
}

#[test]
fn an_entry_swapped_for_a_symlink_is_not_opened() -> TestResult {
    let temp = tempfile::tempdir()?;
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside)?;
    std::fs::write(outside.join("secret.txt"), "secret")?;
    let src = temp.path().join("src");
    let out = temp.path().join("out");
    std::fs::create_dir_all(&src)?;
    std::fs::create_dir_all(&out)?;
    symlink(&outside.join("secret.txt"), &src.join("file"))?;
    symlink(&outside, &src.join("dir"))?;
    let (source, dest) = (open(&src)?, open(&out)?);

    assert!(copy_file(&source, &dest, &OsString::from("file"), Path::new("file")).is_err());
    assert!(source.open_dir_nofollow("dir").is_err());
    assert!(!out.join("file").exists());
    Ok(())
}

#[test]
fn destination_entries_never_follow_a_symlink() -> TestResult {
    let temp = tempfile::tempdir()?;
    let outside = temp.path().join("outside");
    let root = temp.path().join("root");
    std::fs::create_dir_all(&outside)?;
    std::fs::create_dir_all(&root)?;
    symlink(&outside, &root.join("dir"))?;
    symlink(&outside.join("target.txt"), &root.join("file"))?;
    let tree = open(&root)?;

    assert!(create_dir(&tree, OsStr::new("dir")).is_err());
    assert!(create_file(&tree, OsStr::new("file")).is_err());
    assert!(!outside.join("target.txt").exists());
    Ok(())
}

#[test]
fn errors_name_the_path_relative_to_the_tree() -> TestResult {
    let temp = tempfile::tempdir()?;
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src)?;
    std::fs::File::create(src.join("big.bin"))?.set_len(MAX_MATERIALIZE_FILE_BYTES + 1)?;

    let error = refused(temp.path(), "out", &[bundle("demo", &src)?])?;

    assert!(
        matches!(&error, MaterializeError::FileTooLarge { path, .. } if *path == Path::new("demo").join("big.bin")),
        "{error:?}"
    );
    Ok(())
}

fn files(dir_name: &str, files: Vec<MaterializeFile>) -> MaterializeEntry {
    MaterializeEntry {
        dir_name: dir_name.to_string(),
        source: MaterializeSource::Files(files),
    }
}

#[test]
fn files_source_writes_nested_paths() -> TestResult {
    let temp = tempfile::tempdir()?;
    let parent = open(temp.path())?;
    let entry = files(
        "demo",
        vec![
            MaterializeFile::new("SKILL.md", "doc"),
            MaterializeFile::new("a/b/one.txt", "1"),
            MaterializeFile::new("a/b/two.txt", "2"),
            MaterializeFile::new("a/three.txt", "3"),
        ],
    );
    let report = materialize_tree(&parent, "out", &[entry])?;
    assert_eq!(report.files, 4);
    assert_eq!(report.dirs, ["demo"]);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("out/demo/a/b/two.txt"))?,
        "2"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("out/demo/a/three.txt"))?,
        "3"
    );
    Ok(())
}

#[test]
fn files_source_refuses_unsafe_deep_and_oversized_files() -> TestResult {
    let temp = tempfile::tempdir()?;
    for path in ["", "../x", "/x", "a//b", "a/./b"] {
        let error = refused(
            temp.path(),
            "out",
            &[files("demo", vec![MaterializeFile::new(path, "x")])],
        )?;
        assert!(
            matches!(error, MaterializeError::UnsafeFilePath { .. }),
            "{path:?}: {error:?}"
        );
    }
    let deep = format!("{}/f", vec!["d"; MAX_MATERIALIZE_DEPTH + 1].join("/"));
    let error = refused(
        temp.path(),
        "out",
        &[files("demo", vec![MaterializeFile::new(deep, "x")])],
    )?;
    assert!(
        matches!(error, MaterializeError::TooDeep { .. }),
        "{error:?}"
    );
    let big = vec![0u8; usize::try_from(MAX_MATERIALIZE_FILE_BYTES)? + 1];
    let error = refused(
        temp.path(),
        "out",
        &[files("demo", vec![MaterializeFile::new("big", big)])],
    )?;
    assert!(
        matches!(error, MaterializeError::FileTooLarge { .. }),
        "{error:?}"
    );
    assert!(!temp.path().join("out").exists());
    Ok(())
}

#[test]
fn files_source_refuses_a_file_and_directory_at_one_path() -> TestResult {
    let temp = tempfile::tempdir()?;
    let error = refused(
        temp.path(),
        "out",
        &[files(
            "demo",
            vec![
                MaterializeFile::new("a", "x"),
                MaterializeFile::new("a/b", "y"),
            ],
        )],
    )?;
    assert!(matches!(error, MaterializeError::Io { .. }), "{error:?}");
    Ok(())
}
