//! Search: the candidate walk, the scope filter, the line matching, the
//! enclosing units and the bounded, pageable answer.
//!
//! Folded in from excerpt's `search.rs`. Matching is ripgrep's own engine used
//! as a library (`grep-regex`, `grep-searcher`), so a pattern means what it
//! means at a shell. Every candidate is opened through `source::content`,
//! never by the searcher itself, so the project confinement, the regular-file
//! test and strict UTF-8 hold for search because they are the read path's own
//! code.

use super::{ReadDomain, bound, location::{Capability, Resumes}, model::{Scope, SearchRequest, Unit}, outline, source};
use globset::{GlobBuilder, GlobMatcher};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder, sinks::UTF8};
use ignore::WalkBuilder;
use serde::Serialize;
use serde_json::{Value, json};

use crate::envelope::Refusal;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const ANSWER_BOUND: usize = super::slice::ANSWER_BOUND;

/// How many lines either side of a matched line a window carries.
///
/// A window is what a hit with no enclosing unit falls back to: a file with no
/// grammar, a parse that failed or spent its budget, or a match in the space
/// between definitions. A bare matched line with no context is worse than
/// grep; dropping the hit, which the brace-counting layer did, is worse still.
const WINDOW_CONTEXT: usize = 2;

/// The name a window hit carries. Parenthesized like Markdown's `(preamble)`
/// so it cannot be mistaken for a name the source declares.
const WINDOW_NAME: &str = "(no enclosing unit)";

/// Wall-clock ceiling on all the tree-sitter parses one search performs.
///
/// [`outline::PARSE_BUDGET`] bounds one file and says nothing about a
/// hundred, and resolving units means parsing every file with at least one
/// hit: excerpt measured 108 files matching `readFile` in this tree at 4 MB,
/// about 54 seconds at the per-parse worst case. Five seconds sits an order
/// of magnitude above the clean parse rate for those same bytes. Spending it
/// costs a unit resolution and never a hit: the files still to resolve take
/// the window path and the answer says so.
pub(super) const AGGREGATE_PARSE_BUDGET: Duration = Duration::from_secs(5);

const STOPPED_NOTE: &str = "unit resolution stopped at the aggregate parse budget; later files are line windows";
const BOUNDED_NOTE: &str = "search answer was bounded; repeat the same search with this cursor to continue";
const EXHAUSTED_NOTE: &str = "the cursor is at or past this search's last match; nothing follows";

fn refusal(slot: &str, code: &str, reason: impl Into<String>) -> Value {
    // D-147: the model never originates a location; it reads only at a location the binary handed back.
    Refusal::new(code, reason).rule("issued-location").slot(slot).value()
}

fn selector(project: &Path, scope: &Scope) -> Result<(PathBuf, Option<GlobMatcher>), Value> {
    match scope {
        Scope::Project => Ok((project.to_path_buf(), None)),
        Scope::Directory { selector } => {
            let path = confined_selector(project, selector).ok_or_else(|| refusal("scope", "invalid-scope", "directory selector must be relative and confined"))?;
            if !path.is_dir() { return Err(refusal("scope", "invalid-scope", "directory selector does not name a directory")); }
            Ok((path, None))
        }
        Scope::Glob { selector } => {
            if absolute_or_escape(selector) { return Err(refusal("scope", "invalid-scope", "glob selector must be relative and confined")); }
            // `literal_separator(true)` because globset defaults it to false:
            // without it `*` crosses `/` and `src/*.rs` matches `src/read/mod.rs`.
            let glob = GlobBuilder::new(selector).literal_separator(true).build()
                .map_err(|error| refusal("scope", "invalid-scope", error.to_string()))?.compile_matcher();
            Ok((project.to_path_buf(), Some(glob)))
        }
        Scope::CurrentTaskLease { .. } => unreachable!("the lease scope is resolved from retained authority"),
    }
}

fn absolute_or_escape(selector: &str) -> bool {
    Path::new(selector).is_absolute() || Path::new(selector).components().any(|part| matches!(part, Component::ParentDir))
}

