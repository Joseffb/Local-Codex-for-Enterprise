use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use utoipa::ToSchema;

pub const PACK_MANIFEST: &str = "PACK.md";

const STANDARD_FILENAMES: &[&str] = &[
    "PACK.md",
    "CALIBRATION.md",
    "OPERATING-INSTRUCTIONS.md",
    "PROJECT-RULES.md",
    "HANDOFF.md",
    "VERIFICATION.md",
    "ESCALATION.md",
    "CONTEXT.md",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContextPackDocumentInput {
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedContextDocument {
    pub filename: String,
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
        validated.push(ValidatedContextDocument {
            filename: filename.clone(),
            content_hash: content_hash(&document.content),
            load_order: index as i32,
            required: manifest.required_files.contains(&filename),
        });
    }

    Ok(ValidatedContextPack {
        documents: validated,
    })
}

pub fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("sha256:{digest:x}")
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
            "required_files:" | "required:" => Some("required_files"),
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
