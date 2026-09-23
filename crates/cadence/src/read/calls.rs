use super::outline::{self, Grammar};
use std::{path::Path, time::Duration};

fn call_grammar(path: &Path) -> Option<Grammar> {
    match outline::grammar_for_path(path) {
        Some(grammar @ (Grammar::Rust | Grammar::JavaScript | Grammar::Python | Grammar::C)) => Some(grammar),
        _ => None,
    }
}

fn sites(_content: &str, _grammar: Grammar, _name: &str, _now: &mut dyn FnMut() -> Duration) -> Option<Vec<usize>> {
    Some(Vec::new())
}

#[cfg(test)]
mod tests;