fn confined_selector(project: &Path, selector: &str) -> Option<PathBuf> {
    (!absolute_or_escape(selector)).then(|| project.join(selector)).and_then(|path| source::confined(project, &path))
}

/// Every regular file under `root` that the glob admits, sorted by path.
///
/// The walk respects `.gitignore` and `.ignore` but does not skip hidden
/// entries, since `.claude/` is exactly what gets searched here. `.git` is
/// excluded by name, and `.planning` because process records are reached by
/// identity through `document`, never by path.
pub(super) fn files(root: &Path, project: &Path, glob: Option<&GlobMatcher>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = WalkBuilder::new(root).hidden(false).require_git(false).follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git" && entry.file_name() != ".planning")
        .build().filter_map(Result::ok).filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path()).filter(|path| source::confined(project, path).is_some())
        .filter(|path| glob.is_none_or(|matcher| matcher.is_match(path.strip_prefix(project).unwrap_or(path)))).collect();
    paths.sort(); paths
}

pub(super) fn matcher(pattern: &str, case_insensitive: bool) -> Result<RegexMatcher, Value> {
    RegexMatcherBuilder::new().case_insensitive(case_insensitive).build(pattern)
        .map_err(|error| refusal("pattern", "invalid-pattern", error.to_string()))
}

/// A searcher that reports line numbers. Without them the sink reports none,
/// and a hit with no line number is a hit nothing can be resolved from.
pub(super) fn searcher() -> Searcher {
    SearcherBuilder::new().line_number(true).build()
}

/// The 1-based line numbers of `content` that match, ascending, each once.
pub(super) fn matching_lines(searcher: &mut Searcher, matcher: &RegexMatcher, content: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let outcome = searcher.search_slice(matcher, content.as_bytes(), UTF8(|number, _| { lines.push(number as usize); Ok(true) }));
    // Searching a slice already in memory has no I/O to fail at, but the
    // sink's signature admits an error; one file's failure is that file's.
    if outcome.is_err() { lines.clear(); }
    lines
}

/// One file that matched, with everything the answer needs from it.
pub(super) struct FileHits {
    /// The canonical path `source::content` resolved.
    pub(super) path: PathBuf,
    pub(super) revision: String,
    /// The content the matching ran against, kept so the answer renders from
    /// the same bytes the line numbers refer to.
    pub(super) content: String,
    /// 1-based matching line numbers, ascending.
    pub(super) lines: Vec<usize>,
}

/// One contiguous span shared by the readable targets of its matching lines.
struct Block {
    file: usize,
    first_line: usize,
    last_line: usize,
    /// The enclosing unit, or `None` when this block is the line window that
    /// stands in for one.
    unit: Option<Unit>,
    /// The matched lines that fall inside it.
    lines: Vec<usize>,
}

/// One matched line, and the block that answers for it.
///
/// The site list, one entry per (file, matching line) in path-then-line
/// order, is the one ordering in a search that nothing downstream can move.
/// Blocks can be re-partitioned between two otherwise identical calls when
/// the aggregate parse budget runs out at a different file; sites cannot,
/// which is what makes a site the thing a cursor is allowed to name.
struct Site {
    file: usize,
    line: usize,
    block: usize,
}

pub(super) struct Answer {
    blocks: Vec<Block>,
    sites: Vec<Site>,
    /// Whether unit resolution stopped at the aggregate parse budget.
    stopped: bool,
}

/// The innermost unit containing 1-based `line`: of the units that contain
/// it, the one spanning the fewest lines. Where two span exactly the same
/// lines, the one appearing later in the vector, which is the nested one for
/// every extractor here since each emits a container before its members.
fn enclosing(units: &[Unit], line: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, unit) in units.iter().enumerate() {
        if unit.first_line > line || line > unit.last_line { continue; }
        let span = unit.last_line - unit.first_line;
        match best {
            Some(current) if span > units[current].last_line - units[current].first_line => {}
            _ => best = Some(index),
        }
    }
    best
}

