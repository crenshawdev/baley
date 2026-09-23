use super::{Capability, ReadDomain, bound::{self, Page}, location::Resumes, model::{ReadRequest, Unit}, outline, source};
use serde_json::{Value, json};

use crate::envelope::Refusal;
use std::path::{Path, PathBuf};

pub(super) const ANSWER_BOUND: usize = 65_536;

const BOUNDED_NOTE: &str = "outline answer was bounded; repeat the same read with this cursor to continue";

struct OutlineRead<'a> {
    path: PathBuf,
    revision: String,
    unit: Option<&'a str>,
    cursor: Option<&'a str>,
}

fn outline_page(units: &[Unit], start: usize, room: usize) -> Page {
    let start = start.min(units.len());
    let rows: Vec<_> = units[start..].iter().map(|unit| {
        json!({"name":unit.name,"kind":unit.kind,"range":unit.range(),"location":bound::longest_token("loc")})
    }).collect();
    let page = bound::page(&rows, room, usize::MAX);
    Page {
        served: page.served.into_iter().map(|index| start + index).collect(),
        passed: page.passed.into_iter().map(|index| start + index).collect(),
        next: page.next.map(|index| start + index),
    }
}

fn refusal(slot: &str, code: &str, reason: impl Into<String>) -> Value {
    // D-147: the model never originates a location; it reads only at a location the binary handed back.
    Refusal::new(code, reason).rule("issued-location").slot(slot).value()
}

/// The units a caller can name in `path`, and a note when the outline that
/// should have produced them was unavailable.
///
/// A file with no grammar, and a file whose grammar found nothing to name, is
/// one unit spanning the whole file, so every file stays readable by name.
pub(super) fn nameable_units(path: &Path, content: &str) -> (Vec<Unit>, Vec<&'static str>) {
    let outline = outline::outline(path, content, ANSWER_BOUND, &mut outline::monotonic());
    let notes = outline.error.map(outline::OutlineError::note).into_iter().collect();
    if outline.units.is_empty() { (source::fallback(path, content), notes) } else { (outline.units, notes) }
}

impl ReadDomain {
    /// Resolve only resident-issued capabilities, retaining their exact span.
    pub fn acquire(&self, token: &str) -> Result<(String, Vec<u8>), Value> {
        let (path, revision, span) = match self.registry.get(token) {
            Some(Capability::Unit { path, revision, unit, .. }) => (path, revision, Some(unit.first_byte..unit.last_byte)),
            Some(Capability::File { path, revision }) => (path, revision, None),
            _ => return Err(refusal("location", "location-not-issued", "location was not issued by this resident")),
        };
        let content = self.current(&path, &revision)?;
        let bytes = match span {
            Some(span) => content.get(span).ok_or_else(|| refusal("location", "stale-location", "issued span no longer exists"))?.as_bytes().to_vec(),
            None => content.into_bytes(),
        };
        Ok((path.to_string_lossy().into_owned(), bytes))
    }
    pub(super) fn read(&mut self, request: ReadRequest) -> Value {
        match (request.location, request.file, request.unit, request.cursor) {
            (Some(location), None, None, None) => self.read_location(&location),
            (None, Some(file), Some(name), cursor) => self.read_named(&file, &name, cursor.as_deref()),
            (None, Some(file), None, cursor) => self.read_file(&file, cursor.as_deref()),
            _ => refusal("location", "read-contract", "read accepts an issued location without a cursor, or an issued file reference with an optional unit name and outline cursor"),
        }
    }

    fn current(&self, path: &Path, expected: &str) -> Result<String, Value> {
        current_answer(expected, source::content(&self.project, path))
    }

    fn read_location(&mut self, token: &str) -> Value {
        let Some(Capability::Unit { path, revision, unit, offset }) = self.registry.get(token) else {
            return refusal("location", "location-not-issued", "location was not issued by this resident");
        };
        let content = match self.current(&path, &revision) { Ok(content) => content, Err(answer) => return answer };
        if offset < unit.first_byte || offset > unit.last_byte { return refusal("location", "location-not-issued", "location is outside its issued unit"); }
        self.slice(path, revision, unit, offset, &content)
    }

