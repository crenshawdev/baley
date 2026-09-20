//! Explicit API endpoints keep forge effects limited to the authorized step.
use super::{effects::Invocation, model::{ExternalInput, Forge, Landing}};
use crate::store::{Error, Result};
use serde_json::Value;

pub fn validate(forge: &Forge) -> Result<()> {
    let safe = |part: &str| !part.is_empty() && part != "." && part != ".." && !part.starts_with('-')
        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if !matches!(forge.provider.as_str(), "github" | "gitlab" | "forgejo")
        || forge.repo.len() > 200 || forge.repo.split('/').count() < 2 || !forge.repo.split('/').all(safe)
        || forge.host.len() > 253 || !forge.host.split(':').all(|part| part.split('.').all(safe)) {
        return Err(Error::Invalid("forge requires a supported provider, explicit repository and host".into()));
    }
    Ok(())
}

pub fn configured(forge: &Forge, config: &Value) -> Result<()> {
    validate(forge)?;
    let host = config.pointer("/git/forge_host").and_then(Value::as_str).unwrap_or(match forge.provider.as_str() {
        "github" => "github.com", "gitlab" => "gitlab.com", _ => "",
    });
    if config.pointer("/git/forge_provider").and_then(Value::as_str) != Some(&forge.provider)
        || config.pointer("/git/forge_repo").and_then(Value::as_str) != Some(&forge.repo) || host != forge.host {
        return Err(Error::Invalid("authorized forge differs from effective configuration".into()));
    }
    Ok(())
}

fn api(forge: &Forge, method: &str, endpoint: String, fields: Vec<(&str, String)>) -> Invocation {
    let tea = forge.provider == "forgejo";
    let program = match forge.provider.as_str() { "github" => "gh", "gitlab" => "glab", _ => "tea" };
    let mut args = vec!["api".into(), "--method".into(), method.into()];
    args.extend([if tea { "--login" } else { "--hostname" }.into(), forge.host.clone()]);
    for (key, value) in fields {
        let flag = if tea || key == "should_remove_source_branch" { "--field" } else { "--raw-field" };
        args.extend([flag.into(), format!("{key}={value}")]);
    }
    args.push(endpoint);
    Invocation { program: program.into(), args }
}

pub fn mutation(landing: &Landing, inputs: &ExternalInput) -> Result<Invocation> {
    match inputs {
        ExternalInput::Open { forge, title, body } => {
            let (endpoint, fields) = if forge.provider == "gitlab" {
                (format!("projects/{}/merge_requests", forge.repo.replace('/', "%2F")), vec![
                    ("source_branch", landing.source.branch.clone()), ("target_branch", landing.base.branch.clone()),
                    ("title", title.clone()), ("description", body.clone())])
            } else {
                (format!("repos/{}/pulls", forge.repo), vec![("head", landing.source.branch.clone()),
                    ("base", landing.base.branch.clone()), ("title", title.clone()), ("body", body.clone())])
            };
            Ok(api(forge, "POST", endpoint, fields))
        }
        ExternalInput::Merge { forge, pr } => {
            let (method, endpoint, fields) = match forge.provider.as_str() {
                "gitlab" => ("PUT", format!("projects/{}/merge_requests/{pr}/merge", forge.repo.replace('/', "%2F")),
                    vec![("sha", landing.source.head.clone()), ("should_remove_source_branch", "false".into())]),
                "forgejo" => ("POST", format!("repos/{}/pulls/{pr}/merge", forge.repo),
                    vec![("Do", "merge".into()), ("head_commit_id", landing.source.head.clone())]),
                _ => ("PUT", format!("repos/{}/pulls/{pr}/merge", forge.repo),
                    vec![("sha", landing.source.head.clone()), ("merge_method", "merge".into())]),
            };
            Ok(api(forge, method, endpoint, fields))
        }
        _ => Err(Error::Invalid("step is not a forge mutation".into())),
    }
}

pub fn tracker(forge: &Forge) -> Invocation {
    let endpoint = if forge.provider == "gitlab" {
        format!("projects/{}/issues?state=opened&per_page=100", forge.repo.replace('/', "%2F"))
    } else { format!("repos/{}/issues?state=open&limit=100", forge.repo) };
    api(forge, "GET", endpoint, vec![])
}
