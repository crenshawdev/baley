//! The three tools, what the server says about itself and what `baley_version` answers.

use rmcp::model::{
    CacheScope, Implementation, ListToolsResult, MetaObject, ProtocolVersion, ServerCapabilities,
    ServerConfig, Tool as RmcpTool,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::operations::{Status, Tool, served_schema};
use crate::envelope::Envelope;

/// What `baley_version` reports on success.
///
/// A struct rather than a bare string because an `ok` envelope's payload sits
/// beside the `status` tag at the top level, so it has to have named fields
/// (see `envelope::Envelope`). Three of them, and each answers a different
/// question a user actually asks when a session behaves unexpectedly: which
/// release, and which of the four release archives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VersionReport {
    /// The crate version of the running binary, matching the release tag it
    /// was cut from.
    pub version: String,
    /// The operating system this binary was built for: `linux` or `macos`.
    pub os: String,
    /// The CPU architecture this binary was built for: `x86_64` or `aarch64`.
    pub arch: String,
}

/// Answers `baley_version`. Only an empty arguments object is accepted.
///
/// Read from this binary's own compile-time constants, never from a manifest
/// on disk: the question is which binary is serving, and a file beside it can
/// be from a different install.
pub fn version_answer(arguments: Option<&Value>) -> Value {
    let envelope = match arguments {
        Some(Value::Object(map)) if map.is_empty() => Envelope::Ok(VersionReport {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }),
        _ => Envelope::Refused {
            code: "invalid-arguments".into(),
            reason: "baley_version requires an empty arguments object".into(),
        },
    };
    serde_json::to_value(envelope).expect("version envelope")
}

/// The one-line pointer the server gives a client at initialize. Claude Code
/// cuts instructions at 2,048 characters, so this stays one short line.
pub const INSTRUCTIONS: &str =
    "Call baley_query with {\"operation\":\"help\"} to learn what Baley does and how to use it.";

/// The server's answer to the host's initialize request.
pub fn info() -> ServerConfig {
    let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new("baley", env!("CARGO_PKG_VERSION")));
    info.instructions = Some(INSTRUCTIONS.to_owned());
    info
}

/// The protocol revisions the server implements and has tested, and no others.
pub fn supported_protocol_versions() -> Vec<ProtocolVersion> {
    vec![ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28]
}

/// The server's answer to `tools/list`.
pub fn tool_list() -> ListToolsResult {
    // MCP 2026-07-28 requires both fields on a list result, and a host drops
    // a list without them. Zero makes the host fetch the list again rather
    // than keep one from an older binary.
    ListToolsResult {
        tools: tools(),
        ..Default::default()
    }
    .with_ttl_ms(0)
    .with_cache_scope(CacheScope::Private)
}

/// The three tools, in a fixed order, each loaded up front by Claude Code.
pub fn tools() -> Vec<RmcpTool> {
    let always_load = || {
        let Value::Object(map) = json!({"anthropic/alwaysLoad": true}) else {
            unreachable!("a JSON object literal is an object")
        };
        MetaObject(map)
    };
    let tool = |name: &'static str, description: &'static str, schema: Value| {
        let Value::Object(schema) = schema else {
            unreachable!("a tool schema is an object")
        };
        RmcpTool::new(name, description, schema).with_meta(always_load())
    };
    vec![
        tool(
            "baley_version",
            "Report this binary's version, OS and architecture without changing state.",
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "title": "VersionArguments",
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        ),
        tool(
            "baley_query",
            "Read the bound project's records, configuration, routing and evidence without changing state; request shapes come from baley_query {\"operation\":\"schema\",\"tool\":\"query\",\"for\":\"<operation>\"} and the compiled contracts. Read process records through document and document-search; read project source with the host's own tools.",
            operation_schema(Tool::Query),
        ),
        tool(
            "baley_apply",
            "Change the bound project through one replay-safe operation that is refused with a located rule when it cannot apply; request shapes come from baley_query {\"operation\":\"schema\",\"tool\":\"apply\",\"for\":\"<operation>\"} and the compiled contracts.",
            operation_schema(Tool::Apply),
        ),
    ]
}

