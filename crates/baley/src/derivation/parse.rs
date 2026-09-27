use super::*;

// ECMAScript \s, rather than Rust's broader Unicode White_Space property.
fn space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn dot(c: char) -> bool {
    !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

#[derive(Default)]
struct Fence(Option<(u8, usize)>);

impl Fence {
    fn scan(&mut self, line: &str) -> bool {
        let indent = line.bytes().take_while(|&b| b == b' ').count();
        if indent > 3 {
            return self.0.is_some();
        }
        let rest = &line[indent..];
        let Some(ch @ (b'`' | b'~')) = rest.bytes().next() else {
            return self.0.is_some();
        };
        let len = rest.bytes().take_while(|&b| b == ch).count();
        let info = rest[len..].trim_start_matches(space);
        if len < 3 || !info.chars().all(dot) {
            return self.0.is_some();
        }
        match self.0 {
            None => self.0 = Some((ch, len)),
            Some((opening, length))
                if opening == ch && len >= length && info.trim_matches(space).is_empty() =>
            {
                self.0 = None
            }
            _ => {}
        }
        true
    }
}

impl PhaseId {
    /// Frozen String(Number(spelling)), including decimal/exponent thresholds.
    pub fn address(self) -> String {
        if self.0.is_infinite() {
            return "Infinity".into();
        }
        if self.0 == 0.0 {
            return "0".into();
        }
        let decimal = self.0.to_string();
        if (1e-6..1e21).contains(&self.0) {
            return decimal;
        }
        let (whole, fraction) = decimal.split_once('.').unwrap_or((&decimal, ""));
        let digits = format!("{whole}{fraction}");
        let first = digits.bytes().position(|b| b != b'0').unwrap();
        let significant = digits[first..].trim_end_matches('0');
        let exponent = whole.len() as isize - first as isize - 1;
        let tail = if significant.len() > 1 {
            format!(".{}", &significant[1..])
        } else {
            String::new()
        };
        format!("{}{tail}e{exponent:+}", &significant[..1])
    }
}

fn canonical(line: &str, source_line: usize, ordinal: usize) -> Option<RoadmapPhase> {
    let (checked, rest) = if let Some(rest) = line.strip_prefix("- [ ] **Phase ") {
        (false, rest)
    } else {
        (true, line.strip_prefix("- [x] **Phase ")?)
    };
    let (number, body) = rest.split_once(": ")?;
    let mut parts = number.split('.');
    let integer = parts.next()?;
    if integer.is_empty() || !integer.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(fraction) = parts.next()
        && (fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    for (end, _) in body
        .char_indices()
        .filter(|(i, _)| body[*i..].starts_with("**"))
    {
        let name = &body[..end];
        if name.is_empty() || !name.chars().all(dot) {
            continue;
        }
        let tail = &body[end + 2..];
        let description = if tail.is_empty() {
            ""
        } else if let Some(desc) = tail.trim_start_matches(space).strip_prefix('-') {
            let desc = desc.trim_start_matches(space);
            if !desc.chars().all(dot) {
                continue;
            }
            desc
        } else {
            continue;
        };
        let id = PhaseId(number.parse().ok()?);
        return Some(RoadmapPhase {
            id,
            name: name.into(),
            description: description.into(),
            checked,
            source_line,
            ordinal,
            relative_path: format!("phases/{}", id.address()).into(),
        });
    }
    None
}

fn phase_token(line: &str) -> bool {
    line.match_indices("Phase ").any(|(i, _)| {
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        if line[..i].chars().next_back().is_some_and(word) {
            return false;
        }
        let rest = &line[i + 6..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return false;
        }
        // The optional decimal can backtrack to the integer at its word boundary.
        !rest[digits..].chars().next().is_some_and(word)
    })
}

pub fn parse_roadmap(text: &str) -> Result<ParsedRoadmap, DerivationError> {
    let normalized = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n");
    let lines: Vec<_> = normalized.split('\n').collect();
    let mut fence = Fence::default();
    let mut start = None;
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate() {
        if fence.scan(line) {
            continue;
        }
        if start.is_none() {
            if line.trim_matches(space) == "## Phases" {
                start = Some(i);
            }
        } else if line.starts_with("## ") {
            end = i;
            break;
        }
    }
    let start = start.ok_or_else(|| DerivationError::InvalidRoadmap {
        detail: "no-section: no unfenced ## Phases heading".into(),
    })?;
    let mut phases = Vec::new();
    let mut fence = Fence::default();
    for (i, line) in lines.iter().enumerate().take(end).skip(start + 1) {
        if !fence.scan(line)
            && let Some(phase) = canonical(line, i + 1, phases.len())
        {
            phases.push(phase);
        }
    }
    if !phases.is_empty() {
        // The `## Phases` list order is the run order; the first declaration
        // that is not complete is the current phase, whatever its number.
        return Ok(ParsedRoadmap {
            cycle: Cycle::Live,
            phases,
        });
    }
    let mut fence = Fence::default();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if !fence.scan(line) && phase_token(line) {
            return Err(DerivationError::InvalidRoadmap {
                detail: format!(
                    "out-of-grammar at line {}: {}",
                    i + 1,
                    line.trim_matches(space)
                ),
            });
        }
    }
    Ok(ParsedRoadmap {
        cycle: Cycle::Closed,
        phases,
    })
}
