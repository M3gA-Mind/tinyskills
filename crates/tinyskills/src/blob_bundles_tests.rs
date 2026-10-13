#![allow(clippy::unwrap_used, clippy::expect_used)]

use cap_std::ambient_authority;
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

use super::*;

fn blobs(scope: &str) -> Arc<dyn BlobStore> {
    let storage = MemoryStorage::new();
    Arc::clone(
        storage
            .for_scope(&Scope::new(scope).unwrap())
            .unwrap()
            .blobs(),
    )
}

fn bundles() -> BlobBundles {
    BlobBundles::new(blobs("alice"), "skills").unwrap()
}

fn skill() -> Vec<MaterializeFile> {
    vec![
        MaterializeFile::new("SKILL.md", "---\nname: A\ndescription: B\n---\n"),
        MaterializeFile::new("scripts/run.sh", "echo hi"),
        MaterializeFile::new("references/deep/notes.md", "notes"),
    ]
}

#[tokio::test]
async fn publish_read_list_delete_round_trip() {
    let bundles = bundles();
    assert!(bundles.list().await.unwrap().is_empty());
    assert_eq!(bundles.publish("a", skill()).await.unwrap(), 3);
    bundles
        .publish("b", vec![MaterializeFile::new("SKILL.md", "x")])
        .await
        .unwrap();
    assert_eq!(bundles.list().await.unwrap(), ["a", "b"]);
    let read = bundles.read("a").await.unwrap();
    let paths: Vec<_> = read.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        ["SKILL.md", "references/deep/notes.md", "scripts/run.sh"]
    );
    assert_eq!(read[2].bytes, b"echo hi");
    assert!(bundles.delete("a").await.unwrap());
    assert!(!bundles.delete("a").await.unwrap());
    assert!(matches!(
        bundles.read("a").await,
        Err(BlobBundleError::NotFound(_))
    ));
    assert_eq!(bundles.list().await.unwrap(), ["b"]);
}

#[tokio::test]
async fn republish_drops_files_the_new_bundle_lacks() {
    let bundles = bundles();
    bundles.publish("a", skill()).await.unwrap();
    bundles
        .publish("a", vec![MaterializeFile::new("SKILL.md", "v2")])
        .await
        .unwrap();
    let read = bundles.read("a").await.unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].bytes, b"v2");
}

#[tokio::test]
async fn a_bundle_id_is_not_a_prefix_match() {
    let bundles = bundles();
    bundles.publish("a", skill()).await.unwrap();
    bundles
        .publish("ab", vec![MaterializeFile::new("SKILL.md", "ab")])
        .await
        .unwrap();
    assert_eq!(bundles.read("a").await.unwrap().len(), 3);
    assert!(bundles.delete("a").await.unwrap());
    assert_eq!(bundles.read("ab").await.unwrap().len(), 1);
}

#[tokio::test]
async fn refuses_unsafe_input() {
    assert!(matches!(
        BlobBundles::new(blobs("alice"), "a/b"),
        Err(BlobBundleError::InvalidId(_))
    ));
    let bundles = bundles();
    for id in ["", "..", "a/b"] {
        assert!(matches!(
            bundles.publish(id, skill()).await,
            Err(BlobBundleError::InvalidId(_))
        ));
        assert!(matches!(
            bundles.read(id).await,
            Err(BlobBundleError::InvalidId(_))
        ));
        assert!(matches!(
            bundles.delete(id).await,
            Err(BlobBundleError::InvalidId(_))
        ));
    }
    for path in ["", "../x", "/x", "a//b", "a/./b"] {
        assert!(matches!(
            bundles
                .publish("a", vec![MaterializeFile::new(path, "x")])
                .await,
            Err(BlobBundleError::InvalidPath(_))
        ));
    }
    assert!(matches!(
        bundles.publish("a", Vec::new()).await,
        Err(BlobBundleError::Empty)
    ));
    let big = vec![MaterializeFile::new(
        "big.bin",
        vec![0u8; usize::try_from(MAX_MATERIALIZE_FILE_BYTES).unwrap() + 1],
    )];
    assert!(matches!(
        bundles.publish("a", big).await,
        Err(BlobBundleError::TooLarge { .. })
    ));
    assert!(bundles.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn read_refuses_too_deep_and_oversized_stored_bundles() {
    let store = blobs("alice");
    let bundles = BlobBundles::new(Arc::clone(&store), "skills").unwrap();
    let deep = vec!["d"; MAX_MATERIALIZE_DEPTH + 2].join("/");
    store
        .put(&format!("skills/a/{deep}/f"), b"x".to_vec(), None)
        .await
        .unwrap();
    assert!(matches!(
        bundles.read("a").await,
        Err(BlobBundleError::TooDeep(_))
    ));
    let big = vec![0u8; usize::try_from(MAX_MATERIALIZE_FILE_BYTES).unwrap() + 1];
    store.put("skills/b/big", big, None).await.unwrap();
    assert!(matches!(
        bundles.read("b").await,
        Err(BlobBundleError::TooLarge { .. })
    ));
}

#[tokio::test]
async fn materializes_a_real_directory_tree_that_discovery_can_read() {
    let bundles = bundles();
    bundles.publish("one", skill()).await.unwrap();
    bundles
        .publish(
            "two",
            vec![MaterializeFile::new(
                "SKILL.md",
                "---\nname: T\ndescription: U\n---\n",
            )],
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let parent = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
    let pairs = [
        ("first".to_owned(), "one".to_owned()),
        ("second".to_owned(), "two".to_owned()),
    ];
    let report = bundles.materialize(&parent, "cache", &pairs).await.unwrap();
    assert_eq!(report.dirs, ["first", "second"]);
    assert_eq!(report.files, 4);
    let root = temp.path().join("cache");
    assert_eq!(
        std::fs::read_to_string(root.join("first/scripts/run.sh")).unwrap(),
        "echo hi"
    );
    assert!(root.join("first/references/deep/notes.md").is_file());
    assert!(root.join("second/SKILL.md").is_file());
}

#[tokio::test]
async fn a_missing_bundle_leaves_the_previous_cache_tree_alone() {
    let bundles = bundles();
    bundles.publish("one", skill()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let parent = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
    bundles
        .materialize(&parent, "cache", &[("first".to_owned(), "one".to_owned())])
        .await
        .unwrap();
    let result = bundles
        .materialize(&parent, "cache", &[("first".to_owned(), "gone".to_owned())])
        .await;
    assert!(matches!(result, Err(BlobBundleError::NotFound(_))));
    assert!(temp.path().join("cache/first/SKILL.md").is_file());
}

#[tokio::test]
async fn materialize_reports_directory_name_errors() {
    let bundles = bundles();
    bundles.publish("one", skill()).await.unwrap();
    let temp = tempfile::tempdir().unwrap();
    let parent = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
    let result = bundles
        .materialize(&parent, "cache", &[("../x".to_owned(), "one".to_owned())])
        .await;
    assert!(matches!(
        result,
        Err(BlobBundleError::Materialize(
            MaterializeError::UnsafeDirName { .. }
        ))
    ));
}

#[test]
fn storage_errors_become_backend_errors() {
    let error = BlobBundleError::from(StorageError::new(
        tinystoragedrivers_core::ErrorKind::Backend,
        "down",
    ));
    assert!(matches!(error, BlobBundleError::Backend(_)));
    assert!(format!("{:?}", bundles()).contains("skills"));
}
