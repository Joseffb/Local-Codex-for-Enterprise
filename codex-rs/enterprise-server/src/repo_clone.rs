use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use std::net::IpAddr;
use std::path::Path;
use std::path::PathBuf;
use tokio::process::Command;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneRepoPlan {
    pub repo_url: String,
    pub workspace_root: String,
    pub destination_name: String,
    pub destination_path: String,
}

pub fn plan_clone(
    repo_url: &str,
    workspace_root: impl AsRef<Path>,
    destination_name: &str,
) -> Result<CloneRepoPlan> {
    let url = validate_repo_url(repo_url)?;
    validate_destination_name(destination_name)?;
    let root = workspace_root.as_ref().canonicalize().with_context(|| {
        format!(
            "canonicalize workspace root {}",
            workspace_root.as_ref().display()
        )
    })?;
    let destination = root.join(destination_name);
    if destination.exists() {
        anyhow::bail!("repo clone destination already exists");
    }
    if destination.components().count() != root.components().count() + 1 {
        anyhow::bail!("repo clone destination must stay directly under workspace root");
    }

    Ok(CloneRepoPlan {
        repo_url: url.to_string(),
        workspace_root: root.to_string_lossy().to_string(),
        destination_name: destination_name.to_string(),
        destination_path: destination.to_string_lossy().to_string(),
    })
}

pub fn validate_clone_request(repo_url: &str, destination_name: &str) -> Result<()> {
    let _ = validate_repo_url(repo_url)?;
    validate_destination_name(destination_name)
}

pub async fn clone_repo(plan: &CloneRepoPlan) -> Result<()> {
    let status = Command::new("git")
        .arg("-c")
        .arg("protocol.file.allow=never")
        .arg("-c")
        .arg("protocol.ext.allow=never")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--")
        .arg(&plan.repo_url)
        .arg(&plan.destination_path)
        .status()
        .await
        .context("run git clone")?;
    if !status.success() {
        anyhow::bail!("git clone failed with status {status}");
    }
    Ok(())
}

fn validate_repo_url(repo_url: &str) -> Result<Url> {
    if looks_like_scp_url(repo_url) {
        anyhow::bail!("repo URL must be HTTPS");
    }
    let url = Url::parse(repo_url).context("parse repo URL")?;
    if url.scheme() != "https" {
        anyhow::bail!("repo URL must use https");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("repo URL must not embed credentials");
    }
    let Some(host) = url.host_str() else {
        anyhow::bail!("repo URL must include a host");
    };
    let host_lower = host.to_ascii_lowercase();
    if matches!(
        host_lower.as_str(),
        "localhost" | "metadata.google.internal"
    ) {
        anyhow::bail!("repo URL host is not allowed");
    }
    if let Ok(ip) = host_lower.parse::<IpAddr>()
        && is_disallowed_ip(ip)
    {
        anyhow::bail!("repo URL host is not allowed");
    }
    Ok(url)
}

fn looks_like_scp_url(value: &str) -> bool {
    if value.contains("://") {
        return false;
    }
    let Some(colon_index) = value.find(':') else {
        return false;
    };
    let slash_index = value.find('/').unwrap_or(usize::MAX);
    colon_index < slash_index
}

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.octets() == [169, 254, 169, 254]
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

fn validate_destination_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
        anyhow::bail!("repo clone destination name is not allowed");
    }
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("repo clone destination must be a name, not a path");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("repo clone destination contains unsupported characters");
    }
    Ok(())
}

pub fn destination_path_from_plan(plan: &CloneRepoPlan) -> PathBuf {
    PathBuf::from(&plan.destination_path)
}

pub fn redact_repo_url_for_storage(repo_url: &str) -> String {
    match Url::parse(repo_url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.to_string()
        }
        Err(_) => "<invalid-repo-url>".to_string(),
    }
}
