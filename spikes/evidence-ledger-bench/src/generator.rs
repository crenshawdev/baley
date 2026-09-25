//! Deterministic synthetic content and the workload built from the profile.

use crate::store::{Attachment, Command, NewEvent, prepare};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
pub struct Profile {
    pub compression_ratio_zstd3: HashMap<String, Ratio>,
    pub commands: Vec<ProfileCommand>,
}
#[derive(Deserialize)]
pub struct Ratio {
    pub ratio: f64,
}
#[derive(Deserialize, Clone)]
pub struct ProfileCommand {
    pub phase: i64,
    pub stream: String,
    pub events: Vec<ProfileEvent>,
}
#[derive(Deserialize, Clone)]
pub struct ProfileEvent {
    #[serde(rename = "type")]
    pub etype: String,
    pub inline: usize,
    pub attachments: Vec<ProfileAttachment>,
}
#[derive(Deserialize, Clone)]
pub struct ProfileAttachment {
    pub class: String,
    pub bytes: usize,
    pub content: u64,
}

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Text from a 50,000-word vocabulary with a Zipf distribution, as natural text
/// has, so the search index sees a realistic spread of terms. New lines are
/// mixed with repeats of earlier lines at a calibrated probability to reach a
/// target compression ratio.
pub struct Text {
    words: Vec<String>,
    cumulative: Vec<f64>,
}
impl Text {
    pub fn new() -> Text {
        let mut r = Rng(7);
        let n = 50_000;
        let words = (0..n).map(|_| (0..(2 + r.below(9))).map(|_| (b'a' + r.below(26) as u8) as char).collect()).collect();
        let mut cumulative = Vec::with_capacity(n);
        let mut total = 0.0;
        for i in 0..n {
            total += 1.0 / ((i + 1) as f64).powf(1.07);
            cumulative.push(total);
        }
        for c in &mut cumulative {
            *c /= total;
        }
        Text { words, cumulative }
    }
    fn word(&self, r: &mut Rng) -> &str {
        let u = r.unit();
        let i = self.cumulative.partition_point(|c| *c < u).min(self.words.len() - 1);
        &self.words[i]
    }
    pub fn make(&self, seed: u64, len: usize, repeat: f64) -> Vec<u8> {
        let mut r = Rng(seed);
        let mut out = Vec::with_capacity(len + 128);
        let mut lines: Vec<(usize, usize)> = vec![];
        while out.len() < len {
            if !lines.is_empty() && r.unit() < repeat {
                let (a, b) = lines[r.below(lines.len() as u64) as usize];
                let copy = out[a..b].to_vec();
                out.extend_from_slice(&copy);
            } else {
                let start = out.len();
                for _ in 0..(4 + r.below(12)) {
                    let w = self.word(&mut r).to_string();
                    out.extend_from_slice(w.as_bytes());
                    out.push(b' ');
                }
                out.push(b'\n');
                lines.push((start, out.len()));
            }
        }
        out.truncate(len);
        out
    }
    /// The repeat probability whose zstd level-3 ratio is nearest the target,
    /// measured on a sample of the given length.
    pub fn calibrate(&self, target: f64, sample: usize) -> (f64, f64) {
        let mut best = (0.0, f64::MAX, 0.0);
        for i in 0..=99 {
            let q = i as f64 / 100.0;
            let s = self.make(99 + i, sample, q);
            let ratio = s.len() as f64 / zstd::encode_all(&s[..], 3).unwrap().len() as f64;
            if (ratio - target).abs() < best.1 {
                best = (q, (ratio - target).abs(), ratio);
            }
        }
        (best.0, best.2)
    }
}

pub fn bucket(n: usize) -> &'static str {
    if n < 16_384 {
        "small"
    } else if n < 262_144 {
        "medium"
    } else {
        "large"
    }
}

pub fn class(c: &str) -> &'static str {
    match c {
        "output" => "output",
        "material" => "material",
        _ => "record",
    }
}

pub struct Generator {
    pub profile: Profile,
    pub text: Text,
    /// (class/bucket) -> (repeat probability, achieved ratio, target ratio)
    pub calibration: HashMap<String, (f64, f64, f64)>,
}

impl Generator {
    pub fn new(path: &Path) -> Result<Generator, Box<dyn std::error::Error + Send + Sync>> {
        let profile: Profile = serde_json::from_slice(&std::fs::read(path)?)?;
        let text = Text::new();
        let mut calibration = HashMap::new();
        for (k, r) in &profile.compression_ratio_zstd3 {
            let sample = match k.split('/').nth(1) {
                Some("small") => 8 * 1024,
                Some("medium") => 64 * 1024,
                _ => 1024 * 1024,
            };
            let (q, got) = text.calibrate(r.ratio, sample);
            calibration.insert(k.clone(), (q, got, r.ratio));
        }
        Ok(Generator { profile, text, calibration })
    }

