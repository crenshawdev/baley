use super::*;
use cadence::review::provider::{credentials, delivery, transport};
use std::{collections::{BTreeMap, VecDeque}, fs, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

tokio::task_local! { pub(super) static ENVIRONMENT: Arc<delivery::Environment>; }

struct Credentials;
impl credentials::Inputs for Credentials {
    fn env(&self, name: &str) -> Option<String> {
        (name == "OPENAI_API_KEY").then(|| "fixture-key".into())
    }
    fn read(&self, _: &Path) -> Option<String> { unreachable!("no credential files") }
}
struct Chunks(VecDeque<Vec<u8>>);
impl transport::Body for Chunks {
    fn chunk(&mut self) -> transport::Pending<'_, Option<Vec<u8>>> { Box::pin(async { Ok(self.0.pop_front()) }) }
}
#[derive(Default)]
struct Wire { requests: Mutex<Vec<Value>>, hold: std::sync::atomic::AtomicBool }
impl transport::Transport for Wire {
    fn send(&self, request: transport::Request, _: Duration) -> transport::Pending<'_, transport::Response> {
        Box::pin(async move {
            assert_eq!(request.url, "https://api.openai.com/v1/responses");
            self.requests.lock().unwrap().push(request.body);
            if self.hold.load(std::sync::atomic::Ordering::SeqCst) { std::future::pending::<()>().await; }
            let bytes = serde_json::to_vec(&json!({"id":"fixture-response","model":"fixture-consult",
                "output_text":json!({"angles":[{"hypothesis":"cache key omits tenant","rationale":"two tenants share an entry",
                    "how_to_check":"compare keys for tenant A and B"}]}).to_string(),
                "usage":{"input_tokens":30,"output_tokens":20}})).unwrap();
            Ok(transport::Response { status: 200, headers: BTreeMap::from([("x-request-id".into(), "fixture-http".into())]),
                body: Box::new(Chunks(bytes.chunks(31).map(<[u8]>::to_vec).collect())) })
        })
    }
}
fn factory() -> SessionFactory { SessionFactory::new(None, Arc::new(crate::config::planning_policy)) }
async fn fixture(enabled: bool, model: bool, cap: Option<u64>) -> (tempfile::TempDir, PathBuf, SessionFactory) {
    let tree = tempfile::tempdir().unwrap();
    let root = tree.path().join(".planning");
    fs::create_dir(&root).unwrap();
    let git = std::process::Command::new("git").args(["init", "--initial-branch=fixture/consult"])
        .current_dir(tree.path()).output().unwrap();
    assert!(git.status.success());
    let mut config = json!({"memory":{"backend":"none"},
        "review":{"consult":{"enabled":enabled,"attempt_threshold":2,"tier":"flagship","effort":"high"},
        "providers":{"openai":{"tiers":{"flagship":if model { json!("fixture-consult") } else { Value::Null }}}}}});
    if let Some(cap) = cap { config["review"]["max_prompt_tokens"] = json!(cap); }
    fs::write(root.join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let factory = factory();
    factory.first_touch(&root).await.unwrap();
    (tree, root, factory)
}
async fn read(factory: &SessionFactory, root: &Path) -> Value {
    execute(factory, root, Command::Read { slug: "tenant-cache".into() }).await.unwrap()
}
async fn call(factory: &SessionFactory, root: &Path, operation: &str, request: Value) -> Value {
    execute(factory, root, Command::Apply(serde_json::from_value(json!({"operation":operation,"request":request})).unwrap())).await.unwrap()
}
async fn change(factory: &SessionFactory, root: &Path, operation: &str, id: &str, mut fields: Value) -> Value {
    let version = if operation == "debug-open" { json!(0) } else { read(factory, root).await["record"]["version"].clone() };
    fields["request_id"] = json!(id); fields["slug"] = json!("tenant-cache"); fields["expected_version"] = version;
    let answer = call(factory, root, operation, fields).await;
    assert_eq!(answer["status"], "ok", "{answer}"); answer
}
async fn failed(factory: &SessionFactory, root: &Path, id: &str) -> Value {
    change(factory, root, "debug-attempt", id, json!({"attempt":{"description":"flush cache","result":"still shares tenants"}})).await
}
fn offers(answer: &Value) -> Vec<Value> { answer["record"]["consults"].as_array().cloned().unwrap_or_default() }
fn decision(answer: &Value, id: &str, action: &str) -> Value {
    let offer = offers(answer).last().unwrap().clone();
    json!({"request_id":id,"slug":"tenant-cache","expected_version":answer["record"]["version"],
        "offer":offer["id"],"epoch":offer["epoch"],"decision":action})
}

#[tokio::test]
async fn debug_consult_is_offered_once_per_dead_end() {
    let wire = Arc::new(Wire::default());
    let environment = Arc::new(delivery::Environment { credentials: Arc::new(Credentials), transport: wire.clone(),
        sleep: Arc::new(|_| Box::pin(std::future::pending())) });
    ENVIRONMENT.scope(environment, async {
        for (enabled, model) in [(false,true),(true,false)] {
            let (_tree, root, factory) = fixture(enabled, model, None).await;
            change(&factory, &root, "debug-open", "open", json!({"symptom":"tenant cache leaks entries"})).await;
            failed(&factory, &root, "failed-one").await;
            assert!(offers(&failed(&factory, &root, "failed-two").await).is_empty());
            assert!(wire.requests.lock().unwrap().is_empty());
        }
        let (_tree, root, mut service) = fixture(true, true, None).await;
        let opened = change(&service, &root, "debug-open", "open", json!({"symptom":"tenant cache leaks entries; API_KEY=secret-value"})).await;
        assert!(offers(&opened).is_empty(), "empty hypotheses are not a dead end");
        assert!(offers(&failed(&service, &root, "failed-one").await).is_empty());
        let offered = failed(&service, &root, "failed-two").await;
        assert_eq!(offers(&offered).len(), 1, "threshold must durably offer consult");
        assert_eq!(offers(&offered)[0]["state"], "offered");
        assert!(wire.requests.lock().unwrap().is_empty(), "offering never spends");
        let third = failed(&service, &root, "failed-three").await;
        assert_eq!(offers(&third), offers(&offered));
        drop(service); service = factory();
        for _ in 0..3 { assert_eq!(offers(&read(&service, &root).await), offers(&offered)); }
        let request = decision(&read(&service, &root).await, "accept-first", "accept");
        let accepted = call(&service, &root, "debug-consult", request.clone()).await;
        assert_eq!(accepted["status"], "ok", "{accepted}");
        assert_eq!(offers(&accepted)[0]["state"], "completed");
        let angles = json!([{"hypothesis":"cache key omits tenant","rationale":"two tenants share an entry",
            "how_to_check":"compare keys for tenant A and B"}]);
        assert_eq!(offers(&accepted)[0]["angles"], angles);
        assert_eq!(call(&service, &root, "debug-consult", request.clone()).await, accepted);
        let mut changed = request; changed["decision"] = json!("decline");
        assert_eq!(call(&service, &root, "debug-consult", changed).await["code"], "request-reused");
        assert_eq!(wire.requests.lock().unwrap().len(), 1);
        {
            let requests = wire.requests.lock().unwrap(); let body = &requests[0];
            assert_eq!(body["model"], "fixture-consult"); assert_eq!(body["reasoning"]["effort"], "high");
            assert_eq!(body["text"]["format"]["name"], "consult_angles");
            assert_eq!(body["text"]["format"]["schema"]["required"], json!(["angles"]));
            let situation = body["input"][1]["content"].as_str().unwrap();
            assert!(situation.contains("tenant cache leaks entries") && situation.contains("still shares tenants"));
            assert!(!situation.contains("secret-value"));
            assert!(situation.contains("<debug-situation>") && situation.contains("</debug-situation>"));
        }
        drop(service); service = factory();
        let resumed = read(&service, &root).await;
        assert_eq!(offers(&resumed), offers(&accepted));
        assert_eq!(resumed["record"]["status"], "open"); assert!(resumed["record"]["resolution"].is_null());
        let projection = fs::read_to_string(root.join("debug/tenant-cache.md")).unwrap();
        assert_eq!(resumed["projection"], projection);
        for text in ["cache key omits tenant","two tenants share an entry","compare keys for tenant A and B","suggestions"] { assert!(projection.contains(text), "{text}"); }
        assert_eq!(offers(&failed(&service, &root, "failed-four").await).len(), 1);
        let observation = json!({"observation":{"test":"compare request logs","result":"two tenant ids share a key","rules_in":[],"rules_out":[]}});
        let next = change(&service, &root, "debug-observation", "new-evidence", observation.clone()).await;
        assert_eq!(offers(&next).len(), 2);
        assert_ne!(offers(&next)[0]["epoch"], offers(&next)[1]["epoch"]);
        let duplicate = change(&service, &root, "debug-observation", "duplicate-evidence", observation).await;
        assert_eq!(offers(&duplicate).len(), 2, "identical evidence does not open another epoch");
        let second = call(&service, &root, "debug-consult", decision(&duplicate, "accept-second", "accept")).await;
        assert_eq!(offers(&second)[1]["angles"], angles);
        assert_eq!(wire.requests.lock().unwrap().len(), 2);

        let (_decline_tree, decline_root, declined_service) = fixture(true, true, None).await;
        change(&declined_service, &decline_root, "debug-open", "open", json!({"symptom":"tenant cache"})).await;
        for (id, state) in [("h-one","untested"),("h-two","untested"),("h-one","refuted"),("h-two","refuted")] {
            let answer = change(&declined_service, &decline_root, "debug-hypothesis", &format!("{id}-{state}"),
                json!({"hypothesis":{"id":id,"description":"candidate cause","rank_reason":"cheap test","state":state}})).await;
            assert_eq!(offers(&answer).len(), usize::from(id == "h-two" && state == "refuted"));
        }
        let refuted = read(&declined_service, &decline_root).await;
        assert_eq!(refuted["record"]["attempt_count"], 0);
        let decline_request = decision(&refuted, "decline", "decline");
        let declined = call(&declined_service, &decline_root, "debug-consult", decline_request.clone()).await;
        assert_eq!(offers(&declined)[0]["state"], "declined");
        assert_eq!(call(&declined_service, &decline_root, "debug-consult", decline_request).await, declined);
        failed(&declined_service, &decline_root, "failed-after-decline").await;
        drop(declined_service);
        assert_eq!(offers(&read(&factory(), &decline_root).await), offers(&declined));
        assert_eq!(wire.requests.lock().unwrap().len(), 2);

        let (_cap_tree, cap_root, capped) = fixture(true, true, Some(1)).await;
        change(&capped, &cap_root, "debug-open", "open", json!({"symptom":"tenant cache leaks entries"})).await;
        failed(&capped, &cap_root, "failed-one").await;
        let offered = failed(&capped, &cap_root, "failed-two").await;
        assert_eq!(offers(&offered).len(), 1, "cap fixture must durably offer consult");
        assert_eq!(offers(&offered)[0]["state"], "offered", "cap is checked on acceptance");
        let accepted = call(&capped, &cap_root, "debug-consult", decision(&offered, "accept-capped", "accept")).await;
        assert_eq!(accepted["status"], "ok", "cap failure must be recorded: {accepted}");
        assert_eq!(offers(&accepted)[0]["state"], "failed", "cap must fail the accepted consult");
        assert!(offers(&accepted)[0]["failure"].as_str().unwrap().contains("provider prompt over cap"), "cap failure must explain the prompt limit");
        assert_eq!(offers(&accepted)[0]["angles"], json!([]), "cap failure must leave angles empty");
        assert!(offers(&accepted)[0]["evidence"].is_null(), "cap failure must leave evidence empty");
        assert_eq!(wire.requests.lock().unwrap().len(), 2, "a capped situation never reaches the provider");
        drop(capped);
        assert_eq!(offers(&read(&factory(), &cap_root).await), offers(&accepted), "cap failure must survive restart");

        // Cancel after the HTTP boundary: durable accepted intent cannot authorize
        // another spend, including a concurrent request or a restarted service.
        let next = change(&service, &root, "debug-observation", "later-evidence",
            json!({"observation":{"test":"inspect keys","result":"key format measured","rules_in":[],"rules_out":[]}})).await;
        let request = decision(&next, "uncertain-accept", "accept");
        wire.hold.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut pending = Box::pin(call(&service, &root, "debug-consult", request.clone()));
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(pending.as_mut(), cx).is_pending());
            if wire.requests.lock().unwrap().len() == 3 { std::task::Poll::Ready(()) }
            else { cx.waker().wake_by_ref(); std::task::Poll::Pending }
        }).await;
        let concurrent = call(&service, &root, "debug-consult", request.clone()).await;
        assert_eq!(concurrent["code"], "debug-consult-pending");
        drop(pending); drop(service); service = factory();
        assert_eq!(call(&service, &root, "debug-consult", request).await["code"], "debug-consult-pending");
        assert_eq!(wire.requests.lock().unwrap().len(), 3);
    }).await;
}
