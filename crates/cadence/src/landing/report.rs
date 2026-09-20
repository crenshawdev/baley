//! Read-only Git and tracker observations; no tracker filing belongs here.
use super::{effects, forge, model::{Forge, Landing}};
use crate::store::{Error, Result};
use serde_json::{Value, json};
use std::path::Path;

pub fn git(root: &Path, landing: &Landing) -> Result<Value> {
    let branch = effects::observe(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let head = effects::observe(root, &["rev-parse", "HEAD"])?;
    let dirty = !effects::observe(root, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty();
    let url = effects::observe(root, &["remote", "get-url", "--all", &landing.remote.name])?;
    let push_url = effects::observe(root, &["remote", "get-url", "--push", "--all", &landing.remote.name])?;
    let source_head = effects::remote_head(root, landing, &format!("refs/heads/{}", landing.source.branch))?;
    let base_head = effects::remote_head(root, landing, &format!("refs/heads/{}", landing.base.branch))?;
    let comparison = source_head.as_ref().unwrap_or(&landing.base.head);
    let ahead = effects::observe(root, &["rev-list", "--count", &format!("{comparison}..{head}")])?
        .parse::<u64>().map_err(|_| Error::Invalid("invalid Git ahead count".into()))?;
    Ok(json!({"branch":branch,"head":head,"dirty":dirty,"ahead":ahead,"ahead_of":comparison,
        "remote":{"name":landing.remote.name,"url":url,"push_url":push_url,"source_head":source_head,"base_head":base_head}}))
}

pub fn tracker(root: &Path, config: &Value) -> Value {
    let Some(provider) = config.pointer("/git/forge_provider").and_then(Value::as_str) else {
        return json!({"status":"unconfigured","read_only":true});
    };
    let repo = config.pointer("/git/forge_repo").and_then(Value::as_str).unwrap_or("");
    let host = config.pointer("/git/forge_host").and_then(Value::as_str).unwrap_or(match provider {
        "github" => "github.com", "gitlab" => "gitlab.com", _ => "",
    });
    let forge = Forge { provider: provider.into(), repo: repo.into(), host: host.into() };
    let result = forge::configured(&forge, config).and_then(|_| effects::run(root, &forge::tracker(&forge)))
        .and_then(|output| serde_json::from_str::<Value>(&output).map_err(Into::into));
    match result {
        Ok(issues) if issues.is_array() => json!({"status":"ok","read_only":true,"forge":forge,"issues":issues,"bounded":true}),
        Ok(_) => json!({"status":"unavailable","read_only":true,"reason":"tracker response is not an issue list"}),
        Err(error) => json!({"status":"unavailable","read_only":true,"reason":error.to_string()}),
    }
}
