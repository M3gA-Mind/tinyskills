#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn input(name: &str) -> ManifestInput {
    ManifestInput {
        name: name.into(),
        description: None,
        required: true,
        type_: None,
    }
}

#[test]
fn no_inputs_returns_header_only() {
    let out = render_manifest_toml("my-skill", "Does the thing.", &[]);
    assert!(out.contains("id = \"my-skill\""));
    assert!(out.contains("when_to_use = \"Does the thing.\""));
    assert!(!out.contains("[[inputs]]"));
}

#[test]
fn one_input_with_all_fields_roundtrips() {
    let inputs = vec![ManifestInput {
        description: Some("owner/name".into()),
        type_: Some("string".into()),
        ..input("repo")
    }];
    let out = render_manifest_toml("my-skill", "Does the thing.", &inputs);
    let parsed: toml::Value = toml::from_str(&out).expect("emitted manifest must parse");
    let entry = &parsed["inputs"].as_array().expect("array")[0];
    assert_eq!(entry["name"].as_str(), Some("repo"));
    assert_eq!(entry["description"].as_str(), Some("owner/name"));
    assert_eq!(entry["required"].as_bool(), Some(true));
    assert_eq!(entry["type"].as_str(), Some("string"));
}

#[test]
fn optional_fields_omitted_when_empty() {
    let inputs = vec![ManifestInput {
        required: false,
        ..input("n")
    }];
    let out = render_manifest_toml("my-skill", "x", &inputs);
    let parsed: toml::Value = toml::from_str(&out).expect("parse");
    let entry = &parsed["inputs"].as_array().expect("array")[0];
    assert_eq!(entry["required"].as_bool(), Some(false));
    assert!(entry.get("description").is_none());
    assert!(entry.get("type").is_none());
}

#[test]
fn escapes_dangerous_chars_in_strings() {
    let text = "has \"quotes\" and \\ backslash\nand newline\ttab";
    let inputs = vec![ManifestInput {
        description: Some(text.into()),
        ..input("n")
    }];
    let out = render_manifest_toml("my-skill", "x", &inputs);
    let parsed: toml::Value = toml::from_str(&out).expect("escaped strings must parse");
    assert_eq!(
        parsed["inputs"].as_array().expect("array")[0]["description"].as_str(),
        Some(text)
    );
}

#[test]
fn validation_trims_names_and_rejects_blank_or_duplicate() {
    let mut ok = vec![input("  repo "), input("Branch")];
    validate_manifest_inputs(&mut ok).expect("valid");
    assert_eq!(ok[0].name, "repo");

    let mut blank = vec![input("   ")];
    assert_eq!(
        validate_manifest_inputs(&mut blank),
        Err(ManifestInputError::EmptyName)
    );

    let mut dup = vec![input("Repo"), input(" repo")];
    assert_eq!(
        validate_manifest_inputs(&mut dup),
        Err(ManifestInputError::Duplicate("repo".into()))
    );
}

#[test]
fn required_defaults_to_true_when_deserialized() {
    let parsed: ManifestInput =
        serde_json::from_str(r#"{"name":"a","type":"integer"}"#).expect("json");
    assert!(parsed.required);
    assert_eq!(parsed.type_.as_deref(), Some("integer"));
}
