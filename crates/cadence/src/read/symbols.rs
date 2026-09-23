use super::model::Unit;
use std::{path::PathBuf, time::Duration};

struct Scan {
    rows: Vec<(usize, usize, Unit)>,
    unreached: Vec<usize>,
}

fn scan(
    _files: &[(PathBuf, String)],
    _name: &str,
    _case_insensitive: bool,
    _start: (usize, usize),
    _want: usize,
    _aggregate: Duration,
    _now: &mut dyn FnMut() -> Duration,
) -> Scan {
    Scan { rows: Vec::new(), unreached: Vec::new() }
}

#[cfg(test)]
mod tests;
