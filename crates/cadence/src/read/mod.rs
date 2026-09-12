pub mod location;
pub mod model;
pub mod outline;
pub mod search;
pub mod slice;
pub mod source;

use location::Registry;
use model::{DocumentRequest, ReadRequest, SearchRequest, Unit};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub use location::Capability;

#[derive(Clone, Debug)]
pub enum Query { Search(SearchRequest), Read(ReadRequest), Document(DocumentRequest) }

pub struct ReadDomain { pub(super) project: PathBuf, pub(super) registry: Registry }

impl ReadDomain {
    pub fn new(planning_root: &Path) -> Result<Self, String> {
        let project = planning_root.parent().ok_or_else(|| "planning root has no project parent".to_string())?;
        Ok(Self { project: std::fs::canonicalize(project).map_err(|error| error.to_string())?, registry: Registry::default() })
    }
    fn units(&self, path: &Path, content: &str) -> Vec<Unit> { outline::units(path, content) }
    pub fn query(&mut self, query: Query) -> Value {
        match query { Query::Search(request) => self.search(request), Query::Read(request) => self.read(request), Query::Document(_) => json!({"status":"refused","code":"document-not-delivered","rule":"D-145","slot":"operation","reason":"document is reserved for phase 31 plan 2"}) }
    }
}
