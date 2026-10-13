//! Reads what the fetch command pipes into `baley models import`: the body,
//! a newline, then the `baley-fetch` trailer line the shell wrote after
//! curl exited (design 0003 section 5).

use baley_core::catalog::detection::{Category, Observation, ObservedResponse};

use super::MAX_BODY_BYTES;

/// The observation one fetch's bytes show, or `incomplete`.
///
/// The input must end in a newline, and the line before it is the trailer:
/// `baley-fetch transport-failed`, or `baley-fetch status=<code>
/// complete=<true|false>` with a three-digit code from 100 to 599. The body
/// is every byte before the newline that precedes the trailer, unchanged.
/// A body over the 4 MiB bound is `incomplete` whatever its trailer says.
/// The grammar is strict so a damaged fetch never reads as a valid empty
/// list. Anything else is `incomplete`.
pub fn parse_fetched(input: &[u8]) -> Result<Observation, Category> {
    let incomplete = Err(Category::Incomplete);
    let Some(rest) = input.strip_suffix(b"\n") else {
        return incomplete;
    };
    let Some(separator) = rest.iter().rposition(|byte| *byte == b'\n') else {
        return incomplete;
    };
    let (body, trailer) = (&rest[..separator], &rest[separator + 1..]);
    if body.len() > MAX_BODY_BYTES {
        return incomplete;
    }
    if trailer == b"baley-fetch transport-failed" {
        // curl may have written part of the body before it failed.
        return Ok(Observation {
            response: None,
            transport_failed: true,
        });
    }
    let Some(fields) = trailer.strip_prefix(b"baley-fetch status=") else {
        return incomplete;
    };
    let (code, complete) = fields.split_at_checked(3).unwrap_or((b"", b""));
    let cut_short = match complete {
        b" complete=true" => false,
        b" complete=false" => true,
        _ => return incomplete,
    };
    // Digit by digit, since integer parsing takes a sign or a leading zero.
    if !code.iter().all(u8::is_ascii_digit) || code.len() != 3 {
        return incomplete;
    }
    let status = code
        .iter()
        .fold(0u16, |status, digit| status * 10 + u16::from(digit - b'0'));
    if !(100..=599).contains(&status) {
        return incomplete;
    }
    Ok(Observation {
        response: Some(ObservedResponse {
            status,
            body: body.to_vec(),
            cut_short,
        }),
        transport_failed: false,
    })
}

#[cfg(test)]
mod tests {
    use baley_core::catalog::detection::ObservedResponse;

    use super::*;

    const BOUND: usize = 4_194_304;

    fn joined(body: &[u8], trailer: &str) -> Vec<u8> {
        [body, trailer.as_bytes()].concat()
    }

    fn response(observation: Observation) -> ObservedResponse {
        assert!(!observation.transport_failed);
        observation.response.expect("a response")
    }

    #[test]
    fn a_complete_trailer_gives_the_status_and_the_body_byte_for_byte() {
        let input = joined(
            b"{\"data\":[]}\n",
            "\nbaley-fetch status=200 complete=true\n",
        );
        let observation = parse_fetched(&input).unwrap();
        assert!(!observation.transport_failed);
        let response = response(observation);
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"data\":[]}\n");
        assert!(!response.cut_short);
    }

    #[test]
    fn complete_false_marks_the_body_cut_short() {
        let input = joined(b"[1,2]", "\nbaley-fetch status=200 complete=false\n");
        let response = response(parse_fetched(&input).unwrap());
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"[1,2]");
        assert!(response.cut_short);
    }

    #[test]
    fn transport_failed_gives_a_transport_failure_and_no_response() {
        let observation = parse_fetched(b"\nbaley-fetch transport-failed\n").unwrap();
        assert!(observation.transport_failed);
        assert_eq!(observation.response, None);
    }

    #[test]
    fn transport_failed_after_a_partial_body_is_still_a_transport_failure() {
        let input = joined(b"{\"object\":\"li", "\nbaley-fetch transport-failed\n");
        let observation = parse_fetched(&input).unwrap();
        assert!(observation.transport_failed);
        assert_eq!(observation.response, None);
    }

    #[test]
    fn a_non_2xx_status_passes_through_for_classification() {
        let input = joined(
            b"{\"error\":{}}",
            "\nbaley-fetch status=401 complete=true\n",
        );
        let response = response(parse_fetched(&input).unwrap());
        assert_eq!(response.status, 401);
        assert!(!response.cut_short);
    }

    #[test]
    fn an_input_without_a_trailer_is_incomplete() {
        for input in [&b"{\"object\":\"list\",\"data\":[]}\n"[..], b""] {
            assert_eq!(parse_fetched(input), Err(Category::Incomplete), "{input:?}");
        }
    }

    #[test]
    fn a_malformed_trailer_is_incomplete() {
        let trailers = [
            "\nbaley-fetch status=200\n",
            "\nbaley-fetch status=200 complete=yes\n",
            "\nbaley-fetch status=20x complete=true\n",
            "\nbaley-fetch status=000 complete=true\n",
            "\nbaley-fetch status=99 complete=true\n",
            "\nbaley-fetch status=2000 complete=true\n",
            "\nbaley-fetch status=+200 complete=true\n",
            "\nbaley-fetch status=0200 complete=true\n",
            "\nbaley-fetch  status=200 complete=true\n",
            "\nbaley-fetch status=200 complete=true",
            "\nBALEY-FETCH transport-failed\n",
        ];
        for trailer in trailers {
            let input = joined(b"[]", trailer);
            assert_eq!(
                parse_fetched(&input),
                Err(Category::Incomplete),
                "{trailer:?}"
            );
        }
        assert_eq!(
            parse_fetched(b"baley-fetch status=200 complete=true\n"),
            Err(Category::Incomplete)
        );
    }

    #[test]
    fn a_trailer_that_is_not_the_last_line_is_incomplete() {
        let input = b"{\"data\":[]}\nbaley-fetch status=200 complete=true\n{\"data\":[]}\n";
        assert_eq!(parse_fetched(input), Err(Category::Incomplete));
    }

    #[test]
    fn a_body_one_byte_over_four_mib_is_incomplete_and_one_at_the_bound_is_kept() {
        let complete = "\nbaley-fetch status=200 complete=true\n";
        let at_bound = joined(&vec![b'a'; BOUND], complete);
        let response = response(parse_fetched(&at_bound).unwrap());
        assert_eq!(response.body.len(), BOUND);

        let over = joined(&vec![b'a'; BOUND + 1], complete);
        assert_eq!(parse_fetched(&over), Err(Category::Incomplete));

        let over_failed = joined(&vec![b'a'; BOUND + 1], "\nbaley-fetch transport-failed\n");
        assert_eq!(parse_fetched(&over_failed), Err(Category::Incomplete));
    }
}
