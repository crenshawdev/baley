//! Genuine Claude Code and Codex host evidence for the phase 31 read boundary.
use super::phase31::{Client, ProcessFixture, approve, process_plan_submission};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    process::{Command, ExitStatus, Output, Stdio},
};

#[derive(Debug)]
struct Call {
    caller: String,
    arguments: Value,
    result: Value,
}

pub fn prove_worker_hosts_receive_main_thread_answers() {
    let fixture = host_fixture();
    let project = fixture.project();

    // A separate resident is the handwritten stdio oracle. Host workers never
    // use this connection, and every assertion below is made again on raw host
    // tool results rather than on either model's final prose.
    let mut oracle = Client::open(project);
    let oracle_answers = oracle_round(&mut oracle);
    assert_round("direct stdio oracle", &oracle_answers);
    oracle.finish();

    for host in [Host::Claude, Host::Codex] {
        let evidence = host.run(project);
        assert!(evidence.output.status.success(),
            "{} host/authentication/worker evidence is required; status={}\nstdout:\n{}\nstderr:\n{}",
            host.name(), evidence.output.status,
            bounded_output(&evidence.output.stdout), bounded_output(&evidence.output.stderr));
        assert!(!evidence.events.is_empty(), "{} emitted no JSON host evidence", host.name());
        assert!(evidence.worker_ids.len() == 1,
            "{} must dispatch one actual worker, got {:?}; events={}", host.name(),
            evidence.worker_ids, event_summary(&evidence.events));

        let calls = host.calls(&evidence.events);
        let callers: BTreeSet<_> = calls.iter().map(|call| call.caller.as_str()).collect();
        assert!(callers.len() >= 2,
            "{} did not expose distinct main/worker tool-result identities; calls={}",
            host.name(), call_summary(&calls));
        let worker = evidence.worker_ids.iter().next().unwrap();
        let worker_calls: Vec<_> = calls.iter().filter(|call| call.caller == *worker).collect();
        let main_calls: Vec<_> = calls.iter().filter(|call| call.caller != *worker).collect();
        assert_round(&format!("{} main thread", host.name()), &main_calls);
        assert_round(&format!("{} actual worker", host.name()), &worker_calls);

        let issued = main_calls.iter().find(|call| call.arguments["operation"] == "search"
            && call.arguments["pattern"] == "fn beta").unwrap().result["hits"][0]["location"]
            .as_str().unwrap();
        assert!(worker_calls.iter().any(|call| call.arguments == json!({"operation":"read","location":issued})
            && call.result["body"] == "fn beta() {\n    let needle = 3;\n}\n"),
            "{} worker did not follow the main thread's issued location on the shared resident",
            host.name());
        let repeated: Vec<_> = main_calls.iter().filter(|call| call.arguments["operation"] == "search"
            && call.arguments["pattern"] == "fn beta").collect();
        assert!(repeated.len() >= 2 && stable_hits(&repeated[0].result) == stable_hits(&repeated[1].result),
            "{} repeated main read changed after worker use", host.name());

        let server_pids: BTreeSet<_> = evidence.processes.iter().filter_map(|(pid, command)|
            command.contains("cadence serve").then_some(pid)).collect();
        assert_eq!(server_pids.len(), 1,
            "{} must share exactly one cadence server process: {:?}", host.name(), evidence.processes);
    }
}

fn stable_hits(answer: &Value) -> Vec<Value> {
    answer["hits"].as_array().unwrap().iter().map(|hit| json!({
        "file":hit["file"], "name":hit["name"], "kind":hit["kind"],
        "range":hit["range"], "match_lines":hit["match_lines"], "body":hit["body"],
        "body_truncated":hit["body_truncated"]
    })).collect()
}

fn bounded_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4_096)]).into_owned()
}

fn event_summary(events: &[Value]) -> String {
    events.iter().filter_map(|event| {
        let item = event.get("item")?;
        Some(format!("{}:{}", item["type"].as_str().unwrap_or("?"),
            item["tool"].as_str().or_else(|| item["server"].as_str()).unwrap_or("?")))
    }).collect::<Vec<_>>().join(",")
}

