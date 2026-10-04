//! The three tools, what the server says about itself and what `baley_version` answers.

use rmcp::model::{
    CacheScope, Implementation, ListToolsResult, MetaObject, ProtocolVersion, ServerCapabilities,
    ServerConfig, Tool as RmcpTool,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::operations::Tool;
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
/// baseline order, retired ones included since names are only added. Any other
/// property is allowed, and `schema` answers each operation's own shape.
fn operation_schema(tool: Tool) -> Value {
    let names: Vec<&str> = tool.operations().iter().map(|op| op.name).collect();
    let which = match tool {
        Tool::Query => "query",
        Tool::Apply => "apply",
    };
    json!({
        "type": "object",
        "required": ["operation"],
        "properties": {"operation": {
            "type": "string",
            "enum": names,
            "description": format!("Full request shapes: baley_query {{\"operation\":\"schema\",\"tool\":\"{which}\",\"for\":\"<operation>\"}} or the compiled contracts.")
        }},
        "additionalProperties": true
    })
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
