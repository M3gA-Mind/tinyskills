use super::*;

fn skill(dir: &str, name: &str, description: &str, tags: &[&str]) -> Skill {
    Skill {
        dir_name: dir.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

fn corpus() -> Vec<Skill> {
    vec![
        skill(
            "flow-authoring",
            "flow-authoring",
            "The tinyflows authoring reference: expression syntax, node configuration, dry runs.",
            &["flows", "workflows", "reference"],
        ),
        skill(
            "changelog",
            "Release changelog",
            "Turn a git commit range into a written changelog for a release.",
            &["git", "release"],
        ),
        skill(
            "ascii-art",
            "ASCII art",
            "Render text as ASCII art via pyfiglet.",
            &["fun"],
        ),
    ]
}

#[test]
fn a_capability_query_finds_the_skill_that_serves_it() {
    let all = corpus();
    let hits = rank(&all, "write a changelog from commits", 3);
    assert_eq!(hits[0].id(), "changelog");
}

#[test]
fn a_skill_is_findable_by_its_directory_id_even_when_the_display_name_differs() {
    let all = corpus();
    let hits = rank(&all, "ascii-art", 3);
    assert_eq!(hits[0].id(), "ascii-art");
}

#[test]
fn tags_are_searchable() {
    let all = corpus();
    let hits = rank(&all, "pyfiglet fun", 3);
    assert_eq!(hits[0].id(), "ascii-art");
}

#[test]
fn nothing_relevant_returns_nothing() {
    // Also the stopword regression: this once matched `changelog` on "a".
    let all = corpus();
    assert!(rank(&all, "provision a kubernetes cluster", 3).is_empty());
    assert!(rank(&all, "photosynthesis", 3).is_empty());
}

#[test]
fn the_limit_caps_the_result_set() {
    let all = corpus();
    let query = "changelog ascii tinyflows";
    assert_eq!(rank(&all, query, 10).len(), 3);
    assert_eq!(rank(&all, query, 1).len(), 1);
}

#[test]
fn an_empty_corpus_is_safe() {
    assert!(rank(&[], "anything", 5).is_empty());
}

#[test]
fn the_projection_omits_the_body_and_caps_the_description() {
    let long = "x".repeat(1000);
    let hit = project(&skill("s", "S", &long, &["t"]));
    let value = serde_json::to_value(&hit).expect("serialize");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["description", "id", "name", "scope", "tags"]);
    assert!(hit.description.len() <= MAX_DESCRIPTION + 8);
}

#[test]
fn the_projection_id_falls_back_to_the_name_without_a_directory() {
    let hit = project(&skill("", "Only Name", "d", &[]));
    assert_eq!(hit.id, "Only Name");
}