fn call_summary(calls: &[Call]) -> String {
    calls.iter().map(|call| format!("{}:{}:{}", call.caller,
        call.arguments["operation"].as_str().unwrap_or("?"),
        call.result["status"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join(",")
}

fn oracle_round(client: &mut Client) -> Vec<Call> {
    let mut calls = Vec::new();
    fn call(client: &mut Client, calls: &mut Vec<Call>, arguments: Value) {
        let result = client.call("cadence_query", arguments.clone());
        calls.push(Call { caller: "oracle".into(), arguments, result });
    }
    call(client, &mut calls, json!({"operation":"search","pattern":"fn beta","scope":{"kind":"directory","selector":"src"}}));
    let location = calls[0].result["hits"][0]["location"].clone();
    call(client, &mut calls, json!({"operation":"read","location":location}));
    call(client, &mut calls, json!({"operation":"search","pattern":"outline_needle","scope":{"kind":"directory","selector":"src"}}));
    let file = calls[2].result["hits"][0]["file_reference"].clone();
    call(client, &mut calls, json!({"operation":"read","file":file}));
    call(client, &mut calls, json!({"operation":"search","pattern":"first-marker","scope":{"kind":"directory","selector":"src"}}));
    let large = calls[4].result["hits"][0]["location"].clone();
    call(client, &mut calls, json!({"operation":"read","location":large}));
    call(client, &mut calls, json!({"operation":"document","identity":{"kind":"phase-context","phase":31},"part":"truth:T4"}));
    call(client, &mut calls, json!({"operation":"read","location":"unissued-opaque-token"}));
    calls
}

fn assert_round(label: &str, calls: &[impl std::borrow::Borrow<Call>]) {
    let calls: Vec<_> = calls.iter().map(std::borrow::Borrow::borrow).collect();
    let summary = calls.iter().map(|call| format!("{}:{}:{}", call.caller,
        call.arguments["operation"].as_str().unwrap_or("?"),
        call.result["status"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join(",");
    let find = |operation: &str, pattern: Option<&str>| calls.iter().find(|call| {
        call.arguments["operation"] == operation
            && pattern.is_none_or(|wanted| call.arguments["pattern"] == wanted)
    }).unwrap_or_else(|| panic!("{label} omitted {operation} {pattern:?}: {summary}"));
    let search = find("search", Some("fn beta"));
    assert_eq!(search.result["status"], "ok", "{label}: {summary}");
    assert_eq!(search.result["hits"][0]["name"], "beta", "{label}: {summary}");
    assert_eq!(search.result["hits"][0]["range"], json!([5, 7]), "{label}: {summary}");
    assert_eq!(search.result["hits"][0]["body"], "fn beta() {\n    let needle = 3;\n}\n");
    assert!(calls.iter().any(|call| call.arguments["operation"] == "read"
        && call.result["body"] == "fn beta() {\n    let needle = 3;\n}\n"), "{label} omitted the short slice");
    let outline = calls.iter().find(|call| call.arguments["operation"] == "read"
        && call.result["kind"] == "outline").unwrap_or_else(|| panic!("{label} omitted outline: {summary}"));
    assert_eq!(outline.result["rows"][0]["name"], "first_unit");
    assert_eq!(outline.result["rows"][0]["range"], json!([1, 3]));
    assert_eq!(outline.result["rows"][1]["name"], "second_unit");
    let cut = calls.iter().find(|call| call.result["kind"] == "slice"
        && call.result["truncated"] == true).unwrap_or_else(|| panic!("{label} omitted cut slice: {summary}"));
    assert!(cut.result["body"].as_str().unwrap().starts_with("fn oversized() {\n    // first-marker"));
    assert!(cut.result["continuation"].as_str().is_some_and(|value| !value.is_empty()));
    assert!(calls.iter().any(|call| call.arguments["operation"] == "document"
        && call.result["body"] == "When the caller asks for the host truth, the caller gets HANDWRITTEN HOST PROCESS SLICE.\n"),
        "{label} omitted the process slice");
    assert!(calls.iter().any(|call| call.arguments["location"] == "unissued-opaque-token"
        && call.result["status"] == "refused" && call.result["code"] == "location-not-issued"
        && call.result["rule"] == "D-147"), "{label} omitted the named refusal");
}

#[derive(Clone, Copy)]
enum Host { Claude, Codex }

struct Evidence {
    output: Output,
    events: Vec<Value>,
    worker_ids: BTreeSet<String>,
    processes: BTreeMap<u32, String>,
}

impl Host {
    fn name(self) -> &'static str { match self { Self::Claude => "Claude", Self::Codex => "Codex" } }

    fn run(self, project: &Path) -> Evidence {
        match self { Self::Claude => run_claude(project), Self::Codex => run_codex(project) }
    }

    fn calls(self, events: &[Value]) -> Vec<Call> {
        match self { Self::Claude => claude_calls(events), Self::Codex => codex_calls(events) }
    }
}

fn run_claude(project: &Path) -> Evidence {
    let config = project.join(".host/claude-mcp.json");
    fs::write(&config, serde_json::to_vec(&json!({"mcpServers":{"cadence":{
        "command":env!("CARGO_BIN_EXE_cadence"),"args":["serve","--project-root",project]
    }}})).unwrap()).unwrap();
    let worker = json!({"cad-read-worker":{"description":"Use the shared Cadence read contract",
        "prompt":worker_prompt(),"tools":["mcp__cadence__cadence_query"],"model":"inherit"}});
    let mut command = Command::new("claude");
    command.args(["--print","--output-format","stream-json","--verbose","--forward-subagent-text",
        "--strict-mcp-config","--mcp-config",config.to_str().unwrap(),"--agents",&worker.to_string(),
        "--tools","Agent,mcp__cadence__cadence_query","--dangerously-skip-permissions",
        "--permission-mode","bypassPermissions",
        "--no-session-persistence",main_prompt("Agent", "cad-read-worker")])
        .current_dir(project).stdin(Stdio::null());
    run_and_capture(command, Host::Claude)
}

fn run_codex(project: &Path) -> Evidence {
    let override_value = format!("mcp_servers.cadence={{command={:?},args=[\"serve\",\"--project-root\",{:?}]}}",
        env!("CARGO_BIN_EXE_cadence"), project.to_str().unwrap());
    let mut command = Command::new("codex");
    command.args(["exec","--json","--ephemeral","--ignore-user-config","--skip-git-repo-check",
        "--dangerously-bypass-approvals-and-sandbox","--config",&override_value,
        main_prompt("spawn_agent", "cad_read_worker")])
        .current_dir(project).stdin(Stdio::null());
    run_and_capture(command, Host::Codex)
}

fn run_and_capture(mut command: Command, host: Host) -> Evidence {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap_or_else(|error| panic!("{} host unavailable: {error}", host.name()));
    let root_pid = child.id();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_reader = std::thread::spawn(move || { let mut bytes = Vec::new(); stdout.read_to_end(&mut bytes).unwrap(); bytes });
    let err_reader = std::thread::spawn(move || { let mut bytes = Vec::new(); stderr.read_to_end(&mut bytes).unwrap(); bytes });
    let mut processes = BTreeMap::new();
    let status: ExitStatus = loop {
        sample_process_tree(root_pid, &mut processes);
        if let Some(status) = child.try_wait().unwrap() { break status }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let output = Output { status, stdout: out_reader.join().unwrap(), stderr: err_reader.join().unwrap() };
    let events: Vec<Value> = String::from_utf8_lossy(&output.stdout).lines()
        .filter_map(|line| serde_json::from_str(line).ok()).collect();
    let worker_ids = worker_ids(host, &events);
    Evidence { output, events, worker_ids, processes }
}

fn main_prompt(worker_tool: &str, worker_name: &str) -> &'static str {
    let dispatch = if worker_tool == "Agent" {
        format!("Call the Agent tool with subagent_type `{worker_name}` and a prompt containing the exact beta location. Wait for that foreground Agent call to return.")
    } else {
        format!("Call the collaboration spawn_agent tool with task_name `{worker_name}` and a message containing the exact beta location. Then call wait_agent for that returned worker id until it completes.")
    };
    Box::leak(format!(r#"Use only the configured cadence MCP server and the {worker_tool} worker mechanism. Do not use filesystem, shell, web, or final-answer reconstruction for project content.
1. On the main thread call cadence_query search for `fn beta` in directory scope `src`, then read its issued location. Call search for `outline_needle` in `src` and read the issued file_reference for its outline. Call search for `first-marker` in `src` and read its issued location for the cut slice. Call document for phase-context 31 part truth:T4. Call read with location `unissued-opaque-token`.
2. {dispatch} Tell it to perform the same search/read/outline/cut/document/refusal requests through its inherited cadence_query connection and no other read tool.
3. After the worker returns, repeat the main-thread `fn beta` search. Then stop. Tool results are the evidence; do not summarize their content."#).into_boxed_str())
}

fn worker_prompt() -> &'static str {
    "Use only mcp__cadence__cadence_query on the already configured shared server. Never use a filesystem, shell, web, or another MCP server. Follow the parent's exact read requests and return only after all real tool calls complete."
}

// The two host parsers below intentionally accept only actual emitted tool
// events. They never accept an assistant text block as evidence.
fn claude_calls(events: &[Value]) -> Vec<Call> {
    let mut pending: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut calls = Vec::new();
    for event in events {
        let caller = event["parent_tool_use_id"].as_str()
            .or_else(|| event["session_id"].as_str()).unwrap_or("main").to_owned();
        if let Some(content) = event.pointer("/message/content").and_then(Value::as_array) {
            for block in content {
                if block["type"] == "tool_use" && block["name"] == "mcp__cadence__cadence_query" {
                    pending.insert(block["id"].as_str().unwrap().to_owned(), (caller.clone(), block["input"].clone()));
                }
                if block["type"] == "tool_result" {
                    if let Some((caller, arguments)) = block["tool_use_id"].as_str().and_then(|id| pending.remove(id)) {
                        if let Some(result) = tool_result_value(block) { calls.push(Call { caller, arguments, result }); }
                    }
                }
            }
        }
    }
    calls
}

fn tool_result_value(block: &Value) -> Option<Value> {
    if block["content"].is_object() { return Some(block["content"].clone()); }
    let text = block["content"].as_str().map(str::to_owned).or_else(|| block["content"].as_array().map(|parts| {
        parts.iter().filter_map(|part| part["text"].as_str()).collect::<String>()
    }))?;
    serde_json::from_str(&text).ok()
}

fn codex_calls(events: &[Value]) -> Vec<Call> {
    events.iter().filter_map(|event| {
        let item = &event["item"];
        if event["type"] != "item.completed" || item["type"] != "mcp_tool_call"
            || item["server"] != "cadence" || item["tool"] != "cadence_query" { return None }
        Some(Call {
            caller: item["agent_id"].as_str().or_else(|| item["thread_id"].as_str())
                .unwrap_or("main").to_owned(),
            arguments: item["arguments"].clone(),
            result: item.pointer("/result/structured_content").cloned()?,
        })
    }).collect()
}

fn worker_ids(host: Host, events: &[Value]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for event in events {
        match host {
            Host::Claude => {
                if let Some(id) = event["parent_tool_use_id"].as_str() { ids.insert(id.to_owned()); }
            }
            Host::Codex => {
                let item = &event["item"];
                if item["type"] == "collab_tool_call" && item["tool"] == "spawn_agent" {
                    if let Some(id) = item["receiver_thread_id"].as_str()
                        .or_else(|| item.pointer("/result/agent_id").and_then(Value::as_str)) { ids.insert(id.to_owned()); }
                }
            }
        }
    }
    ids
}

fn sample_process_tree(root: u32, found: &mut BTreeMap<u32, String>) {
    let mut parents = BTreeMap::new();
    let Ok(entries) = fs::read_dir("/proc") else { return };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else { continue };
        let Some(after_name) = stat.rsplit_once(") ").map(|(_, tail)| tail) else { continue };
        let Some(ppid) = after_name.split_whitespace().nth(1).and_then(|value| value.parse::<u32>().ok()) else { continue };
        parents.insert(pid, ppid);
    }
    for (&pid, _) in &parents {
        let mut cursor = pid;
        let mut descendant = pid == root;
        for _ in 0..64 {
            if descendant || cursor == 0 { break }
            cursor = parents.get(&cursor).copied().unwrap_or(0);
            descendant = cursor == root;
        }
        if descendant {
            let bytes = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let command = String::from_utf8_lossy(&bytes).replace('\0', " ");
            found.insert(pid, command);
        }
    }
}

fn host_fixture() -> ProcessFixture {
    let fixture = ProcessFixture::new();
    let project = fixture.project();
    fs::create_dir_all(project.join(".host")).unwrap();
    fs::write(project.join("src/units.rs"), "fn alpha() {\n    let needle = 1;\n}\n\nfn beta() {\n    let needle = 3;\n}\n").unwrap();
    fs::write(project.join("src/units.js"), "function javascriptUnit() {\n  const needle = 1;\n}\n").unwrap();
    fs::write(project.join("docs/units.md"), "# Markdown unit\nneedle\n").unwrap();
    fs::write(project.join("src/units.json"), "{\n  \"jsonUnit\": \"needle\"\n}\n").unwrap();
    fs::write(project.join("src/units.c"), "int c_unit(void) {\n  int needle = 1;\n  return needle;\n}\n").unwrap();
    fs::create_dir_all(project.join("ignored")).unwrap();
    fs::write(project.join("ignored/sentinel.rs"), "fn ignored() { let needle = 0; }\n").unwrap();
    fs::write(project.join(".gitignore"), ".planning/\n.fixture-gnupg/\n.host/\nignored/\n").unwrap();
    let padding = "// outline padding\n".repeat(2_000);
    fs::write(project.join("src/outline.rs"), format!(
        "fn first_unit() {{\n    let outline_needle = 1;\n}}\n{padding}fn second_unit() {{\n    let outline_needle = 2;\n}}\n")).unwrap();
    fs::write(project.join("src/oversized.rs"), format!(
        "fn oversized() {{\n    // first-marker {} last-marker\n}}\n", "é".repeat(40_000))).unwrap();

    let mut client = Client::open(project);
    let context = client.call("cadence_apply", approve(json!({"operation":"context-submit","submission":{
        "phase":31,"title":"Host read boundary","scope":"A genuine worker host fixture.",
        "durable_decisions":[],"decisions":[],"assumptions":[],"truths":[{
            "id":"T4","trigger":"the caller asks for the host truth","observer":"the caller",
            "verb":"gets","outcome":"HANDWRITTEN HOST PROCESS SLICE","kind":"property",
            "observable":true,"fixed_oracle":true}]}})));
    assert_eq!(context["persisted"], true, "{context}");
    let allocation = client.call("cadence_query", json!({"operation":"plan-read","phase_address":"31","count":2}));
    let preview_request = process_plan_submission(&allocation, "HOST PLAN SELECTED TASK\n");
    let preview = client.call("cadence_query", json!({"operation":"plan-read","phase_address":"31",
        "submission":preview_request["submission"]}));
    let published = client.call("cadence_apply", approve(json!({"operation":"plan-submit",
        "submission":preview["submission"]})));
    assert_eq!(published["persisted"], true, "{published}");
    client.finish();
    fixture
}
