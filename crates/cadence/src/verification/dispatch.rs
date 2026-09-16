use super::{inputs::Inputs, instructions};

pub fn prompt(inputs: &Inputs, documents: &std::collections::BTreeMap<String, String>) -> crate::store::Result<String> {
    let authored: std::collections::BTreeMap<_, _> = inputs.basis.publications.iter().filter_map(|p| {
        let path = format!("phases/{}/PLAN-{}.md", inputs.basis.phase, p.plan);
        documents.get(&path).map(|body| (path, body))
    }).collect();
    Ok(format!("{}\n<operational-input>\n{}\n</operational-input>\n<authored-material>\n{}\n</authored-material>\n",
        instructions::contract_markdown(), serde_json::to_string_pretty(inputs)?, serde_json::to_string_pretty(&authored)?))
}

#[cfg(test)]
mod tests {
    use super::super::model::{Basis, Source};
    use super::super::inputs::Inputs;
    use serde_json::json;

    fn inputs(execution: serde_json::Value) -> Inputs {
        Inputs {
            basis: Basis { project: "/p".into(), root_binding: "1:1;".into(), phase: 6, occurrence: "active-cycle:phase:6".into(),
                context_digest: "c".repeat(64), truths: vec![], publications: vec![], map_digest: "m".repeat(64),
                admission_digests: vec![], execution_digest: "e".repeat(64),
                source: Source { head: "h".repeat(40), tree: "t".repeat(40), index_digest: "i".repeat(64), material_digest: "d".repeat(64) } },
            map: json!({}), admissions: vec![], execution, checks: vec![], authority_digest: "a".repeat(64),
        }
    }

    // D-177: a retained run's captured output stays on the record and reaches
    // the verifier by identity; the prompt never carries the bytes.
    #[test]
    fn prompt_names_captured_output_by_identity_and_length() {
        let stdout = json!({"bytes":[116,101,115,116,32,114,101,115,117,108,116,58,32,111,107,10],"digest":"f".repeat(64),"complete":true,"result_lines":["test result: ok"]});
        let stderr = json!({"bytes":[],"digest":"0".repeat(64),"complete":true});
        let execution = json!({"events":[{"version":2,"request":{"request_id":"r1","event":{"kind":"run","stdout":stdout,"stderr":stderr}}}],
            "plan_events":[],"outcomes":{"bytes":"not a capture","keep":[1,2,3]}});
        let prompt = super::prompt(&inputs(execution), &Default::default()).unwrap();
        let start = prompt.find("<operational-input>\n").unwrap() + "<operational-input>\n".len();
        let end = prompt.find("\n</operational-input>").unwrap();
        let rendered: serde_json::Value = serde_json::from_str(&prompt[start..end]).unwrap();
        let run = &rendered["execution"]["events"][0]["request"]["event"];
        assert_eq!(run["stdout"], json!({"digest":"f".repeat(64),"complete":true,"result_lines":["test result: ok"],"byte_length":16}));
        assert_eq!(run["stderr"], json!({"digest":"0".repeat(64),"complete":true,"byte_length":0}));
        assert_eq!(rendered["execution"]["outcomes"], json!({"bytes":"not a capture","keep":[1,2,3]}));
        assert!(!prompt.contains("116,"), "capture bytes leaked into the prompt");
    }
}
