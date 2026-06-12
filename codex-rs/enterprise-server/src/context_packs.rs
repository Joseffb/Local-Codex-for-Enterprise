use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::path::Path;
use utoipa::ToSchema;

pub const PACK_MANIFEST: &str = "PACK.md";
pub const MAX_CONTEXT_PACK_FILE_BYTES: usize = 10 * 1024 * 1024;

const STANDARD_FILENAMES: &[&str] = &[
    "PACK.md",
    "CALIBRATION.md",
    "OPERATING-INSTRUCTIONS.md",
    "PROJECT-RULES.md",
    "WORKFLOWS.md",
    "HANDOFF.md",
    "VERIFICATION.md",
    "ESCALATION.md",
    "CONTEXT.md",
    "PROMPTS.md",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackDocumentInput {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub load_order: Option<i32>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub file_kind: Option<String>,
    #[serde(default)]
    pub loadable: Option<bool>,
    #[serde(default)]
    pub source_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedContextDocument {
    pub filename: String,
    pub relative_path: String,
    pub content_bytes: Vec<u8>,
    pub content_type: String,
    pub file_size_bytes: i64,
    pub file_kind: String,
    pub loadable: bool,
    pub is_system_file: bool,
    pub source_type: String,
    pub content_hash: String,
    pub load_order: i32,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedContextPack {
    pub documents: Vec<ValidatedContextDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackManifest {
    required_files: Vec<String>,
    load_order: Vec<String>,
}

pub fn validate_documents(documents: &[ContextPackDocumentInput]) -> Result<ValidatedContextPack> {
    if !documents
        .iter()
        .any(|document| document.filename == PACK_MANIFEST)
    {
        anyhow::bail!("context pack must include PACK.md");
    }
    for document in documents {
        validate_filename(&document.filename)?;
    }

    let manifest = documents
        .iter()
        .find(|document| document.filename == PACK_MANIFEST)
        .map(|document| parse_manifest(&document.content))
        .transpose()?
        .context("context pack must include PACK.md")?;

    for required_file in &manifest.required_files {
        if !documents
            .iter()
            .any(|document| document.filename == *required_file)
        {
            anyhow::bail!("context pack is missing required document {required_file}");
        }
    }

    let mut load_order = manifest.load_order;
    for document in documents {
        if document.filename != PACK_MANIFEST && !load_order.contains(&document.filename) {
            load_order.push(document.filename.clone());
        }
    }
    load_order.sort_by_key(|filename| {
        STANDARD_FILENAMES
            .iter()
            .position(|standard| standard == filename)
            .unwrap_or(STANDARD_FILENAMES.len())
    });

    let mut validated = Vec::new();
    for (index, filename) in load_order.into_iter().enumerate() {
        let Some(document) = documents
            .iter()
            .find(|document| document.filename == filename)
        else {
            anyhow::bail!("context pack load order references missing document {filename}");
        };
        let content_bytes = document.content.as_bytes().to_vec();
        let relative_path = document
            .relative_path
            .clone()
            .unwrap_or_else(|| document.filename.clone());
        let relative_path = validate_relative_path(&relative_path)?;
        validate_file_size(content_bytes.len())?;
        let content_type = document
            .content_type
            .clone()
            .unwrap_or_else(|| infer_content_type(&relative_path).to_string());
        let file_kind = normalize_file_kind(document.file_kind.as_deref(), &relative_path)?;
        let source_type = normalize_source_type(document.source_type.as_deref())?;
        let loadable = document
            .loadable
            .unwrap_or_else(|| infer_loadable(&relative_path, &content_type, &file_kind));
        validated.push(ValidatedContextDocument {
            filename: filename.clone(),
            relative_path,
            content_hash: content_hash_bytes(&content_bytes),
            content_bytes,
            content_type,
            file_size_bytes: document.content.len() as i64,
            file_kind,
            loadable,
            is_system_file: is_system_file(&filename),
            source_type,
            load_order: document.load_order.unwrap_or(index as i32),
            required: document
                .required
                .unwrap_or_else(|| manifest.required_files.contains(&filename)),
        });
    }

    Ok(ValidatedContextPack {
        documents: validated,
    })
}

pub fn content_hash(content: &str) -> String {
    content_hash_bytes(content.as_bytes())
}

pub fn content_hash_bytes(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("sha256:{digest:x}")
}

pub fn validate_context_pack_file(
    relative_path: &str,
    content_bytes: Vec<u8>,
    content_type: Option<String>,
    file_kind: Option<String>,
    loadable: Option<bool>,
    source_type: Option<String>,
    load_order: Option<i32>,
    required: Option<bool>,
) -> Result<ValidatedContextDocument> {
    let relative_path = validate_relative_path(relative_path)?;
    validate_file_size(content_bytes.len())?;
    let filename = Path::new(&relative_path)
        .file_name()
        .and_then(|value| value.to_str())
        .context("context pack file path is not allowed")?
        .to_string();
    let content_type =
        content_type.unwrap_or_else(|| infer_content_type(&relative_path).to_string());
    let file_kind = normalize_file_kind(file_kind.as_deref(), &relative_path)?;
    let source_type = normalize_source_type(source_type.as_deref())?;
    let loadable =
        loadable.unwrap_or_else(|| infer_loadable(&relative_path, &content_type, &file_kind));
    Ok(ValidatedContextDocument {
        filename,
        relative_path: relative_path.clone(),
        content_hash: content_hash_bytes(&content_bytes),
        file_size_bytes: content_bytes.len() as i64,
        content_bytes,
        content_type,
        file_kind,
        loadable,
        is_system_file: is_system_file(&relative_path),
        source_type,
        load_order: load_order.unwrap_or(100),
        required: required.unwrap_or(false),
    })
}

pub fn validate_relative_path(path: &str) -> Result<String> {
    if path.is_empty() || path.len() > 512 {
        anyhow::bail!("context pack file path is not allowed");
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        anyhow::bail!("context pack file path is not allowed");
    }
    if path.chars().any(char::is_control) {
        anyhow::bail!("context pack file path is not allowed");
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.starts_with('.')
            || part.chars().any(char::is_control)
        {
            anyhow::bail!("context pack file path is not allowed");
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

pub fn validate_file_size(bytes: usize) -> Result<()> {
    if bytes > MAX_CONTEXT_PACK_FILE_BYTES {
        anyhow::bail!("context pack file exceeds 10 MB limit");
    }
    Ok(())
}

pub fn normalize_file_kind(requested: Option<&str>, relative_path: &str) -> Result<String> {
    let value = requested.unwrap_or_else(|| infer_file_kind(relative_path));
    match value {
        "document" | "bundle" | "asset" => Ok(value.to_string()),
        _ => anyhow::bail!("context pack file kind is not allowed"),
    }
}

pub fn normalize_source_type(requested: Option<&str>) -> Result<String> {
    let value = requested.unwrap_or("upload");
    match value {
        "manual" | "upload" | "import" => Ok(value.to_string()),
        _ => anyhow::bail!("context pack file source type is not allowed"),
    }
}

pub fn is_system_file(relative_path: &str) -> bool {
    STANDARD_FILENAMES.contains(&relative_path)
}

pub fn infer_content_type(relative_path: &str) -> &'static str {
    let lower = relative_path.to_ascii_lowercase();
    if lower.ends_with(".md") {
        "text/markdown"
    } else if lower.ends_with(".txt")
        || lower.ends_with(".json")
        || lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
    {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

fn infer_file_kind(relative_path: &str) -> &'static str {
    if relative_path.starts_with("skills/") {
        "bundle"
    } else if is_text_like_path(relative_path) {
        "document"
    } else {
        "asset"
    }
}

fn infer_loadable(relative_path: &str, content_type: &str, file_kind: &str) -> bool {
    if is_script_path(relative_path) {
        return false;
    }
    if file_kind == "asset" {
        return false;
    }
    is_text_like_path(relative_path) || content_type.starts_with("text/")
}

fn is_text_like_path(relative_path: &str) -> bool {
    let lower = relative_path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".json")
        || lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
}

fn is_script_path(relative_path: &str) -> bool {
    let lower = relative_path.to_ascii_lowercase();
    lower.contains("/scripts/") || lower.ends_with(".sh") || lower.ends_with(".bash")
}

fn parse_manifest(content: &str) -> Result<PackManifest> {
    let mut required_files = Vec::new();
    let mut load_order = Vec::new();
    let mut current_list: Option<&str> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            match current_list {
                Some("required_files") => required_files.push(item.trim().to_string()),
                Some("load_order") => load_order.push(item.trim().to_string()),
                Some(_) | None => {}
            }
            continue;
        }
        current_list = match line {
            "required_files:" | "required_documents:" | "required:" => Some("required_files"),
            "load_order:" => Some("load_order"),
            _ => None,
        };
    }

    for filename in required_files.iter().chain(load_order.iter()) {
        validate_filename(filename)?;
    }
    if has_duplicate(&load_order) {
        anyhow::bail!("context pack load order contains duplicate documents");
    }

    Ok(PackManifest {
        required_files,
        load_order,
    })
}

fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename == "."
        || filename == ".."
        || filename.starts_with('.')
    {
        anyhow::bail!("context pack document filename is not allowed");
    }
    if !filename
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("context pack document filename must be uppercase markdown");
    }
    if !filename.ends_with(".md") {
        anyhow::bail!("context pack document filename must end in .md");
    }
    Ok(())
}

fn has_duplicate(values: &[String]) -> bool {
    for (index, value) in values.iter().enumerate() {
        if values.iter().skip(index + 1).any(|other| other == value) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(filename: &str, content: &str) -> ContextPackDocumentInput {
        ContextPackDocumentInput {
            filename: filename.to_string(),
            content: content.to_string(),
            relative_path: None,
            content_type: None,
            load_order: None,
            required: None,
            file_kind: None,
            loadable: None,
            source_type: None,
        }
    }

    #[test]
    fn validates_operating_package_standard_and_custom_markdown_files() {
        let validated = validate_documents(&[
            document(
                "PACK.md",
                "name: Operating Pack\nrequired_documents:\n- WORKFLOWS.md\nload_order:\n- PACK.md\n- WORKFLOWS.md\n- PROMPTS.md\n- CUSTOM-STANDARD.md\n",
            ),
            document("WORKFLOWS.md", "workflow guidance only"),
            document("PROMPTS.md", "prompt templates only"),
            document("CUSTOM-STANDARD.md", "custom markdown is allowed"),
        ])
        .expect("context pack validates");

        let filenames = validated
            .documents
            .iter()
            .map(|document| document.filename.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            filenames,
            vec![
                "PACK.md",
                "WORKFLOWS.md",
                "PROMPTS.md",
                "CUSTOM-STANDARD.md"
            ]
        );
        assert!(
            validated
                .documents
                .iter()
                .any(|document| document.filename == "WORKFLOWS.md" && document.required)
        );
    }
}
