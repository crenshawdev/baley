//! The remembered policy (design 0010, GRD-R7): the denials of the last
//! complete policy, kept per session project, target checkout and host so a
//! torn settings file cannot open the door. The `guard_policy` view holds
//! them, and the judges here say what is kept, when it changed and what the
//! commit judge is handed under torn settings.

use baley_store::{
    Change, DocKey, Event, FieldKind, FieldSpec, KeyValue, Projector, ProjectorError, ViewSpec,
};
use serde_json::{Map, Value, json};

use super::event::GUARD_POLICY_RECORDED;
use super::settings::GuardSettings;
use crate::policy::OnProtected;

/// The view's name.
pub const GUARD_POLICY_VIEW: &str = "guard_policy";

/// The first key field: the session project's canonical repository root.
const PROJECT_FIELD: &str = "project_root";
/// The second key field: the target checkout's canonical root, `""` when
/// the commit target is in no checkout. It cannot be `checkout_root`, which the
/// payload holds as `null` then.
const CHECKOUT_KEY_FIELD: &str = "checkout_key";
/// The third key field: the host's name.
const HOST_FIELD: &str = "host";

/// What a complete policy leaves behind for torn settings: its denials and
/// nothing that allows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenialParts {
    /// Whether `git.on_protected` was `refuse`.
    pub refuse: bool,
    /// Whether `git.guard_hard_fail` was on.
    pub hard_fail: bool,
    /// `git.protected_branches`, kept only when a denial is on, since only a
    /// denial needs to prove a branch protected. Empty otherwise.
    pub protected_branches: Vec<String>,
}

/// What `settings` leaves behind: whether `on_protected` is `refuse`,
/// whether hard fail is on, and the protected list when either is.
///
/// An `on_protected` of `allow` or `ask` is kept as not `refuse`, never as
/// itself, so nothing remembered can allow. The list stays with hard fail
/// as well as with `refuse`, because a remembered hard fail denies only a
/// branch it can prove protected, and the list is that proof.
pub fn denial_parts(settings: &GuardSettings) -> DenialParts {
    let refuse = settings.on_protected == OnProtected::Refuse;
    let protected_branches = if refuse || settings.hard_fail {
        settings.protected_branches.clone()
    } else {
        Vec::new()
    };
    DenialParts {
        refuse,
        hard_fail: settings.hard_fail,
        protected_branches,
    }
}

/// Whether `current` must be recorded as a `guard.policy_recorded` over
/// `stored`, the `guard_policy` document under its key, if any.
///
/// No document remembers no denial, so parts with no denial and no document
/// are the same and nothing is appended. A newer policy with no denial over
/// a remembered one is a change, so an older `refuse` is cleared rather than
/// kept. A document that cannot be read is replaced.
pub fn denials_changed(stored: Option<&Value>, current: &DenialParts) -> bool {
    let remembered = match stored {
        None => Some(DenialParts::default()),
        Some(document) => stored_parts(document),
    };
    remembered.as_ref() != Some(current)
}

/// The settings a stored `guard_policy` document hands the commit judge as
/// `remembered`, or none when the document cannot be read. `refuse` becomes
/// `OnProtected::Refuse` and anything else `OnProtected::Ask`, never
/// `Allow`. The judge uses them only under torn settings and only to deny.
pub fn remembered_settings(stored: &Value) -> Option<GuardSettings> {
    let parts = stored_parts(stored)?;
    Some(GuardSettings {
        protected_branches: parts.protected_branches,
        on_protected: if parts.refuse {
            OnProtected::Refuse
        } else {
            OnProtected::Ask
        },
        hard_fail: parts.hard_fail,
    })
}

