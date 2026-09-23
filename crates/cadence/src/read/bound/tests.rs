use super::*;
use serde_json::json;

#[test]
fn answers_stay_under_the_serialized_bound() {
    let text = format!("{}{}{}", "\"".repeat(50), "\\".repeat(50), "\u{0001}".repeat(100));
    let rows = vec![json!({"text": text}); 100];
    assert_eq!(fit(65_534, &rows), 80);
    assert_eq!(fit(811, &rows[..1]), 0);
    assert_eq!(fit(812, &rows[..1]), 1);
}

#[test]
fn room_charges_the_envelope_with_tokens_at_their_longest() {
    let token = longest_token("cur");
    assert_eq!(token, "cur-0000000000000000-18446744073709551615");
    assert_eq!(token.len(), 41);
    assert_eq!(room(&json!({"cursor": token, "rows": []})), 65_472);
}

#[test]
fn a_page_stops_at_the_limit_and_names_the_next_row() {
    let rows = [json!("a"), json!("b"), json!("c")];
    let page = page(&rows, 65_534, 2);
    assert_eq!(page.served, [0, 1]);
    assert!(page.passed.is_empty());
    assert_eq!(page.next, Some(2));
}

#[test]
fn a_row_no_page_can_hold_is_passed_and_named() {
    let rows = [json!("x".repeat(1_000)), json!("a")];
    let page = page(&rows, 1_000, 50);
    assert_eq!(page.passed, [0]);
    assert_eq!(page.served, [1]);
    assert_eq!(page.next, None);
}

#[test]
fn a_limit_over_the_maximum_is_refused_naming_it() {
    assert_eq!(limit(None).unwrap(), 50);
    assert_eq!(limit(NonZeroU32::new(200)).unwrap(), 200);
    let refusal = limit(NonZeroU32::new(201)).unwrap_err();
    assert_eq!(refusal["code"], "invalid-limit");
    assert_eq!(refusal["slot"], "limit");
    assert!(refusal["reason"].as_str().unwrap().contains("200"));
}

#[test]
fn a_line_over_200_characters_is_cut_with_the_marker() {
    assert_eq!(line_text(&"x".repeat(200)), "x".repeat(200));
    assert_eq!(line_text(&format!("needle{}", "x".repeat(294))), format!("needle{}…", "x".repeat(194)));
    assert_eq!(line_text(&"é".repeat(201)), format!("{}…", "é".repeat(200)));
}