/// A flat schema: one required `operation` naming the tool's spellings in
/// baseline order, retired ones included since names are only added, and
/// beside it every argument the served operations take, typed.
///
/// The top stays one plain object because the Messages API refuses a tool
/// whose input schema has `oneOf`, `anyOf` or `allOf` at the top. Each
/// argument comes from the same request shapes `schema` serves, so the two
/// cannot drift. An argument two operations type differently is an `anyOf`
/// of the shapes, and each shape names the operations that take it. Other
/// properties stay allowed so a retired spelling is answered, not blocked.
fn operation_schema(tool: Tool) -> Value {
    let names: Vec<&str> = tool.operations().iter().map(|op| op.name).collect();
    let which = match tool {
        Tool::Query => "query",
        Tool::Apply => "apply",
    };
    let mut properties = Map::new();
    properties.insert(
        "operation".into(),
        json!({
            "type": "string",
            "enum": names,
            "description": format!("Full request shapes: baley_query {{\"operation\":\"schema\",\"tool\":\"{which}\",\"for\":\"<operation>\"}} or the compiled contracts.")
        }),
    );
    properties.extend(arguments(tool));
    json!({
        "type": "object",
        "required": ["operation"],
        "properties": properties,
        "additionalProperties": true
    })
}

/// One argument's distinct shapes, in baseline order, each with the
/// operations that take it and whether each requires it.
type Shapes = Vec<(Value, Vec<(&'static str, bool)>)>;

/// The arguments of the tool's served operations, keyed by name.
fn arguments(tool: Tool) -> Map<String, Value> {
    let mut found: Vec<(String, Shapes)> = Vec::new();
    for op in tool.operations() {
        if !matches!(op.status, Status::Available { .. }) {
            continue;
        }
        let shape = served_schema(op.name).expect("every served operation has a request shape");
        let required = shape["required"].as_array().cloned().unwrap_or_default();
        let Some(own) = shape["properties"].as_object() else {
            continue;
        };
        for (name, schema) in own.iter().filter(|(name, _)| *name != "operation") {
            let taker = (op.name, required.contains(&json!(name)));
            let index = match found.iter().position(|(known, _)| known == name) {
                Some(index) => index,
                None => {
                    found.push((name.clone(), Vec::new()));
                    found.len() - 1
                }
            };
            let shapes = &mut found[index].1;
            match shapes.iter_mut().find(|(known, _)| known == schema) {
                Some((_, takers)) => takers.push(taker),
                None => shapes.push((schema.clone(), vec![taker])),
            }
        }
    }
    found
        .into_iter()
        .map(|(name, shapes)| {
            let mut alternatives: Vec<Value> = shapes
                .into_iter()
                .map(|(schema, takers)| with_takers(schema, &takers))
                .collect();
            let property = if alternatives.len() == 1 {
                alternatives.remove(0)
            } else {
                json!({"anyOf": alternatives})
            };
            (name, property)
        })
        .collect()
}

/// The shape with a closing sentence naming its operations, such as
/// "Taken by document and instruction.".
fn with_takers(mut schema: Value, takers: &[(&str, bool)]) -> Value {
    let named: Vec<String> = takers
        .iter()
        .map(|(op, required)| match required {
            true => format!("{op} (required)"),
            false => (*op).to_owned(),
        })
        .collect();
    let list = match named.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    let sentence = format!("Taken by {list}.");
    let description = match schema["description"].as_str() {
        Some(own) => format!("{own} {sentence}"),
        None => sentence,
    };
    schema["description"] = Value::String(description);
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::operations::expected::{APPLY, QUERY};

    #[test]
    fn version_with_empty_arguments_is_ok_with_version_os_and_arch() {
        let answer = version_answer(Some(&json!({})));
        assert_eq!(answer["status"], "ok");
        assert_eq!(answer["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(answer["os"], std::env::consts::OS);
        assert_eq!(answer["arch"], std::env::consts::ARCH);
    }

    #[test]
    fn version_with_arguments_missing_or_not_an_object_is_refused() {
        for arguments in [
            Some(json!({"x": 1})),
            None,
            Some(json!([])),
            Some(json!(null)),
        ] {
            let answer = version_answer(arguments.as_ref());
            assert_eq!(answer["status"], "refused", "{arguments:?}");
            assert_eq!(answer["code"], "invalid-arguments");
        }
    }
    fn listed() -> Value {
        serde_json::to_value(tool_list()).unwrap()
    }

    #[test]
    fn the_tool_list_is_version_query_apply_in_that_order() {
        let list = listed();
        let names: Vec<_> = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["baley_version", "baley_query", "baley_apply"]);
    }

    #[test]
    fn every_descriptor_asks_claude_code_to_always_load_it() {
        for tool in listed()["tools"].as_array().unwrap() {
            assert_eq!(
                tool["_meta"],
                json!({"anthropic/alwaysLoad": true}),
                "{}",
                tool["name"]
            );
        }
    }

    #[test]
    fn the_list_carries_ttl_zero_and_a_private_cache_scope() {
        let list = listed();
        assert_eq!(list["ttlMs"], json!(0));
        assert_eq!(list["cacheScope"], json!("private"));
    }

    #[test]
    fn the_operation_enums_are_todays_spellings_retired_ones_included() {
        let list = listed();
        let tools = list["tools"].as_array().unwrap();
        for (tool, expected) in [(&tools[1], QUERY), (&tools[2], APPLY)] {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["required"], json!(["operation"]));
            assert_eq!(schema["properties"]["operation"]["enum"], json!(expected));
            assert_eq!(schema["additionalProperties"], json!(true));
        }
    }

    fn input_schema(tool: Tool) -> Value {
        let index = match tool {
            Tool::Query => 1,
            Tool::Apply => 2,
        };
        listed()["tools"][index]["inputSchema"].clone()
    }

    /// One property's alternatives: its `anyOf` members, or itself.
    fn alternatives(property: &Value) -> Vec<Value> {
        match property["anyOf"].as_array() {
            Some(members) => members.clone(),
            None => vec![property.clone()],
        }
    }

    /// The plain shapes under one alternative, through any nested `oneOf`.
    fn leaves(schema: &Value) -> Vec<Value> {
        match schema["oneOf"].as_array() {
            Some(members) => members.iter().flat_map(leaves).collect(),
            None => vec![schema.clone()],
        }
    }

    /// The alternative of a property with a leaf of the given type.
    fn alternative_of_type(property: &Value, kind: &str) -> Value {
        alternatives(property)
            .into_iter()
            .find(|alt| leaves(alt).iter().any(|leaf| leaf["type"] == kind))
            .unwrap_or_else(|| panic!("no {kind} alternative in {property}"))
    }

    fn types(schema: &Value) -> Vec<String> {
        match &schema["type"] {
            Value::String(one) => vec![one.clone()],
            Value::Array(many) => many
                .iter()
                .map(|t| t.to_string().replace('"', ""))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_document_identity_or_part_published_as_loose_text_is_caught() {
        // Issue 232: with no shape to follow, the host sent both as JSON text.
        let schema = input_schema(Tool::Query);
        let identity = alternative_of_type(&schema["properties"]["identity"], "object");
        let objects = leaves(&identity);
        assert_eq!(objects.len(), 1, "{identity}");
        let object = &objects[0];
        assert_eq!(object["properties"]["kind"]["const"], "capture");
        assert_eq!(object["properties"]["id"]["type"], "string");
        assert_eq!(object["required"], json!(["kind", "id"]));
        assert_eq!(object["additionalProperties"], false);
        let parts = alternatives(&schema["properties"]["part"]);
        assert!(!parts.is_empty(), "no part in {schema}");
        for part in parts {
            let types = types(&part);
            assert!(types.contains(&"integer".into()), "{part}");
            assert!(!types.contains(&"string".into()), "{part}");
        }
    }

    #[test]
    fn a_served_operations_argument_left_out_of_the_tool_schema_is_caught() {
        // Design 0012: help takes name, schema takes tool and for, instruction
        // and document take identity, all four take part, and capture takes
        // request_id, kind, text, phase and instruction.
        let names = |tool| {
            let schema = input_schema(tool);
            let mut names: Vec<String> = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            names.sort();
            names
        };
        assert_eq!(
            names(Tool::Query),
            ["for", "identity", "name", "operation", "part", "tool"]
        );
        assert_eq!(
            names(Tool::Apply),
            [
                "instruction",
                "kind",
                "operation",
                "phase",
                "request_id",
                "text"
            ]
        );
    }

    #[test]
    fn a_tool_schema_with_a_combinator_at_the_top_is_caught() {
        // The Messages API refuses a tool whose input schema has any of these
        // at the top, and that breaks the whole session.
        for tool in [Tool::Query, Tool::Apply] {
            let schema = input_schema(tool);
            assert_eq!(schema["type"], "object");
            for combinator in ["oneOf", "anyOf", "allOf"] {
                assert!(schema.get(combinator).is_none(), "{combinator} in {schema}");
            }
            assert!(!schema.to_string().contains("\"$ref\""), "{schema}");
        }
    }

    #[test]
    fn an_argument_whose_operations_or_requirement_go_unnamed_is_caught() {
        let query = input_schema(Tool::Query);
        let description = |alt: &Value| alt["description"].as_str().unwrap_or("").to_owned();
        let by_type =
            |kind| description(&alternative_of_type(&query["properties"]["identity"], kind));
        assert!(by_type("object").ends_with("Taken by document (required)."));
        assert!(by_type("string").ends_with("Taken by instruction (required)."));
        let parts: Vec<String> = alternatives(&query["properties"]["part"])
            .iter()
            .map(description)
            .collect();
        assert!(
            parts.iter().any(|d| d.ends_with("Taken by help.")),
            "{parts:?}"
        );
        assert!(
            parts
                .iter()
                .any(|d| d.ends_with("Taken by document and instruction.")),
            "{parts:?}"
        );
        let apply = input_schema(Tool::Apply);
        assert!(
            description(&apply["properties"]["request_id"])
                .ends_with("Taken by capture (required).")
        );
        assert!(description(&apply["properties"]["phase"]).ends_with("Taken by capture."));
    }

    #[test]
    fn a_tool_property_that_drifts_from_the_served_schema_is_caught() {
        let bare = |schema: &Value| {
            let mut schema = schema.clone();
            schema.as_object_mut().unwrap().remove("description");
            schema
        };
        for (tool, which) in [(Tool::Query, "query"), (Tool::Apply, "apply")] {
            let published = input_schema(tool);
            for operation in tool.operations() {
                let served = crate::mcp::operations::schema_answer(
                    &json!({"operation": "schema", "tool": which, "for": operation.name}),
                );
                if served["status"] != "ok" {
                    continue;
                }
                for (name, own) in served["schema"]["properties"].as_object().unwrap() {
                    if name == "operation" {
                        continue;
                    }
                    let own_text = own["description"].as_str().unwrap_or("");
                    let kept = alternatives(&published["properties"][name])
                        .iter()
                        .any(|alt| {
                            bare(alt) == bare(own)
                                && alt["description"]
                                    .as_str()
                                    .is_some_and(|text| text.starts_with(own_text))
                        });
                    assert!(kept, "{}.{name} drifted from {own}", operation.name);
                }
            }
        }
    }

    #[test]
    fn the_version_tool_takes_no_arguments() {
        let list = listed();
        let schema = &list["tools"][0]["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"], json!({}));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn the_instructions_are_one_short_line_naming_help() {
        assert!(!INSTRUCTIONS.contains('\n'));
        assert!(INSTRUCTIONS.chars().count() < 2048);
        assert!(INSTRUCTIONS.contains("help"));
        assert_eq!(info().instructions.as_deref(), Some(INSTRUCTIONS));
    }

    #[test]
    fn the_server_is_named_baley_and_offers_tools() {
        let info = info();
        assert_eq!(info.server_info.name, "baley");
        assert!(info.capabilities.tools.is_some());
    }

    #[test]
    fn only_the_two_tested_protocol_revisions_are_supported() {
        assert_eq!(
            supported_protocol_versions(),
            [ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28]
        );
    }
}