    fn read_named(&mut self, token: &str, name: &str, cursor: Option<&str>) -> Value {
        let Some(Capability::File { path, revision }) = self.registry.get(token) else { return refusal("file", "location-not-issued", "file reference was not issued by this resident"); };
        let content = match self.current(&path, &revision) { Ok(content) => content, Err(answer) => return answer };
        let (units, notes) = nameable_units(&path, &content);
        // An exact qualified name wins outright; otherwise every unit whose
        // bare name matches is a candidate, and two candidates are ambiguous
        // rather than a silent pick.
        let exact: Vec<_> = units.iter().filter(|unit| unit.name == name).cloned().collect();
        let matches = if exact.is_empty() { units.iter().filter(|unit| unit.bare == name).cloned().collect() } else { exact };
        if matches.len() != 1 {
            let missing = matches.is_empty();
            let selected = if missing { units } else { matches };
            let reason = if missing { "missing-unit" } else { "ambiguous-unit" };
            return self.outline(OutlineRead { path, revision, unit: Some(name), cursor }, selected, notes, Some(reason));
        }
        if cursor.is_some() { return refusal("cursor", "read-contract", "a cursor is accepted only for an outline answer"); }
        let unit = matches.into_iter().next().unwrap();
        self.slice(path, revision, unit.clone(), unit.first_byte, &content)
    }

    /// A file reference answers with the whole file when it is small enough to
    /// read at once, and with its outline when it is not.
    ///
    /// The cutoff is grammar-aware: a file with a grammar is outlined from
    /// 24 KB, because an outline is a map of the file and a better answer than
    /// the whole of anything large. A file without a grammar has no map to
    /// offer, so it is served as one slice with a continuation.
    fn read_file(&mut self, token: &str, cursor: Option<&str>) -> Value {
        let Some(Capability::File { path, revision }) = self.registry.get(token) else { return refusal("file", "location-not-issued", "file reference was not issued by this resident"); };
        let content = match self.current(&path, &revision) { Ok(content) => content, Err(answer) => return answer };
        if outline::grammar_for_path(&path).is_none() || content.len() <= outline::OUTLINE_THRESHOLD {
            if cursor.is_some() { return refusal("cursor", "read-contract", "a cursor is accepted only for an outline answer"); }
            let whole = source::fallback(&path, &content).remove(0);
            return self.slice(path, revision, whole, 0, &content);
        }
        let (units, notes) = nameable_units(&path, &content);
        self.outline(OutlineRead { path, revision, unit: None, cursor }, units, notes, None)
    }

    fn slice(&mut self, path: PathBuf, revision: String, unit: Unit, offset: usize, content: &str) -> Value {
        let tail = content.get(offset..unit.last_byte).unwrap_or("");
        let envelope = json!({"status":"ok","kind":"slice","bound":ANSWER_BOUND,"source_revision":revision,
            "name":unit.name,"unit_kind":unit.kind,"requested_range":unit.range(),"served_range":[usize::MAX,usize::MAX],
            "body":"","truncated":false,"continuation":bound::longest_token("loc"),
            "continue_from_line":usize::MAX,"continue_from_byte":usize::MAX});
        let end = offset + bound::fit_text(bound::room(&envelope), tail);
        let truncated = end < unit.last_byte;
        let continuation = truncated.then(|| self.registry.unit(path.clone(), revision.clone(), unit.clone(), end));
        let starts = source::line_starts(content);
        let served_first = source::line_for(&starts, offset);
        let served_last = source::line_for(&starts, end.saturating_sub(1));
        let continue_from_line = truncated.then(|| source::line_for(&starts, end));
        let continue_from_byte = continue_from_line.map(|line| end.saturating_sub(starts[line - 1]));
        json!({"status":"ok","kind":"slice","bound":ANSWER_BOUND,"source_revision":revision,"name":unit.name,"unit_kind":unit.kind,"requested_range":unit.range(),"served_range":[served_first,served_last],"body":content.get(offset..end).unwrap_or(""),"truncated":truncated,"continuation":continuation,"continue_from_line":continue_from_line,"continue_from_byte":continue_from_byte})
    }

