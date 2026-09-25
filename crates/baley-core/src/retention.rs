//! Policy eligibility from supplied closure facts and time.

use baley_store::{PayloadRef, RetentionClass, TimeError, UtcInstant, kept_ranges};

/// The closing fact supplied by the caller, never gathered here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closure {
    /// The reference's closing event has not been recorded.
    Unclosed,
    /// The milestone closing time for an output.
    MilestoneClosed { at: String },
    /// The review closing time for material.
    ReviewClosed { at: String },
}

/// What retention permits for one reference now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Keep the body whole.
    Keep,
    /// Replace this output reference with an excerpt.
    Reduce,
    /// Release this material reference and its body if unshared.
    Purge,
}

/// An invalid supplied time or a closure that does not apply to the class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionError {
    /// A supplied time is malformed or its deadline is out of range.
    Time(TimeError),
    /// This class cannot use the supplied closing fact.
    WrongClosure { class: RetentionClass },
}

/// Ninety fixed 86,400-second days.
pub const MATERIAL_SECONDS: u64 = 90 * 86_400;

/// Judges a reference using its class and length, closure, and supplied UTC time.
pub fn eligibility(
    reference: &PayloadRef,
    closure: &Closure,
    now: &str,
) -> Result<Retention, RetentionError> {
    let now = UtcInstant::parse(now).map_err(RetentionError::Time)?;
    match (reference.class, closure) {
        (RetentionClass::Record, _) => Ok(Retention::Keep),
        (_, Closure::Unclosed) => Ok(Retention::Keep),
        (RetentionClass::Output, Closure::MilestoneClosed { at }) => {
            let at = UtcInstant::parse(at).map_err(RetentionError::Time)?;
            Ok(if at <= now && kept_ranges(reference.bytes).is_some() {
                Retention::Reduce
            } else {
                Retention::Keep
            })
        }
        (RetentionClass::Material, Closure::ReviewClosed { at }) => {
            let at = UtcInstant::parse(at).map_err(RetentionError::Time)?;
            let deadline = at
                .plus_seconds(MATERIAL_SECONDS)
                .map_err(RetentionError::Time)?;
            Ok(if now >= deadline {
                Retention::Purge
            } else {
                Retention::Keep
            })
        }
        (class, _) => Err(RetentionError::WrongClosure { class }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baley_store::Hash;

    const NOW: &str = "2026-09-25T18:00:00Z";

    fn reference(class: RetentionClass, bytes: u64) -> PayloadRef {
        PayloadRef {
            hash: Hash([0; 32]),
            bytes,
            class,
        }
    }

    // Catches a permanent record reduced or purged by a closing fact.
    #[test]
    fn record_is_never_eligible() {
        assert_eq!(
            eligibility(
                &reference(RetentionClass::Record, 300_000),
                &Closure::ReviewClosed {
                    at: "2020-01-01T00:00:00Z".into()
                },
                NOW
            ),
            Ok(Retention::Keep)
        );
    }

    // Catches treating a future milestone as closed or any closure as sufficient.
    #[test]
    fn output_reduces_only_once_its_milestone_closed() {
        let output = reference(RetentionClass::Output, 300_000);
        assert_eq!(
            eligibility(&output, &Closure::Unclosed, NOW),
            Ok(Retention::Keep)
        );
        assert_eq!(
            eligibility(&output, &Closure::MilestoneClosed { at: NOW.into() }, NOW),
            Ok(Retention::Reduce)
        );
        assert_eq!(
            eligibility(
                &output,
                &Closure::MilestoneClosed {
                    at: "2026-09-25T18:00:01Z".into()
                },
                NOW
            ),
            Ok(Retention::Keep)
        );
    }

    // Catches wrong month arithmetic, a day off, or an exclusive deadline.
    #[test]
    fn material_purges_at_exactly_ninety_days() {
        let material = reference(RetentionClass::Material, 100);
        let closed = Closure::ReviewClosed {
            at: "2026-06-27T18:00:00Z".into(),
        };
        assert_eq!(eligibility(&material, &closed, NOW), Ok(Retention::Purge));
        assert_eq!(
            eligibility(&material, &closed, "2026-09-25T17:59:59Z"),
            Ok(Retention::Keep)
        );
    }

    // Catches a missing closure interpreted as a completed one.
    #[test]
    fn an_unclosed_reference_is_kept() {
        for class in [RetentionClass::Output, RetentionClass::Material] {
            assert_eq!(
                eligibility(&reference(class, 300_000), &Closure::Unclosed, NOW),
                Ok(Retention::Keep)
            );
        }
    }

    // Catches reducing an output whose excerpt would be the entire body.
    #[test]
    fn a_small_output_is_kept_whole() {
        assert_eq!(
            eligibility(
                &reference(RetentionClass::Output, 131_072),
                &Closure::MilestoneClosed { at: NOW.into() },
                NOW
            ),
            Ok(Retention::Keep)
        );
    }

    // Catches using a milestone close as a review close for material.
    #[test]
    fn a_closure_of_the_wrong_kind_is_an_error() {
        assert_eq!(
            eligibility(
                &reference(RetentionClass::Material, 10),
                &Closure::MilestoneClosed { at: NOW.into() },
                NOW
            ),
            Err(RetentionError::WrongClosure {
                class: RetentionClass::Material
            })
        );
    }

    // Catches treating an invalid current time as a default instant.
    #[test]
    fn a_malformed_time_is_an_error() {
        assert_eq!(
            eligibility(
                &reference(RetentionClass::Record, 10),
                &Closure::Unclosed,
                "2026-09-25 18:00:00"
            ),
            Err(RetentionError::Time(TimeError))
        );
    }
}
