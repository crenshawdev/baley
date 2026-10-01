//! A settings file rendered whole from its base and the pairs a set changes
//! (design 0003 section 5, ADR 0027).

use toml::{Table, Value as TomlValue};

use super::set::TypedPair;
use crate::policy::parse::document;
use crate::policy::{
    Fault, FileLayer, ParsedLayer, Schema, SettingsFile, Unavailable, Value, line_and_column,
    parse_layer,
};

/// The complete bytes of the settings file `base` becomes when `pairs` are
/// written into it, each at its dotted path, under `host.<name>` when the
/// pair has a host.
///
/// The base is judged first with [`parse_layer`] under the same schema and
/// layer, and one it refuses is refused with that fault, never overwritten
/// (D-10). A base that passes holds a table or nothing at every prefix of a
/// schema path and at every known `host.<name>`, so no value lands under a key
/// that holds something else. Every other key and table of the base is kept:
/// `[project]`, unknown names, wrongly scoped names, other host sections and
/// sibling values. The file is written whole, so comments and key order are
/// not kept (ADR 0027). Repeats must already be collapsed. Nothing is written
/// to disk.
pub fn render_file(
    schema: &Schema,
    layer: FileLayer,
    base: Option<&SettingsFile>,
    pairs: &[TypedPair],
) -> Result<Vec<u8>, Unavailable> {
    let mut table = match base {
        None => Table::new(),
        Some(file) => {
            parse_layer(file, layer, schema)?;
            let (text, _) = document(file)?;
            // `document` has already judged the text, so this parse agrees.
            text.parse::<Table>().map_err(|error| Unavailable {
                path: file.path.clone(),
                fault: Fault::Parse {
                    position: error.span().map(|span| line_and_column(text, span.start)),
                    message: error.message().to_owned(),
                },
            })?
        }
    };
    for pair in pairs {
        let mut at = &mut table;
        if let Some(host) = pair.host {
            at = child(at, "host");
            at = child(at, host.name());
        }
        let segments: Vec<&str> = pair.name.split('.').collect();
        if let Some((last, parents)) = segments.split_last() {
            for segment in parents {
                at = child(at, segment);
            }
            at.insert((*last).to_owned(), toml_value(&pair.value));
        }
    }
    Ok(table.to_string().into_bytes())
}

/// The table at `key`, made when it is missing.
fn child<'t>(table: &'t mut Table, key: &str) -> &'t mut Table {
    let slot = table
        .entry(key)
        .or_insert_with(|| TomlValue::Table(Table::new()));
    // `parse_layer` has judged the base, so a key on a schema path holds a
    // table already. Replacing anything else keeps this total.
    if !slot.is_table() {
        *slot = TomlValue::Table(Table::new());
    }
    slot.as_table_mut().expect("the slot holds a table")
}

/// The TOML type each kind reads back: a boolean, or a string for a rung
/// name and a model name.
fn toml_value(value: &Value) -> TomlValue {
    match value {
        Value::Bool(value) => TomlValue::Boolean(*value),
        Value::Rung(rung) => TomlValue::String(rung.name().to_owned()),
        Value::ModelName(name) => TomlValue::String(name.clone()),
    }
}

/// The pairs the target file does not already hold, in the order given.
///
/// A pair is held when the file's layer has a written value of the same
/// name, at the same host (`None` for the top level) and equal to it. An
/// empty result is a no-op: a set that writes nothing and runs no step
/// (D-03). With no file, every pair is a change. Repeats must already be
/// collapsed.
pub fn changed_pairs(current: Option<&ParsedLayer>, pairs: &[TypedPair]) -> Vec<TypedPair> {
    pairs
        .iter()
        .filter(|pair| {
            !current.is_some_and(|layer| {
                layer.values.iter().any(|written| {
                    written.name == pair.name
                        && written.host == pair.host
                        && written.value == pair.value
                })
            })
        })
        .cloned()
        .collect()
}