/// The denials a payload or document holds, if they can be read.
fn stored_parts(value: &Value) -> Option<DenialParts> {
    let flag = |name: &str| value.get(name).and_then(Value::as_bool);
    let protected_branches = value
        .get("protected_branches")?
        .as_array()?
        .iter()
        .map(|name| name.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(DenialParts {
        refuse: flag("refuse")?,
        hard_fail: flag("hard_fail")?,
        protected_branches,
    })
}

/// The view's declaration: the session project root, the target checkout
/// root and the host as text keys, no indexes, and page bound 1, since every
/// read is a `get` by key.
pub fn guard_policy_spec() -> ViewSpec {
    let text = |name: &str| FieldSpec {
        name: name.into(),
        kind: FieldKind::Text,
    };
    ViewSpec {
        name: GUARD_POLICY_VIEW.into(),
        version: 1,
        key: vec![
            text(PROJECT_FIELD),
            text(CHECKOUT_KEY_FIELD),
            text(HOST_FIELD),
        ],
        indexes: Vec::new(),
        page_bound: 1,
    }
}

/// The key of the denials remembered for one session project, target
/// checkout and host. No checkout keys under `""`, which no canonical root
/// can be.
pub fn guard_policy_key(project_root: &str, checkout_root: Option<&str>, host: &str) -> DocKey {
    DocKey(vec![
        KeyValue::Text(project_root.into()),
        KeyValue::Text(checkout_root.unwrap_or_default().into()),
        KeyValue::Text(host.into()),
    ])
}

/// Keeps the `guard_policy` view current from `guard.policy_recorded`: the
/// latest event for a key replaces its document whole, so a newer policy
/// with no denial clears an older one.
pub struct GuardPolicyProjector {
    spec: ViewSpec,
}

impl GuardPolicyProjector {
    /// Makes the projector.
    pub fn new() -> Self {
        Self {
            spec: guard_policy_spec(),
        }
    }
}

impl Default for GuardPolicyProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl Projector for GuardPolicyProjector {
    fn spec(&self) -> &ViewSpec {
        &self.spec
    }

    fn handles(&self) -> &[&str] {
        &[GUARD_POLICY_RECORDED]
    }

    // The latest event wins, so no stored document is needed.
    fn keys(&self, _event: &Event) -> Vec<DocKey> {
        Vec::new()
    }

    fn apply(
        &self,
        event: &Event,
        _documents: &[(DocKey, Value)],
    ) -> Result<Vec<Change>, ProjectorError> {
        let refuse = |message: &str| {
            ProjectorError(format!(
                "{} at seq {}: {message}",
                event.type_name, event.seq
            ))
        };
        let Value::Object(payload) = &event.payload else {
            return Err(refuse("the payload is not an object"));
        };
        let text = |member: &str| match payload.get(member) {
            Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
            _ => None,
        };
        let (Some(project), Some(host)) = (text(PROJECT_FIELD), text(HOST_FIELD)) else {
            return Err(refuse("project_root or host is missing, not text or empty"));
        };
        // A `""` checkout would take the key of a target in no checkout.
        let checkout = match payload.get("checkout_root") {
            Some(Value::Null) => None,
            Some(Value::String(checkout)) if !checkout.is_empty() => Some(checkout.as_str()),
            _ => return Err(refuse("checkout_root is neither null nor non-empty text")),
        };
        let Some(parts) = stored_parts(&event.payload) else {
            return Err(refuse(
                "refuse, hard_fail or protected_branches is missing or of the wrong type",
            ));
        };
        let mut body = Map::new();
        body.insert(PROJECT_FIELD.into(), json!(project));
        body.insert(
            CHECKOUT_KEY_FIELD.into(),
            json!(checkout.unwrap_or_default()),
        );
        body.insert(HOST_FIELD.into(), json!(host));
        body.insert("refuse".into(), json!(parts.refuse));
        body.insert("hard_fail".into(), json!(parts.hard_fail));
        body.insert("protected_branches".into(), json!(parts.protected_branches));
        body.insert("seq".into(), json!(event.seq));
        Ok(vec![Change::Put {
            key: guard_policy_key(project, checkout, host),
            body: Value::Object(body),
        }])
    }
}
