use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDecision {
    pub allowed: bool,
    pub resolved_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePolicy {
    allowed_roots: Vec<PathBuf>,
}

impl WorkspacePolicy {
    pub fn new(roots: Vec<PathBuf>) -> Result<Self> {
        let mut allowed_roots = Vec::with_capacity(roots.len());
        for root in roots {
            allowed_roots.push(
                root.canonicalize()
                    .with_context(|| format!("canonicalize workspace root {}", root.display()))?,
            );
        }
        Ok(Self { allowed_roots })
    }

    pub fn authorize(&self, requested: impl AsRef<Path>) -> Result<WorkspaceDecision> {
        let requested = requested.as_ref();
        let resolved_path = requested
            .canonicalize()
            .with_context(|| format!("canonicalize requested workspace {}", requested.display()))?;
        let allowed = self
            .allowed_roots
            .iter()
            .any(|root| resolved_path == *root || resolved_path.starts_with(root));
        Ok(WorkspaceDecision {
            allowed,
            resolved_path,
        })
    }
}