    fn outline(&mut self, request: OutlineRead<'_>, units: Vec<Unit>, notes: Vec<&'static str>, reason: Option<&str>) -> Value {
        let OutlineRead { path, revision, unit, cursor } = request;
        let resumes = Resumes::Outline { path: path.clone(), unit: unit.map(str::to_owned) };
        let start = match self.resume(cursor, &resumes) {
            Ok(resume) => resume.map_or(0, |(_, ordinal)| ordinal),
            Err(answer) => return answer,
        };
        let mut envelope = json!({"status":"ok","kind":"outline","bound":ANSWER_BOUND,"source_revision":revision,
            "rows":[],"notes":notes,"incomplete":false,"continuation":Value::Null,"cursor":bound::longest_token("cur")});
        if let Some(reason) = reason { envelope["reason"] = json!(reason); }
        envelope["notes"].as_array_mut().unwrap().push(json!(BOUNDED_NOTE));
        let page = outline_page(&units, start, bound::room(&envelope));
        let rows: Vec<_> = page.served.iter().map(|&index| {
            let unit = &units[index];
            let location = self.registry.unit(path.clone(), revision.clone(), unit.clone(), unit.first_byte);
            json!({"name":unit.name,"kind":unit.kind,"range":unit.range(),"location":location})
        }).collect();
        let cursor = page.next.map(|ordinal| self.registry.cursor(resumes, path, ordinal));
        let mut notes = notes;
        if cursor.is_some() { notes.push(BOUNDED_NOTE); }
        let mut answer = json!({"status":"ok","kind":"outline","bound":ANSWER_BOUND,"source_revision":revision,"rows":rows,"notes":notes,"incomplete":cursor.is_some(),"continuation":Value::Null,"cursor":cursor});
        if let Some(reason) = reason { answer["reason"] = json!(reason); }
        answer
    }
}

fn current_answer(expected: &str, acquired: Result<(PathBuf, String, String), crate::acquisition::Error>) -> Result<String, Value> {
    let (_, revision, content) = acquired.map_err(|error| match error {
        crate::acquisition::Error::Crossing(crossing) => source::incomplete(crossing),
        error => refusal("location", "location-not-issued", error.to_string()),
    })?;
    if revision != expected { return Err(refusal("location", "stale-location", "source changed; reacquire through search")); }
    Ok(content)
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_outline_page_resumes_at_its_first_unserved_unit() {
        let units: Vec<_> = (0..6).map(|index| super::Unit {
            name: format!("u{index}"),
            bare: format!("u{index}"),
            kind: "function",
            first_line: index + 1,
            last_line: index + 1,
            first_byte: index,
            last_byte: index + 1,
        }).collect();
        let first = super::outline_page(&units, 2, 202);
        assert_eq!(first.served, [2, 3]);
        assert!(first.passed.is_empty());
        assert_eq!(first.next, Some(4));
        let last = super::outline_page(&units, 4, 202);
        assert_eq!(last.served, [4, 5]);
        assert!(last.passed.is_empty());
        assert_eq!(last.next, None);
    }

    #[test]
    fn an_issued_file_crossing_precedes_stale_revision() {
        let answer = super::current_answer("issued-old-revision", Err(crate::acquisition::Error::Crossing(
            crate::acquisition::Crossing { file: "src/large.rs".into(), size: 16_777_217, bound: 16_777_216 }
        ))).unwrap_err();
        assert_eq!(answer["incomplete"], true);
        assert_eq!(answer["crossing"], serde_json::json!({"file":"src/large.rs","size":16_777_217,"bound":16_777_216}));
        assert!(answer.get("body").is_none());
        assert!(answer.get("code").is_none());
    }
}
