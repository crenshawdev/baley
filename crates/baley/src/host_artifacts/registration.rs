//! Claude Code's MCP registration for the direct stdio server (D-09).

use serde_json::{Value, json};

use super::executable::Executable;

/// The key the server is registered under in `mcpServers`. Build 4's agent
/// definitions name the session's entry by it, and under Claude Code the
/// query tool's full name follows from it as `mcp__baley__baley_query`.
pub const KEY: &str = "baley";

/// The registration: one `mcpServers` entry under [`KEY`] whose `command` is
/// the executable as given and whose `args` is `["serve"]`, with
/// `alwaysLoad: true` at the entry's level only when asked. The command is
/// an argument array, not a shell string, so the path is not quoted.
///
/// There is no `cwd`, `env`, `type`, `url`, `headers`, token or launcher: the
/// project comes from `CLAUDE_PROJECT_DIR` (ADR 0034), and the sandbox
/// settings never name the server.
pub fn render(executable: &Executable, always_load: bool) -> Value {
    let mut entry = json!({"command": executable.as_str(), "args": ["serve"]});
    if always_load {
        entry["alwaysLoad"] = Value::Bool(true);
    }
    json!({"mcpServers": {KEY: entry}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(always_load: bool) -> Value {
        let rendered = render(
            &Executable::new("/usr/local/bin/baley").unwrap(),
            always_load,
        );
        let servers = rendered["mcpServers"].as_object().expect("mcpServers");
        let keys: Vec<&String> = servers.keys().collect();
        assert_eq!(keys, [KEY]);
        assert_eq!(rendered.as_object().unwrap().len(), 1);
        servers[KEY].clone()
    }

    #[test]
    fn a_cwd_env_or_url_slipped_into_the_entry_or_always_load_missing_or_unasked_is_caught() {
        let plain = entry(false);
        let keys: Vec<&String> = plain.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["command", "args"]);
        assert_eq!(plain["command"], "/usr/local/bin/baley");
        assert_eq!(plain["args"], json!(["serve"]));

        let loaded = entry(true);
        let keys: Vec<&String> = loaded.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["command", "args", "alwaysLoad"]);
        assert_eq!(loaded["alwaysLoad"], true);
        assert_eq!(loaded["command"], plain["command"]);
        assert_eq!(loaded["args"], plain["args"]);
    }

    #[test]
    fn quoting_applied_to_the_command_where_no_shell_reads_it_is_caught() {
        let rendered = render(&Executable::new("/home/o w/baley").unwrap(), false);
        assert_eq!(rendered["mcpServers"][KEY]["command"], "/home/o w/baley");
    }
}
