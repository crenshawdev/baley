use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Revision { pub branch: String, pub head: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Remote { pub name: String, pub url: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub request_id: String,
    pub occurrence: String,
    pub expected_generation: u64,
    pub source: Revision,
    pub base: Revision,
    pub remote: Remote,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    pub request_id: String,
    pub landing: String,
    pub expected_generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum Apply {
    #[serde(rename = "land-start")]
    Start { request: Start },
    #[serde(rename = "land-publish")]
    Publish { request: Publish },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step { Publish, Open, Merge, Checkout, Pull, Tag, TagPush, Reap }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepSlot { pub step: Step, pub receipt: Option<Value> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Landing {
    pub id: String,
    pub root_binding: String,
    pub occurrence: String,
    pub generation: u64,
    pub source: Revision,
    pub base: Revision,
    pub remote: Remote,
    pub steps: Vec<StepSlot>,
}
impl Landing {
    pub fn new(root: String, request: &Start) -> Self {
        Self { id: crate::milestone::model::identity("landing", &root, &request.occurrence), root_binding: root,
            occurrence: request.occurrence.clone(), generation: 1, source: request.source.clone(), base: request.base.clone(), remote: request.remote.clone(),
            steps: [Step::Publish, Step::Open, Step::Merge, Step::Checkout, Step::Pull, Step::Tag, Step::TagPush, Step::Reap]
                .into_iter().map(|step| StepSlot { step, receipt: None }).collect() }
    }
}