/// Turn one file's matched lines into blocks, one per enclosing unit however
/// many matches it holds and a merged line window for every match that has
/// none, and report which block each matched line landed in.
///
/// Two window hits five lines apart produce windows that abut; merging on
/// `first <= previous last + 1` keeps them one block. Only the immediately
/// previous block is a merge candidate, and only when it is itself a window.
fn file_blocks(file: usize, hits: &FileHits, units: &[Unit]) -> (Vec<Block>, Vec<usize>) {
    let total = hits.content.split_inclusive('\n').count().max(1);
    let mut blocks: Vec<Block> = Vec::new();
    let mut of_line: Vec<usize> = Vec::with_capacity(hits.lines.len());
    // Unit index -> the block already opened for it.
    let mut opened: Vec<(usize, usize)> = Vec::new();
    for &line in &hits.lines {
        if let Some(unit) = enclosing(units, line) {
            if let Some(&(_, at)) = opened.iter().find(|(index, _)| *index == unit) {
                blocks[at].lines.push(line);
                of_line.push(at);
                continue;
            }
            let found = &units[unit];
            blocks.push(Block { file, first_line: found.first_line, last_line: found.last_line, unit: Some(found.clone()), lines: vec![line] });
            opened.push((unit, blocks.len() - 1));
            of_line.push(blocks.len() - 1);
            continue;
        }
        let first = line.saturating_sub(WINDOW_CONTEXT).max(1);
        let last = (line + WINDOW_CONTEXT).min(total);
        match blocks.last_mut() {
            Some(previous) if previous.unit.is_none() && first <= previous.last_line + 1 => {
                previous.last_line = previous.last_line.max(last);
                previous.lines.push(line);
            }
            _ => blocks.push(Block { file, first_line: first, last_line: last, unit: None, lines: vec![line] }),
        }
        of_line.push(blocks.len() - 1);
    }
    // First-line order, carrying the per-hit block indices: a unit block opens
    // where its first hit is, and a unit can begin well above a window already
    // pushed for an earlier hit.
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by_key(|&index| (blocks[index].first_line, blocks[index].last_line));
    let mut rank = vec![0usize; blocks.len()];
    for (new, &old) in order.iter().enumerate() { rank[old] = new; }
    let mut sorted: Vec<Option<Block>> = blocks.into_iter().map(Some).collect();
    let blocks = order.iter().map(|&index| sorted[index].take().unwrap()).collect();
    for at in of_line.iter_mut() { *at = rank[*at]; }
    (blocks, of_line)
}

/// Lay a hit list out as blocks and sites, resolving units file by file until
/// the aggregate parse budget is spent on the `now` reader.
pub(super) fn plan_answer(files: &[FileHits], aggregate: Duration, now: &mut dyn FnMut() -> Duration) -> Answer {
    let mut blocks: Vec<Block> = Vec::new();
    let mut sites: Vec<Site> = Vec::new();
    let started = now();
    let mut stopped = false;
    for (index, file) in files.iter().enumerate() {
        let base = blocks.len();
        // The files still to resolve are not skipped and their hits are not
        // dropped: they take the window path.
        let units = if stopped || now().saturating_sub(started) >= aggregate {
            stopped = true;
            Vec::new()
        } else {
            outline::outline(&file.path, &file.content, ANSWER_BOUND, now).units
        };
        let (file_blocks, of_line) = file_blocks(index, file, &units);
        blocks.extend(file_blocks);
        for (position, &line) in file.lines.iter().enumerate() {
            sites.push(Site { file: index, line, block: base + of_line[position] });
        }
    }
    Answer { blocks, sites, stopped }
}

/// The unit a block is served as: its enclosing unit, or a window unit built
/// from its lines.
fn block_unit(block: &Block, hits: &FileHits) -> Unit {
    match &block.unit {
        Some(unit) => unit.clone(),
        None => outline::Lines::new(&hits.content).unit(WINDOW_NAME.into(), WINDOW_NAME.into(), "window", block.first_line, block.last_line),
    }
}