    fn repeat_for(&self, class: &str, bytes: usize) -> f64 {
        let key = format!("{class}/{}", bucket(bytes));
        self.calibration
            .get(&key)
            .or_else(|| self.calibration.iter().find(|(k, _)| k.starts_with(class)).map(|(_, v)| v))
            .map(|v| v.0)
            .unwrap_or(0.5)
    }

    pub fn attachment(&self, seed: u64, class_name: &str, bytes: usize) -> Attachment {
        let c = class(class_name);
        let body = self.text.make(seed, bytes, self.repeat_for(c, bytes));
        prepare(c, &body, c == "record")
    }

    pub fn inline(&self, seed: u64, phase: i64, etype: &str, size: usize) -> Value {
        let text = String::from_utf8(self.text.make(seed, size.saturating_sub(160).max(16), 0.2)).unwrap();
        json!({"phase": phase, "facts": {"status": etype, "phase": phase}, "text": text})
    }

    /// The profile replayed for one project. Streams are the profile's family and
    /// phase split into records of up to ten commands, approximating one stream
    /// per dispatch, plan or review. Commands whose events have external effects
    /// use the claim protocol; one anchor push per 50 commands is added.
    pub fn commands(&self, project: &str, seed: u64) -> Vec<Command> {
        let mut out = vec![];
        let mut groups: HashMap<String, (u64, u64)> = HashMap::new();
        for (n, pc) in self.profile.commands.iter().enumerate() {
            let n = n as u64 + 1;
            let g = groups.entry(pc.stream.clone()).or_insert((0, 0));
            if g.1 == 10 {
                g.0 += 1;
                g.1 = 0;
            }
            g.1 += 1;
            let stream = format!("{}/{}", pc.stream, g.0);
            let kind = pc.stream.split('/').next().unwrap().to_string();
            let mut events = vec![];
            let mut external = false;
            for (i, pe) in pc.events.iter().enumerate() {
                let attachments = pe.attachments.iter().map(|a| self.attachment(seed.wrapping_mul(1_000_003) ^ a.content, &a.class, a.bytes)).collect();
                let t = pe.etype.as_str();
                external |= t.starts_with("suite.") || t == "verification.run" || matches!(t, "review.deliveries" | "review.host_launches" | "review.provider_payloads");
                events.push(NewEvent {
                    stream: stream.clone(),
                    etype: pe.etype.clone(),
                    phase: pc.phase,
                    payload: self.inline(seed ^ (n << 8) ^ i as u64, pc.phase, t, pe.inline),
                    attachments,
                    git: t.starts_with("task.") || t.starts_with("suite.") || t.starts_with("verification."),
                });
            }
            let authority = if kind == "dispatch" || kind == "admission" {
                Some(("plan.approved".to_string(), format!("plan/{}/", pc.phase)))
            } else if pc.events.iter().any(|e| e.etype == "verification.completion") {
                Some(("verification.run".to_string(), "verification/".to_string()))
            } else {
                None
            };
            out.push(Command { project: project.into(), kind, request_id: format!("{seed}-{n}"), phase: pc.phase, external, authority, events });
            if pc.events.iter().any(|e| e.etype.starts_with("task.")) && n % 3 == 0 {
                out.push(guard_command(project, &format!("{seed}-g{n}"), pc.phase));
            }
            if n % 50 == 0 {
                out.push(anchor_command(project, &format!("{seed}-a{n}"), pc.phase));
            }
        }
        out
    }
}

pub fn guard_command(project: &str, request: &str, phase: i64) -> Command {
    Command {
        project: project.into(),
        kind: "guard".into(),
        request_id: request.into(),
        phase,
        external: false,
        authority: None,
        events: vec![NewEvent {
            stream: "guard".into(),
            etype: "guard.allowed".into(),
            phase,
            payload: json!({"phase": phase, "facts": {"verb": "commit", "branch": "phase-work"}, "tool": "Bash"}),
            attachments: vec![],
            git: false,
        }],
    }
}

pub fn anchor_command(project: &str, request: &str, phase: i64) -> Command {
    Command {
        project: project.into(),
        kind: "anchor".into(),
        request_id: request.into(),
        phase,
        external: true,
        authority: None,
        events: vec![NewEvent {
            stream: "project".into(),
            etype: "anchor.pushed".into(),
            phase,
            payload: json!({"phase": phase, "facts": {"tag": format!("baley-anchor/{project}/{request}")}}),
            attachments: vec![],
            git: false,
        }],
    }
}
