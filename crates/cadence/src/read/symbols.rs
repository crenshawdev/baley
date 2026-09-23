use super::{
    bound::{self, Stop}, location::Resumes, model::{SymbolSearchRequest, Unit}, outline,
    search::AGGREGATE_PARSE_BUDGET, slice::ANSWER_BOUND, source, ReadDomain,
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};

const NOT_SEARCHED_BOUND: usize = 8192;
const UNREACHED_NOTE: &str = "files without a completed outline are named in not_searched";
const BOUNDED_NOTE: &str = "more symbols may remain; continue with the cursor";
const OVERSIZED_NOTE: &str = "symbol rows exceeding the answer bound were skipped";

struct Scan {
    rows: Vec<(usize, usize, Unit)>,
    unreached: Vec<usize>,
}

fn scan(
    files: &[(PathBuf, String)],
    name: &str,
    case_insensitive: bool,
    start: (usize, usize),
    want: usize,
    aggregate: Duration,
    now: &mut dyn FnMut() -> Duration,
) -> Scan {
    let mut result = Scan { rows: Vec::new(), unreached: Vec::new() };
    if want == 0 { return result; }
    let started = now();
    let needle = if case_insensitive { name.to_lowercase() } else { name.to_owned() };
    for (index, (path, content)) in files.iter().enumerate().skip(start.0) {
        if now().saturating_sub(started) >= aggregate {
            result.unreached.extend(index..files.len());
            break;
        }
        let outline = outline::outline(path, content, ANSWER_BOUND, now);
        if outline.error.is_some() {
            result.unreached.push(index);
            continue;
        }
        let matches = outline.units.into_iter().filter(|unit| {
            if case_insensitive { unit.name.to_lowercase().contains(&needle) }
            else { unit.name.contains(&needle) }
        });
        let skip = if index == start.0 { start.1 } else { 0 };
        for (ordinal, unit) in matches.enumerate().skip(skip) {
            result.rows.push((index, ordinal, unit));
            if result.rows.len() == want { return result; }
        }
    }
    result
}

struct Symbol {
    candidate: usize,
    ordinal: usize,
    path: PathBuf,
    revision: String,
    unit: Unit,
}

impl ReadDomain {
    pub(super) fn symbol_search(&mut self, request: SymbolSearchRequest) -> Value {
        let limit = match bound::limit(request.limit) { Ok(limit) => limit, Err(answer) => return answer };
        let case_insensitive = request.case_insensitive.unwrap_or(false);
        let resumes = Resumes::SymbolSearch { name: request.name.clone(), scope: request.scope.clone(), case_insensitive };
        let resume = match self.resume(request.cursor.as_deref(), &resumes) { Ok(resume) => resume, Err(answer) => return answer };
        let candidates = match self.candidates(&request.scope) { Ok(paths) => paths, Err(answer) => return answer };
        let start = resume.as_ref().map_or(0, |(path, _)| candidates.partition_point(|candidate| candidate < path));
        let mut now = outline::monotonic();
        let mut symbols = Vec::new();
        let mut unreached = Vec::new();
        let mut skipped = source::Skipped::default();
        let mut frontier = None;
        let mut resume_file = None;
        for (index, candidate) in candidates.iter().enumerate().skip(start) {
            if now() >= AGGREGATE_PARSE_BUDGET {
                unreached.extend(index..candidates.len());
                resume_file = bound::resume_at(index, Stop::Budget, candidates.len());
                break;
            }
            if outline::grammar_for_path(candidate).is_none() { continue; }
            let (path, revision, content) = match source::content(&self.project, candidate) {
                Ok(content) => content,
                Err(crate::acquisition::Error::Crossing(crossing)) => { skipped.push(crossing); unreached.push(index); continue; }
                Err(_) => { unreached.push(index); continue; }
            };
            let ordinal = resume.as_ref().filter(|(file, _)| file == candidate).map_or(0, |(_, ordinal)| *ordinal);
            let remaining = AGGREGATE_PARSE_BUDGET.saturating_sub(now());
            let result = scan(&[(path.clone(), content)], &request.name, case_insensitive,
                (0, ordinal), limit + 1 - symbols.len(), remaining, &mut now);
            if !result.unreached.is_empty() {
                // A file that failed on its own is named and passed; only a
                // spent budget stops the scan and is retried on the next page.
                if now() < AGGREGATE_PARSE_BUDGET { unreached.push(index); continue; }
                unreached.extend(index..candidates.len());
                resume_file = bound::resume_at(index, Stop::Budget, candidates.len());
                break;
            }
            for (_, ordinal, unit) in result.rows {
                symbols.push(Symbol { candidate: index, ordinal, path: path.clone(), revision: revision.clone(), unit });
            }
            if symbols.len() == limit + 1 {
                frontier = symbols.last().map(|symbol| (symbol.candidate, symbol.ordinal + 1));
                break;
            }
        }
        let not_searched: Vec<_> = unreached.iter().map(|&index|
            json!(candidates[index].strip_prefix(&self.project).unwrap_or(&candidates[index]).to_string_lossy())
        ).collect();
        let named = bound::fit(NOT_SEARCHED_BOUND - 2, &not_searched);
        let mut envelope = source::with_skipped(json!({"status":"ok","kind":"symbol-search","bound":ANSWER_BOUND,
            "limit":limit,"incomplete":false,"cursor":bound::longest_token("cur"),"rows":[],
            "not_searched":&not_searched[..named],"not_searched_total":unreached.len(),
            "notes":[UNREACHED_NOTE, BOUNDED_NOTE, OVERSIZED_NOTE]}), &skipped);
        let mut wire: Vec<_> = symbols.iter().map(|symbol| json!({
            "file":symbol.path.strip_prefix(&self.project).unwrap_or(&symbol.path).to_string_lossy(),
            "name":symbol.unit.name,"kind":symbol.unit.kind,"range":symbol.unit.range(),
            "location":bound::longest_token("loc"),
        })).collect();
        let page = bound::page(&wire, bound::room(&envelope), limit);
        let next = page.next.map(|index| (symbols[index].candidate, symbols[index].ordinal))
            .or_else(|| resume_file.map(|index| (index, 0)))
            .or(frontier);
        let cursor = next.map(|(index, ordinal)| self.registry.cursor(resumes, candidates[index].clone(), ordinal));
        let mut emitted = Vec::new();
        for &index in &page.served {
            let symbol = &symbols[index];
            wire[index]["location"] = json!(self.registry.unit(symbol.path.clone(), symbol.revision.clone(),
                symbol.unit.clone(), symbol.unit.first_byte));
            emitted.push(wire[index].take());
        }
        let mut notes = Vec::new();
        if !unreached.is_empty() { notes.push(UNREACHED_NOTE); }
        if cursor.is_some() { notes.push(BOUNDED_NOTE); }
        if !page.passed.is_empty() { notes.push(OVERSIZED_NOTE); }
        envelope["incomplete"] = json!(cursor.is_some() || !unreached.is_empty() || !skipped.notes.is_empty() || !page.passed.is_empty());
        envelope["cursor"] = json!(cursor);
        envelope["rows"] = json!(emitted);
        envelope["notes"] = json!(notes.into_iter().map(str::to_owned).chain(skipped.notes).collect::<Vec<_>>());
        envelope
    }
}

#[cfg(test)]
mod tests;