#[derive(Serialize)]
pub(super) struct Row {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) text: String,
    pub(super) name: String,
    pub(super) kind: &'static str,
    #[serde(skip)]
    pub(super) target: Unit,
    #[serde(skip)]
    pub(super) file_index: usize,
}

pub(super) fn rows(files: &[FileHits], answer: &Answer, project: &Path) -> Vec<Row> {
    let lines: Vec<_> = files.iter().map(|file| outline::Lines::new(&file.content)).collect();
    answer.sites.iter().map(|site| {
        let block = &answer.blocks[site.block];
        let file = &files[block.file];
        let target = block_unit(block, file);
        Row {
            file: file.path.strip_prefix(project).unwrap_or(&file.path).to_string_lossy().into_owned(),
            line: site.line,
            text: bound::line_text(lines[block.file].text(site.line)),
            name: target.name.clone(),
            kind: target.kind,
            target,
            file_index: block.file,
        }
    }).collect()
}

impl ReadDomain {
    pub(super) fn search(&mut self, request: SearchRequest) -> Value {
        let limit = match bound::limit(request.limit) { Ok(limit) => limit, Err(answer) => return answer };
        let case_insensitive = request.case_insensitive.unwrap_or(false);
        let matcher = match matcher(&request.pattern, case_insensitive) { Ok(matcher) => matcher, Err(answer) => return answer };
        let mut searcher = searcher();
        let resumes = Resumes::Search { pattern: request.pattern.clone(), scope: request.scope.clone(), case_insensitive };
        let resume = match self.resume(request.cursor.as_deref(), &resumes) { Ok(resume) => resume, Err(answer) => return answer };
        let candidates = match self.candidates(&request.scope) { Ok(paths) => paths, Err(answer) => return answer };
        let mut files = Vec::new();
        let mut skipped = source::Skipped::default();
        for candidate in candidates {
            let (path, revision, content) = match source::content(&self.project, &candidate) {
                Ok(content) => content,
                Err(crate::acquisition::Error::Crossing(crossing)) => { skipped.push(crossing); continue; }
                Err(_) => continue,
            };
            let lines = matching_lines(&mut searcher, &matcher, &content);
            if lines.is_empty() { continue; }
            files.push(FileHits { path, revision, content, lines });
        }
        // The walk is sorted, but these are the canonical paths, and a symlink
        // can resolve out of the walk's order. Paging compares sites by path.
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let answer = plan_answer(&files, AGGREGATE_PARSE_BUDGET, &mut outline::monotonic());
        self.render(resumes, &files, &answer, resume, &skipped, limit)
    }

    /// The files a scope names, in path order. Named scopes come from the
    /// binary's own retained state; the others from the walk.
    pub(super) fn candidates(&self, scope: &Scope) -> Result<Vec<PathBuf>, Value> {
        match scope {
            Scope::CurrentTaskLease { phase, occurrence, plan, task } => {
                super::scope::task_lease_files(self, phase.get(), occurrence, plan.get(), task)
            }
            _ => {
                let (root, glob) = selector(&self.project, scope)?;
                Ok(files(&root, &self.project, glob.as_ref()))
            }
        }
    }

    /// The site a cursor token resumes at, once it is checked against the
    /// request it was issued for. A token issued for another request would
    /// resume at a site that means nothing in this one.
    pub(super) fn resume(&self, cursor: Option<&str>, resumes: &Resumes) -> Result<Option<(PathBuf, usize)>, Value> {
        match cursor {
            None => Ok(None),
            Some(token) => match self.registry.get(token) {
                Some(Capability::Cursor { resumes: issued, file, line }) if issued == *resumes => Ok(Some((file, line))),
                Some(Capability::Cursor { .. }) => Err(refusal("cursor", "cursor-mismatch", "cursor was issued for a different request")),
                _ => Err(refusal("cursor", "location-not-issued", "cursor was not issued by this resident")),
            },
        }
    }

