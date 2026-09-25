//! The UTC instant form shared by the store and its callers.

use std::fmt;

/// A UTC instant with nanosecond precision, ordered on a fixed 86,400-second day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcInstant {
    seconds: i64,
    nanos: u32,
}

/// A malformed UTC instant or an instant outside years 0000 through 9999.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeError;

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid UTC instant")
    }
}

impl std::error::Error for TimeError {}

impl UtcInstant {
    /// Parses `YYYY-MM-DDTHH:MM:SS[.f{1,9}]Z`, without offsets or leap seconds.
    pub fn parse(value: &str) -> Result<Self, TimeError> {
        let b = value.as_bytes();
        if b.len() < 20
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || *b.last().ok_or(TimeError)? != b'Z'
        {
            return Err(TimeError);
        }
        let year = digits(&b[0..4])?;
        let month = digits(&b[5..7])?;
        let day = digits(&b[8..10])?;
        let hour = digits(&b[11..13])?;
        let minute = digits(&b[14..16])?;
        let second = digits(&b[17..19])?;
        if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
            return Err(TimeError);
        }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > days[(month - 1) as usize] {
            return Err(TimeError);
        }
        let nanos = match &b[19..b.len() - 1] {
            [] => 0,
            [b'.', fraction @ ..] if (1..=9).contains(&fraction.len()) => {
                digits(fraction)? * 10u32.pow(9 - fraction.len() as u32)
            }
            _ => return Err(TimeError),
        };
        let seconds = civil_days(year as i64, month as i64, day as i64) * 86_400
            + (hour * 3600 + minute * 60 + second) as i64;
        Ok(Self { seconds, nanos })
    }

    /// Adds whole seconds, refusing an instant after the end of year 9999.
    pub fn plus_seconds(self, seconds: u64) -> Result<Self, TimeError> {
        let seconds = self
            .seconds
            .checked_add(i64::try_from(seconds).map_err(|_| TimeError)?)
            .ok_or(TimeError)?;
        let last = civil_days(9999, 12, 31) * 86_400 + 86_399;
        if seconds > last {
            return Err(TimeError);
        }
        Ok(Self {
            seconds,
            nanos: self.nanos,
        })
    }
}

fn digits(bytes: &[u8]) -> Result<u32, TimeError> {
    bytes.iter().try_fold(0u32, |n, b| {
        if b.is_ascii_digit() {
            Ok(n * 10 + u32::from(b - b'0'))
        } else {
            Err(TimeError)
        }
    })
}

// Gregorian days from civil, with floor division for year zero's January.
fn civil_days(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe
}

#[cfg(test)]
mod tests {
    use super::*;

    // Catches a deadline wrapping past the last representable UTC instant.
    #[test]
    fn adding_past_the_last_instant_is_an_error() {
        let last = UtcInstant::parse("9999-12-31T23:59:59Z").expect("valid");
        assert_eq!(last.plus_seconds(1), Err(TimeError));
    }
}
