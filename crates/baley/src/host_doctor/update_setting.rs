//! The effective global `updates.auto`, judged the one way install and the
//! doctor both read it (design 0012 sections 5 and 6).
//!
//! Install records the value it judged; the doctor shows the value judged
//! now, never the record's, since `baley config set` can change it after
//! install ran.

use std::path::PathBuf;

use baley_core::policy::{
    FileLayer, Layer, Schema, SettingsFile, Unavailable, Value, merge, parse_layer,
};

/// The effective `updates.auto`, or why the global settings give none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    /// The value in effect.
    Effective {
        /// The value.
        auto: bool,
        /// The global file that sets it, or none when it is the schema's
        /// default.
        file: Option<PathBuf>,
    },
    /// The global settings cannot be read or parsed, so the server cannot
    /// read the setting either.
    Unreadable {
        /// The file at fault.
        path: PathBuf,
        /// The refusal as worded for the owner.
        text: String,
    },
}

impl Judged {
    /// The value install records: the effective value, or none when the
    /// global settings could not be read.
    pub fn auto(&self) -> Option<bool> {
        match self {
            Judged::Effective { auto, .. } => Some(*auto),
            Judged::Unreadable { .. } => None,
        }
    }
}

/// Judges what reading the global settings file found: parsed as the global
/// layer over the standard schema and merged with no project layer and no
/// host.
pub fn judge(read: Result<Option<SettingsFile>, Unavailable>) -> Judged {
    let schema = Schema::standard();
    let parsed = read.and_then(|file| {
        file.as_ref()
            .map(|file| parse_layer(file, FileLayer::Global, schema))
            .transpose()
    });
    let global = match parsed {
        Ok(global) => global,
        Err(refusal) => {
            return Judged::Unreadable {
                path: refusal.path.clone(),
                text: refusal.to_string(),
            };
        }
    };
    let policy = merge(schema, None, global.as_ref(), None);
    let effective = &policy.settings["updates.auto"];
    match effective.value {
        Some(Value::Bool(auto)) => Judged::Effective {
            auto,
            file: match effective.source.layer {
                Layer::Default => None,
                _ => effective.source.file.as_ref().map(|file| file.path.clone()),
            },
        },
        // The schema gives updates.auto a boolean default, so this is a
        // schema fault rather than an owner's file.
        _ => Judged::Unreadable {
            path: policy
                .global
                .map(|file| file.path)
                .unwrap_or_else(|| PathBuf::from("the global settings")),
            text: "updates.auto holds no boolean".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const GLOBAL: &str = "/home/o/.config/crenshawdev/baley/config.toml";

    fn file(text: &str) -> Result<Option<SettingsFile>, Unavailable> {
        Ok(Some(crate::settings::file(
            Path::new(GLOBAL),
            text.as_bytes().to_vec(),
        )))
    }

    #[test]
    fn an_invalid_global_file_read_as_updates_off_is_caught() {
        match judge(file("[updates]\nauto = \"yes\"\n")) {
            Judged::Unreadable { path, text } => {
                assert_eq!(path, Path::new(GLOBAL));
                assert!(text.contains("updates.auto"), "{text}");
            }
            other => panic!("{other:?}"),
        }
        let unclosed = judge(file("[updates\nauto = true\n"));
        assert!(
            matches!(unclosed, Judged::Unreadable { .. }),
            "{unclosed:?}"
        );
        assert_eq!(unclosed.auto(), None);

        assert_eq!(
            judge(file("[updates]\nauto = true\n")),
            Judged::Effective {
                auto: true,
                file: Some(GLOBAL.into()),
            }
        );
        for absent in [file(""), Ok(None)] {
            assert_eq!(
                judge(absent),
                Judged::Effective {
                    auto: false,
                    file: None,
                }
            );
        }
    }
}