    /// Admit rows together with their file references and possible skip notes.
    fn render(&mut self, resumes: Resumes, files: &[FileHits], answer: &Answer, resume: Option<(PathBuf, usize)>, skipped: &source::Skipped, limit: usize) -> Value {
        let start = match &resume {
            None => 0,
            Some((path, line)) => answer.sites.iter()
                .position(|site| (files[site.file].path.as_path(), site.line) >= (path.as_path(), *line))
                .unwrap_or(answer.sites.len()),
        };
        let rows = rows(files, answer, &self.project);
        let mut wire: Vec<_> = rows[start..].iter().map(|row| {
            let mut value = serde_json::to_value(row).unwrap();
            value["location"] = json!(bound::longest_token("loc"));
            value
        }).collect();
        let mut envelope = source::with_skipped(json!({"status":"ok","kind":"search","bound":ANSWER_BOUND,"limit":limit,
            "incomplete":false,"cursor":bound::longest_token("cur"),"rows":[],"files":[],
            "notes":[STOPPED_NOTE, BOUNDED_NOTE, EXHAUSTED_NOTE],"matches":rows.len(),"served":limit}), skipped);
        let mut reserved_files = Vec::new();
        let mut page = bound::page(&[], bound::room(&envelope), limit);
        let mut end = 0;
        // Grow the prefix with its metadata reserved before admitting its rows.
        // Reserving each possible skip note (and even a skipped file's token)
        // is conservative and keeps a long run of oversized rows bounded too.
        for (index, row) in rows[start..].iter().enumerate() {
            if !reserved_files.contains(&row.file_index) {
                envelope["files"].as_array_mut().unwrap().push(json!({"file":row.file,"file_reference":bound::longest_token("file")}));
                reserved_files.push(row.file_index);
            }
            envelope["notes"].as_array_mut().unwrap().push(json!(format!("{}:{}: row exceeds the answer bound", row.file, row.line)));
            let room = bound::room(&envelope);
            if room == 0 { break; }
            let candidate = bound::page(&wire[..=index], room, limit);
            // Added metadata may push a previously admitted row out. Keep the
            // preceding page in that case and retry this row on the next page.
            if candidate.next.is_some() || !candidate.served.starts_with(&page.served) { break; }
            page = candidate;
            end = index + 1;
            if page.served.len() == limit { break; }
        }
        let next = rows.get(start + end).map(|row| {
            self.registry.cursor(resumes, files[row.file_index].path.clone(), row.line)
        });
        let mut emitted = Vec::new();
        let mut references = Vec::new();
        let mut referenced_files = Vec::new();
        for &index in &page.served {
            let row = &rows[start + index];
            let file = &files[row.file_index];
            let unit = &row.target;
            wire[index]["location"] = json!(self.registry.unit(file.path.clone(), file.revision.clone(), unit.clone(), unit.first_byte));
            emitted.push(wire[index].take());
            if !referenced_files.contains(&row.file_index) {
                references.push(json!({"file":row.file,"file_reference":self.registry.file(file.path.clone(), file.revision.clone())}));
                referenced_files.push(row.file_index);
            }
        }
        let mut notes: Vec<String> = Vec::new();
        if answer.stopped { notes.push(STOPPED_NOTE.into()); }
        if next.is_some() { notes.push(BOUNDED_NOTE.into()); }
        if resume.is_some() && start == rows.len() { notes.push(EXHAUSTED_NOTE.into()); }
        for &index in &page.passed {
            let row = &rows[start + index];
            notes.push(format!("{}:{}: row exceeds the answer bound", row.file, row.line));
        }
        source::with_skipped(json!({"status":"ok","kind":"search","bound":ANSWER_BOUND,"limit":limit,
            "incomplete":next.is_some(),"cursor":next,"rows":emitted,"files":references,"notes":notes,
            "matches":rows.len(),"served":page.served.len()}), skipped)
    }
}

