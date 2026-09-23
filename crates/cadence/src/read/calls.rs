use super::{
    bound::{self, Stop}, location::Resumes, model::CallSearchRequest, outline::{self, Grammar},
    search::{self, FileHits, AGGREGATE_PARSE_BUDGET}, slice::ANSWER_BOUND, source, ReadDomain,
};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, time::Duration};
use tree_sitter::Node;

const CALL_MATCHING: &str = "syntactic: calls are matched by the callee's last identifier in the syntax tree, not type-resolved";
const STOPPED_NOTE: &str = "call parsing stopped at an incomplete parse or the aggregate budget; remaining files were not searched";
const OUTLINE_NOTE: &str = "the aggregate budget was spent resolving enclosing units; calls may use window locations";
const BOUNDED_NOTE: &str = "more calls may remain; continue with the cursor";
const OVERSIZED_NOTE: &str = "call rows exceeding the answer bound were skipped";

fn call_grammar(path: &Path) -> Option<Grammar> {
    match outline::grammar_for_path(path) {
        Some(grammar @ (Grammar::Rust | Grammar::JavaScript | Grammar::Python | Grammar::C)) => Some(grammar),
        _ => None,
    }
}

fn last_identifier(mut callee: Node<'_>, grammar: Grammar) -> Option<Node<'_>> {
    loop {
        match (grammar, callee.kind()) {
            (_, "identifier") => return Some(callee),
            (Grammar::Rust, "generic_function") => callee = callee.child_by_field_name("function")?,
            (Grammar::Rust, "scoped_identifier") => return callee.child_by_field_name("name"),
            (Grammar::Rust | Grammar::C, "field_expression") => {
                return callee.child_by_field_name("field").filter(|field| field.kind() == "field_identifier");
            }
            (Grammar::JavaScript, "member_expression") => return callee.child_by_field_name("property"),
            (Grammar::Python, "attribute") => return callee.child_by_field_name("attribute"),
            _ => return None,
        }
    }
}

fn sites(content: &str, grammar: Grammar, name: &str, now: &mut dyn FnMut() -> Duration) -> Option<Vec<usize>> {
    let tree = outline::parse(content, grammar, outline::PARSE_BUDGET, now)?;
    let mut lines = Vec::new();
    outline::walk(&tree, |node, _| {
        let field = match (grammar, node.kind()) {
            (Grammar::Rust | Grammar::JavaScript | Grammar::C, "call_expression") => "function",
            (Grammar::Rust, "macro_invocation") => "macro",
            (Grammar::JavaScript, "new_expression") => "constructor",
            (Grammar::Python, "call") => "function",
            _ => return,
        };
        let Some(callee) = node.child_by_field_name(field) else { return; };
        if node.kind() == "macro_invocation" && !matches!(callee.kind(), "identifier" | "scoped_identifier") { return; }
        if last_identifier(callee, grammar).and_then(|identifier| identifier.utf8_text(content.as_bytes()).ok()) == Some(name) {
            lines.push(node.start_position().row + 1);
        }
    }).ok()?;
    Some(lines)
}

struct Call {
    candidate: usize,
    ordinal: usize,
    path: PathBuf,
    revision: String,
    row: search::Row,
}

impl ReadDomain {
    pub(super) fn call_search(&mut self, request: CallSearchRequest) -> Value {
        let limit = match bound::limit(request.limit) { Ok(limit) => limit, Err(answer) => return answer };
        let resumes = Resumes::CallSearch { name: request.name.clone(), scope: request.scope.clone() };
        let resume = match self.resume(request.cursor.as_deref(), &resumes) { Ok(resume) => resume, Err(answer) => return answer };
        let candidates = match self.candidates(&request.scope) { Ok(paths) => paths, Err(answer) => return answer };
        let start = resume.as_ref().map_or(0, |(path, _)| candidates.partition_point(|candidate| candidate < path));
        let mut now = outline::monotonic();
        let mut calls = Vec::new();
        let mut not_searched = 0;
        let mut skipped = source::Skipped::default();
        let mut stopped = false;
        let mut resume_file = None;
        let mut outline_spent = false;
        let mut frontier = None;
        for (index, candidate) in candidates.iter().enumerate().skip(start) {
            if now() >= AGGREGATE_PARSE_BUDGET {
                not_searched += candidates.len() - index;
                stopped = true;
                resume_file = bound::resume_at(index, Stop::Budget, candidates.len());
                break;
            }
            let Some(grammar) = call_grammar(candidate) else { not_searched += 1; continue; };
            let (path, revision, content) = match source::content(&self.project, candidate) {
                Ok(content) => content,
                Err(crate::acquisition::Error::Crossing(crossing)) => { skipped.push(crossing); not_searched += 1; continue; }
                Err(_) => { not_searched += 1; continue; }
            };
            if !content.contains(&request.name) { continue; }
            if now() >= AGGREGATE_PARSE_BUDGET {
                not_searched += candidates.len() - index;
                stopped = true;
                resume_file = bound::resume_at(index, Stop::Budget, candidates.len());
                break;
            }
            let Some(lines) = sites(&content, grammar, &request.name, &mut now) else {
                let stop = if now() >= AGGREGATE_PARSE_BUDGET { Stop::Budget } else { Stop::Failed };
                not_searched += candidates.len() - index;
                stopped = true;
                resume_file = bound::resume_at(index, stop, candidates.len());
                break;
            };
            let ordinal = resume.as_ref().filter(|(file, _)| file == candidate).map_or(0, |(_, ordinal)| *ordinal);
            let lines: Vec<_> = lines.into_iter().skip(ordinal).take(limit + 1 - calls.len()).collect();
            if lines.is_empty() { continue; }
            let files = [FileHits { path, revision, content, lines }];
            let remaining = AGGREGATE_PARSE_BUDGET.saturating_sub(now());
            let answer = search::plan_answer(&files, remaining, &mut now);
            outline_spent |= now() >= AGGREGATE_PARSE_BUDGET;
            for (offset, row) in search::rows(&files, &answer, &self.project).into_iter().enumerate() {
                calls.push(Call { candidate: index, ordinal: ordinal + offset,
                    path: files[0].path.clone(), revision: files[0].revision.clone(), row });
            }
            if calls.len() == limit + 1 {
                frontier = calls.last().map(|call| (call.candidate, call.ordinal + 1));
                break;
            }
        }
        let mut envelope = source::with_skipped(json!({"status":"ok","kind":"call-search","bound":ANSWER_BOUND,
            "limit":limit,"matching":CALL_MATCHING,"incomplete":false,"cursor":bound::longest_token("cur"),
            "rows":[],"not_searched":not_searched,"notes":[STOPPED_NOTE, OUTLINE_NOTE, BOUNDED_NOTE, OVERSIZED_NOTE]}), &skipped);
        let mut wire: Vec<_> = calls.iter().map(|call| {
            let mut value = serde_json::to_value(&call.row).unwrap();
            value["location"] = json!(bound::longest_token("loc"));
            value
        }).collect();
        let page = bound::page(&wire, bound::room(&envelope), limit);
        let next = page.next.map(|index| (calls[index].candidate, calls[index].ordinal))
            .or_else(|| resume_file.map(|index| (index, 0))).or(frontier);
        let cursor = next.map(|(index, ordinal)| self.registry.cursor(resumes, candidates[index].clone(), ordinal));
        let mut emitted = Vec::new();
        for &index in &page.served {
            let call = &calls[index];
            let unit = &call.row.target;
            wire[index]["location"] = json!(self.registry.unit(call.path.clone(), call.revision.clone(), unit.clone(), unit.first_byte));
            emitted.push(wire[index].take());
        }
        let mut notes = Vec::new();
        if stopped { notes.push(STOPPED_NOTE); }
        if outline_spent { notes.push(OUTLINE_NOTE); }
        if cursor.is_some() { notes.push(BOUNDED_NOTE); }
        if !page.passed.is_empty() { notes.push(OVERSIZED_NOTE); }
        envelope["incomplete"] = json!(cursor.is_some() || not_searched > 0 || !page.passed.is_empty());
        envelope["cursor"] = json!(cursor);
        envelope["rows"] = json!(emitted);
        envelope["notes"] = json!(notes.into_iter().map(str::to_owned).chain(skipped.notes).collect::<Vec<_>>());
        envelope
    }
}

#[cfg(test)]
mod tests;
