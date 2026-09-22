//! Reading Git's branch listing, and choosing the branch work is measured
//! against.
//!
//! `observe` runs the commands; these two take what it read. Every check here
//! is bytes or values in, values out, so none of them starts Git or fakes it.
use super::branch::{Policy, choose_base, parse_branches};
use std::collections::BTreeMap;

const MAIN: &str = "1111111111111111111111111111111111111111";
const WORK: &str = "2222222222222222222222222222222222222222";

fn policy(base: Option<&str>, protected: &[&str]) -> Policy {
    Policy {
        protected: protected.iter().map(|name| (*name).to_owned()).collect(),
        on_protected: "refuse".into(),
        base: base.map(str::to_owned),
        integration: "main".into(),
        auto_branch: "ask".into(),
    }
}

fn branches(names: &[(&str, &str)]) -> BTreeMap<String, String> {
    names.iter().map(|(name, sha)| ((*name).to_owned(), (*sha).to_owned())).collect()
}

#[test]
fn each_ref_reads_as_a_branch_name_without_its_prefix() {
    assert_eq!(
        parse_branches(&format!("refs/heads/main {MAIN}\nrefs/heads/cadence/work {WORK}\n")).unwrap(),
        branches(&[("main", MAIN), ("cadence/work", WORK)])
    );
}

#[test]
fn no_refs_is_no_branches() {
    assert!(parse_branches("").unwrap().is_empty());
}

// A ref outside refs/heads keeps its full name rather than being mistaken for
// a branch of the same short name.
#[test]
fn a_ref_outside_refs_heads_keeps_its_whole_name() {
    assert_eq!(
        parse_branches(&format!("refs/remotes/origin/main {MAIN}\n")).unwrap(),
        branches(&[("refs/remotes/origin/main", MAIN)])
    );
}

#[test]
fn a_line_without_a_commit_is_refused() {
    assert!(parse_branches("refs/heads/main\n").is_err());
}

// The owner named the base, so a policy that names one either gets it or gets
// nothing. Falling back would measure the work against a branch they did not
// choose.
#[test]
fn a_named_base_is_used_when_it_exists_and_nothing_is_used_when_it_does_not() {
    let branches = branches(&[("main", MAIN), ("develop", WORK)]);
    assert_eq!(choose_base(&policy(Some("develop"), &["main"]), &branches), Some("develop".to_owned()));
    assert_eq!(choose_base(&policy(Some("release"), &["main"]), &branches), None);
}

#[test]
fn with_no_named_base_the_first_protected_branch_that_exists_is_used() {
    let branches = branches(&[("main", MAIN)]);
    assert_eq!(choose_base(&policy(None, &["trunk", "main", "master"]), &branches), Some("main".to_owned()));
}

// Protected order is the owner's preference order, not the repository's.
#[test]
fn the_protected_order_decides_which_of_several_is_chosen() {
    let branches = branches(&[("main", MAIN), ("master", WORK)]);
    assert_eq!(choose_base(&policy(None, &["master", "main"]), &branches), Some("master".to_owned()));
    assert_eq!(choose_base(&policy(None, &["main", "master"]), &branches), Some("main".to_owned()));
}

#[test]
fn no_protected_branch_exists_and_none_is_named_leaves_no_base() {
    assert_eq!(choose_base(&policy(None, &["main"]), &branches(&[("cadence/work", WORK)])), None);
    assert_eq!(choose_base(&policy(None, &[]), &branches(&[("main", MAIN)])), None);
}
