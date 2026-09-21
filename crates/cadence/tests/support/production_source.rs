use std::fs;
use std::path::Path;

/// Every matching line in production source, as
/// `path:line`. Test modules and test files are skipped, the way the phase 7
/// lease test skips them when it counts `fn covers(`.
pub fn production_sites(root: &Path, matches: &impl Fn(&str) -> bool) -> Vec<String> {
    let mut sites = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sites.extend(production_sites(&path, matches));
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        if !name.ends_with(".rs") || name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            // An inline test module ends the production part of a file. A
            // `#[cfg(test)] mod name;` declaration does not: the file goes on.
            let opens_test_module = line.trim() == "#[cfg(test)]"
                && lines.get(index + 1).is_some_and(|next| next.trim().starts_with("mod ") && next.trim().ends_with('{'));
            if opens_test_module {
                break;
            }
            if line.trim().starts_with("//") {
                continue;
            }
            if matches(line) {
                sites.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    sites
}
