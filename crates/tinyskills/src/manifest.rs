//! The sibling manifest (`skill.toml`) a scaffolded bundle may carry: declared
//! `[[inputs]]` and a `when_to_use` trigger.
//!
//! A host passes the inputs a user declared at create time through
//! [`validate_manifest_inputs`], then writes [`render_manifest_toml`] next to the
//! scaffolded document. The manifest file name is the host's choice.

use serde::{Deserialize, Serialize};

/// One declared `[[inputs]]` entry, as supplied at create time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestInput {
    /// Input key; trimmed and unique (case-insensitively) once validated.
    pub name: String,
    /// Optional help text; omitted from the manifest when empty.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the input must be supplied; defaults to `true`.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Type hint (`string`, `integer`, `boolean`), stored verbatim.
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
}

fn default_required() -> bool {
    true
}

/// Why a set of declared inputs was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestInputError {
    /// An input name was empty or whitespace-only.
    #[error("input name must not be empty")]
    EmptyName,
    /// Two inputs share a name, ignoring ASCII case.
    #[error("duplicate input name '{0}'")]
    Duplicate(String),
}

/// Trim every input name in place and refuse blank or duplicate names.
///
/// # Errors
///
/// [`ManifestInputError::EmptyName`] for a blank name and
/// [`ManifestInputError::Duplicate`] when two names collide ignoring ASCII case.
pub fn validate_manifest_inputs(inputs: &mut [ManifestInput]) -> Result<(), ManifestInputError> {
    let mut seen = std::collections::HashSet::new();
    for input in inputs.iter_mut() {
        let trimmed = input.name.trim();
        if trimmed.is_empty() {
            return Err(ManifestInputError::EmptyName);
        }
        if !seen.insert(trimmed.to_ascii_lowercase()) {
            return Err(ManifestInputError::Duplicate(trimmed.to_owned()));
        }
        input.name = trimmed.to_owned();
    }
    Ok(())
}

/// Render the manifest: `id`, `when_to_use`, then one `[[inputs]]` per input.
#[must_use]
pub fn render_manifest_toml(slug: &str, when_to_use: &str, inputs: &[ManifestInput]) -> String {
    let mut out = String::new();
    out.push_str(&format!("id = {}\n", toml_string_literal(slug)));
    out.push_str(&format!(
        "when_to_use = {}\n",
        toml_string_literal(when_to_use)
    ));
    for input in inputs {
        out.push_str("\n[[inputs]]\n");
        out.push_str(&format!("name = {}\n", toml_string_literal(&input.name)));
        if let Some(d) = input.description.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("description = {}\n", toml_string_literal(d)));
        }
        out.push_str(&format!("required = {}\n", input.required));
        if let Some(t) = input.type_.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("type = {}\n", toml_string_literal(t)));
        }
    }
    out
}

/// A single-line TOML basic string with `\`, `"` and control characters escaped.
fn toml_string_literal(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c => escaped.push(c),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
