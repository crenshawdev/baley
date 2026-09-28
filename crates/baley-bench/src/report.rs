//! Aggregation keeps a bad run and a long batch visible.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A row's one-run measurement and any longest individual wait.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Figure {
    /// One-run p99, maximum batch, total, or size, as the row specifies.
    pub value: Option<f64>,
    /// Longest individual queue wait.
    pub longest: Option<f64>,
}
/// Measured rows keyed by their published names.
pub type Run = BTreeMap<String, Figure>;
/// A recorded batch hold and whether its following cursor probe was refused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pause {
    /// Batch hold in milliseconds.
    pub ms: f64,
    /// The cursor has crossed a generation flip.
    pub cursor_refused: bool,
}
/// Nearest-rank percentile, with no interpolation.
pub fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1)]
}
/// The first refused cursor identifies the flip's preceding batch.
pub fn flip(pauses: &[Pause]) -> Option<f64> {
    pauses.iter().find(|p| p.cursor_refused).map(|p| p.ms)
}
/// The longest hold, including a single outlier.
pub fn longest_batch(pauses: &[Pause]) -> f64 {
    pauses.iter().map(|p| p.ms).fold(0.0, f64::max)
}
/// Median and worst run, preserving one-run maxima for batch rows.
pub fn aggregate(values: &[f64]) -> (f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.is_empty() {
        return (0.0, 0.0);
    }
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    };
    (median, *sorted.last().unwrap())
}
/// Measured rows with their units and design 0001's budgets.
pub const ROWS: &[(&str, &str, Option<f64>)] = &[
    ("Open with checks (slice 1)", "ms", Some(10.0)),
    ("Commit a command", "ms", Some(20.0)),
    ("Commit a command of 10 events", "ms", Some(20.0)),
    ("Get one view document by key and parse it", "ms", Some(2.0)),
    (
        "Guard hook, store work only, in a new process",
        "ms",
        Some(25.0),
    ),
    ("Wait for the write lock, 8 sessions", "ms", Some(250.0)),
    ("Rebuild every view while writers run", "s", Some(30.0)),
    ("Longest rebuild batch", "ms", Some(100.0)),
    ("Rebuild final flip", "ms", Some(50.0)),
    ("Size", "MB", Some(50.0)),
    ("Purge including scrub", "ms", None),
    ("Standalone scrub", "ms", None),
    ("Verify reference project", "ms", None),
];
/// Markdown for the owner's measurement record.
pub fn markdown(runs: &[Run]) -> String {
    let mut lines = vec![
        "| Operation | Budget | Median [worst run] | Verdict |".into(),
        "|---|---|---|---|".into(),
    ];
    for (name, unit, budget) in ROWS {
        let values: Vec<_> = runs.iter().filter_map(|r| r.get(*name)?.value).collect();
        let budget_text = budget.map_or("none".into(), |n| format!("{n} {unit}"));
        if values.len() != runs.len() {
            lines.push(format!(
                "| {name} | {budget_text} | not measured | not measured |"
            ));
            continue;
        }
        let (median, worst) = aggregate(&values);
        let longest = runs
            .iter()
            .filter_map(|r| r.get(*name)?.longest)
            .reduce(f64::max)
            .map_or(String::new(), |n| format!(", longest {n:.3} {unit}"));
        let verdict = budget.map_or("no budget", |b| if worst <= b { "within" } else { "over" });
        lines.push(format!(
            "| {name} | {budget_text} | {median:.3} [{worst:.3}] {unit}{longest} | {verdict} |"
        ));
    }
    for (name, budget) in [
        ("Server start with quick_check (Build 9)", "2 s"),
        ("Search (Build 8)", "measured in Build 8"),
        (
            "Server resident memory (Build 9)",
            "does not grow with the store",
        ),
        (
            "Ownership, mode, link and filesystem open checks (slice 2)",
            "10 ms including open",
        ),
    ] {
        lines.push(format!(
            "| {name} | {budget} | measured later | measured later |"
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_rank_does_not_select_the_hundredth_sample() {
        assert_eq!(
            percentile(&(1..=100).map(f64::from).collect::<Vec<_>>(), 0.99),
            99.0
        );
    }
    #[test]
    fn flip_is_the_first_refused_cursor_pause() {
        assert_eq!(
            flip(&[
                Pause {
                    ms: 2.0,
                    cursor_refused: false
                },
                Pause {
                    ms: 7.0,
                    cursor_refused: true
                },
                Pause {
                    ms: 4.0,
                    cursor_refused: true
                }
            ]),
            Some(7.0)
        );
    }
    #[test]
    fn longest_batch_aggregation_preserves_an_outlier() {
        let mut pauses: Vec<_> = (0..100)
            .map(|_| Pause {
                ms: 1.0,
                cursor_refused: false,
            })
            .collect();
        pauses.push(Pause {
            ms: 200.0,
            cursor_refused: false,
        });
        assert_eq!(aggregate(&[longest_batch(&pauses), 5.0, 6.0]), (6.0, 200.0));
    }
}
