use crate::{config::reload::ConfigIo, import::SessionFactory};
use cadence::{debug::model::{self, Apply, Status}, envelope::Refusal, store::{Result, writer::Operation}};
use serde_json::{Value, json};
use std::path::Path;

pub enum Command { List, Read { slug: String }, Apply(Apply) }

pub async fn execute<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    let slug = match &command { Command::List => None, Command::Read { slug } => Some(slug.clone()),
        Command::Apply(apply) => Some(apply.identity().1.to_owned()) };
    Ok(match execute_inner(factory, root, command).await {
        Ok(answer) => answer,
        Err(error) => Refusal::new("debug-unavailable", error.to_string()).slot("slug").details(json!({"slug":slug})).value(),
    })
}

async fn execute_inner<I: ConfigIo + Clone + Sync>(factory: &SessionFactory<I>, root: &Path, command: Command) -> Result<Value> {
    if !matches!(command, Command::Apply(_)) {
        // Continuation has no configuration dependency, including in a stopped
        // copy. The shared Store reader verifies all three journal files.
        let pending = || match std::fs::symlink_metadata(root.join(".store-intent.json")) {
            Ok(_) => Err(cadence::store::Error::Conflict("pending Store intent requires recovery".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
        pending()?;
        let snapshot = cadence::store::cache::read(root)?;
        pending()?;
        let empty = json!({});
        let data = snapshot.as_ref().map(|s| &s.data).unwrap_or(&empty);
        return match command {
            Command::List => Ok(json!({"status":"ok","records":model::namespace(data)?.records.into_values()
                .filter(|r| r.status == Status::Open).collect::<Vec<_>>()})),
            Command::Read { slug } => {
                model::validate_slug(&slug)?;
                Ok(model::namespace(data)?.records.get(&slug).map(model::answer).unwrap_or_else(|| model::unknown(&slug)))
            }
            Command::Apply(_) => unreachable!("read branch"),
        };
    }
    let session = factory.first_touch(root).await?;
    let store = session.review_store();
    let view = store.request(Operation::ReadVerified).await?;
    let data = &view.snapshot.data;
    match command {
        Command::List | Command::Read { .. } => unreachable!("read before config"),
        Command::Apply(apply) => {
            let mut write = model::Write { root_binding: cadence::verification::inputs::root_binding(root)?, apply, recall: None };
            if let Some(answer) = model::replay(data, &write)? { return Ok(answer); }
            cadence::milestone::model::name(write.apply.identity().0)?;
            if let Apply::Open { request } = &write.apply
                && model::outcome(data, &write).is_ok() {
                let recalled = super::recall::resident::answer(&session, root, &request.symptom, None, None, &mut None).await?;
                write.recall = Some(serde_json::from_value(serde_json::to_value(recalled)?)?);
            }
            let response = match model::outcome(data, &write) { Ok(record) => model::answer(&record), Err(refusal) => refusal };
            store.request(Operation::DebugV1 { expected_generation: view.snapshot.generation,
                expected_integrity: view.snapshot.integrity.clone(), write: Box::new(write) }).await?;
            Ok(response)
        }
    }
}