#[cfg(test)]
mod row_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn a_crossing_is_named_in_an_incomplete_search() {
        let mut skipped = super::source::Skipped::default();
        skipped.push(crate::acquisition::Crossing { file: "src/large.rs".into(), size: 16_777_217, bound: 16_777_216 });
        let answer = super::source::with_skipped(serde_json::json!({"kind":"search","incomplete":false,
            "hits":[{"file":"src/small.rs","body":"match"}],"notes":[]}), &skipped);
        assert_eq!(answer["incomplete"], true);
        assert_eq!(answer["hits"], serde_json::json!([{"file":"src/small.rs","body":"match"}]));
        assert_eq!(answer["notes"], serde_json::json!(["src/large.rs: size 16777217 exceeds acquisition bound 16777216"]));
    }
    use super::*;

    fn unit(name: &str, first: usize, last: usize) -> Unit {
        Unit { name: name.into(), bare: name.into(), kind: "function", first_line: first, last_line: last, first_byte: 0, last_byte: 0 }
    }

    #[test]
    fn the_innermost_unit_encloses_and_a_same_span_tie_goes_to_the_later_one() {
        let units = vec![unit("impl Widget", 1, 10), unit("Widget::build", 2, 5), unit("same", 2, 5)];
        assert_eq!(enclosing(&units, 3), Some(2));
        assert_eq!(enclosing(&units, 8), Some(0));
        assert_eq!(enclosing(&units, 11), None);
    }

    fn hits(content: &str, lines: &[usize]) -> FileHits {
        FileHits { path: PathBuf::from("/x/a.rs"), revision: String::new(), content: content.to_owned(), lines: lines.to_vec() }
    }

    /// Two matches in one unit are one block; a match outside every unit is a
    /// window, and windows that abut merge.
    #[test]
    fn matches_in_one_unit_share_a_block_and_stray_matches_become_merged_windows() {
        let content: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let file = hits(&content, &[3, 4, 12, 15, 28]);
        let units = vec![unit("alpha", 2, 6)];
        let (blocks, of_line) = file_blocks(0, &file, &units);
        assert_eq!(blocks.len(), 3, "{}", blocks.iter().map(|b| format!("{}-{} ", b.first_line, b.last_line)).collect::<String>());
        assert_eq!((blocks[0].first_line, blocks[0].last_line, blocks[0].lines.clone()), (2, 6, vec![3, 4]));
        assert!(blocks[0].unit.is_some());
        assert_eq!((blocks[1].first_line, blocks[1].last_line, blocks[1].lines.clone()), (10, 17, vec![12, 15]));
        assert!(blocks[1].unit.is_none());
        assert_eq!((blocks[2].first_line, blocks[2].last_line), (26, 30));
        assert_eq!(of_line, vec![0, 0, 1, 1, 2]);
    }

    /// A unit block opens where its first hit is, which can be below a window
    /// already pushed; the answer is still in first-line order.
    #[test]
    fn blocks_come_out_in_first_line_order_with_their_hits_re_indexed() {
        let content: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let file = hits(&content, &[2, 15]);
        let units = vec![unit("late", 10, 20)];
        let (blocks, of_line) = file_blocks(0, &file, &units);
        assert_eq!((blocks[0].first_line, blocks[0].last_line), (1, 4));
        assert_eq!((blocks[1].first_line, blocks[1].last_line), (10, 20));
        assert_eq!(of_line, vec![0, 1]);
    }

    /// A spent aggregate budget costs unit resolution, never a hit.
    #[test]
    fn a_spent_aggregate_budget_still_places_every_match_in_a_window() {
        let file = hits("fn alpha() {\n    let needle = 1;\n    needle\n}\n", &[2, 3]);
        let resolved = plan_answer(std::slice::from_ref(&file), Duration::from_secs(5), &mut || Duration::ZERO);
        assert!(!resolved.stopped);
        assert_eq!(resolved.blocks.len(), 1);
        assert_eq!(resolved.blocks[0].unit.as_ref().map(|unit| unit.name.as_str()), Some("alpha"));
        let spent = plan_answer(std::slice::from_ref(&file), Duration::ZERO, &mut || Duration::ZERO);
        assert!(spent.stopped);
        assert_eq!(spent.sites.len(), 2);
        assert!(spent.blocks.iter().all(|block| block.unit.is_none()));
        assert_eq!(block_unit(&spent.blocks[0], &file).name, WINDOW_NAME);
    }

}
