use super::model::Unit;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

#[derive(Clone)]
pub enum Capability {
    Unit { path: PathBuf, revision: String, unit: Unit, offset: usize },
    File { path: PathBuf, revision: String },
}

pub struct Registry {
    nonce: String,
    next: u64,
    entries: BTreeMap<String, Capability>,
}

impl Default for Registry {
    fn default() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let mut hasher = Sha256::new();
        hasher.update(format!("{now}:{}", std::process::id()));
        Self { nonce: format!("{:x}", hasher.finalize())[..16].to_owned(), next: 0, entries: BTreeMap::new() }
    }
}

impl Registry {
    fn issue(&mut self, prefix: &str, value: Capability) -> String {
        self.next += 1;
        let token = format!("{prefix}-{}-{}", self.nonce, self.next);
        self.entries.insert(token.clone(), value);
        token
    }
    pub fn unit(&mut self, path: PathBuf, revision: String, unit: Unit, offset: usize) -> String {
        self.issue("loc", Capability::Unit { path, revision, unit, offset })
    }
    pub fn file(&mut self, path: PathBuf, revision: String) -> String {
        self.issue("file", Capability::File { path, revision })
    }
    pub fn get(&self, token: &str) -> Option<Capability> { self.entries.get(token).cloned() }
}
