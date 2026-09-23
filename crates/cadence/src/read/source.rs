use super::model::Unit;
use crate::acquisition::{self, Class, Crossing, Error};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}};

pub fn revision(content: &str) -> String { format!("{:x}", Sha256::digest(content.as_bytes())) }

pub fn confined(project: &Path, path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    if canonical.strip_prefix(project).is_ok() { Some(canonical) } else { None }
}

pub fn content(project: &Path, path: &Path) -> Result<(PathBuf, String, String), Error> {
    let path = confined(project, path).ok_or_else(|| std::io::Error::other("source target escapes the bound project"))?;
    let text = acquisition::text(&path, Class::Source).map_err(|error| match error {
        Error::Crossing(mut crossing) => {
            crossing.file = path.strip_prefix(project).unwrap_or(&path).to_string_lossy().into_owned();
            Error::Crossing(crossing)
        }
        error => error,
    })?;
    Ok((path, revision(&text), text))
}

pub(super) fn incomplete(crossing: Crossing) -> serde_json::Value {
    serde_json::json!({"status":"ok","kind":"acquisition","incomplete":true,
        "crossing":crossing,"notes":[crossing.to_string()]})
}

/// A bounded set of skipped-file notes, reserved before result bodies.
#[derive(Default)]
pub(super) struct Skipped {
    pub notes: Vec<String>,
    used: usize,
    omitted: bool,
}
impl Skipped {
    const BOUND: usize = 8192;
    pub fn push(&mut self, crossing: Crossing) {
        if self.omitted { return; }
        let note = crossing.to_string();
        let cost = serde_json::to_vec(&note).map_or(usize::MAX, |bytes| bytes.len() + 1);
        if cost <= Self::BOUND.saturating_sub(self.used + 128) {
            self.used += cost;
            self.notes.push(note);
        } else {
            self.notes.push("additional acquisition crossings omitted at the skipped-note bound of 8192 bytes".into());
            self.omitted = true;
        }
    }
}

pub(super) fn with_skipped(mut answer: serde_json::Value, skipped: &Skipped) -> serde_json::Value {
    if !skipped.notes.is_empty() {
        answer["incomplete"] = serde_json::json!(true);
        answer["notes"].as_array_mut().expect("result notes")
            .extend(skipped.notes.iter().map(|note| serde_json::json!(note)));
    }
    answer
}

#[cfg(test)]
mod tests {
    #[test]
    fn skipped_crossings_keep_a_bounded_note_and_mark_omissions() {
        let mut skipped = super::Skipped::default();
        for _ in 0..200 {
            skipped.push(crate::acquisition::Crossing { file: "src/large.rs".into(), size: 16_777_217, bound: 16_777_216 });
        }
        assert_eq!(skipped.notes[0], "src/large.rs: size 16777217 exceeds acquisition bound 16777216");
        assert_eq!(skipped.notes.last().unwrap(), "additional acquisition crossings omitted at the skipped-note bound of 8192 bytes");
        assert!(serde_json::to_vec(&skipped.notes).unwrap().len() <= 8192);
    }
}

pub fn line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(content.match_indices('\n').map(|(index, _)| index + 1));
    starts
}

pub fn line_for(starts: &[usize], byte: usize) -> usize {
    starts.partition_point(|start| *start <= byte).max(1)
}

/// The whole file as one unit, for a file with no grammar or no units.
pub fn fallback(path: &Path, content: &str) -> Vec<Unit> {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("source").to_owned();
    let last_line = content.split_inclusive('\n').count().max(1);
    vec![Unit { name: name.clone(), bare: name, kind: "source", first_line: 1, last_line, first_byte: 0, last_byte: content.len() }]
}
