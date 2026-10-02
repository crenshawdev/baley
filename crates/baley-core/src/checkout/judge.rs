//! The fork judgement: one checkout against the project's `checkout` rows
//! (design 0001 Project identity and policy, ADR 0004).

use std::fmt;

use serde_json::Value;

use super::event::Checkout;

/// The refusal code for a checkout whose remote differs from another
/// checkout's in the same project.
pub const PROJECT_ID_CONFLICT: &str = "project-id-conflict";

/// A checkout that shares a project id with a checkout of another remote,
/// which makes it a fork. Checkout admission records nothing for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdConflict {
    /// The checkout being admitted.
    pub path: String,
    /// Its stripped remote URL.
    pub remote_url: String,
    /// The path of the other checkout.
    pub other_path: String,
    /// The other checkout's stripped remote URL.
    pub other_remote_url: String,
}

impl ProjectIdConflict {
    /// Always `project-id-conflict`.
    pub fn code(&self) -> &'static str {
        PROJECT_ID_CONFLICT
    }
}

impl fmt::Display for ProjectIdConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: the checkout at {} has the remote {}, but the project already has a checkout at {} with the remote {}, so this one is a fork. \
             Run `baley init --new-id` in this checkout to give it its own project",
            self.code(),
            self.path,
            self.remote_url,
            self.other_path,
            self.other_remote_url
        )
    }
}

impl std::error::Error for ProjectIdConflict {}

/// What checkout admission does with a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutVerdict {
    /// Another checkout of the project holds a different remote. Refuse.
    Conflict(ProjectIdConflict),
    /// This path's row already holds this root commit and remote URL.
    Unchanged,
    /// Record one `checkout.seen`: a new path, or a changed root commit or
    /// remote URL.
    Record,
}

fn text<'a>(row: &'a Value, member: &str) -> Option<&'a str> {
    row.get(member).and_then(Value::as_str)
}

/// A member as `CheckoutProjector` writes it: `Some(None)` for `null`,
/// `Some(Some(text))` for text, and `None` for anything else or a missing
/// member.
fn stored<'a>(row: &'a Value, member: &str) -> Option<Option<&'a str>> {
    match row.get(member)? {
        Value::Null => Some(None),
        Value::String(text) => Some(Some(text)),
        _ => None,
    }
}

/// Judges `checkout` against `rows`, the bodies of the project's `checkout`
/// rows.
///
/// A conflict wins over the other two verdicts. It exists when this
/// checkout has a remote URL and the row of another path holds a text URL
/// that is not byte for byte equal to it. URLs are not normalized, so an
/// https clone and an ssh clone of one repository conflict until
/// `git remote set-url` in one of them. This path's own row, a row with no
/// URL and a checkout with no URL never conflict, the root commit plays no
/// part and rows never expire. Of several conflicting rows the path that
/// sorts first is named, so the text does not depend on page order.
///
/// An own row whose members are not as the projector writes them is judged
/// changed, so recording again repairs it.
pub fn judge_checkout(checkout: &Checkout, rows: &[Value]) -> CheckoutVerdict {
    if let Some(remote_url) = &checkout.remote_url {
        let other = rows
            .iter()
            .filter_map(|row| Some((text(row, "path")?, text(row, "remote_url")?)))
            .filter(|(path, url)| *path != checkout.path && *url != remote_url)
            .min_by_key(|(path, _)| *path);
        if let Some((other_path, other_remote_url)) = other {
            return CheckoutVerdict::Conflict(ProjectIdConflict {
                path: checkout.path.clone(),
                remote_url: remote_url.clone(),
                other_path: other_path.into(),
                other_remote_url: other_remote_url.into(),
            });
        }
    }
    let unchanged = rows
        .iter()
        .filter(|row| text(row, "path") == Some(checkout.path.as_str()))
        .any(|row| {
            stored(row, "root_commit") == Some(checkout.root_commit.as_deref())
                && stored(row, "remote_url") == Some(checkout.remote_url.as_deref())
        });
    if unchanged {
        CheckoutVerdict::Unchanged
    } else {
        CheckoutVerdict::Record
    }
}
