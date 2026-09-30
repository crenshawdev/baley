//! Gathers one provider's model listing with its key, in one request. It
//! judges nothing itself.

use std::future::Future;
use std::time::Duration;

use baley_core::catalog::Provider;
use baley_core::catalog::detection::{Observation, ObservedResponse};

use crate::keys::{Key, list_request};

/// The most body bytes kept per response. Detection's own bound, not the
/// review transport's.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// How long one request may take, body included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Lists one provider's models with its key.
pub trait ModelLister {
    /// What one provider's list endpoint answered to one request.
    fn list(&self, provider: Provider, key: &Key) -> impl Future<Output = Observation>;
}

/// The lister that reaches each provider over HTTPS.
pub struct HttpLister {
    // `None` when the client could not be built, which fails every call.
    client: Option<reqwest::Client>,
}
impl HttpLister {
    /// One client for every provider: https only, no redirect followed, and
    /// a 20-second bound per request.
    pub fn new() -> HttpLister {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .ok();
        HttpLister { client }
    }
}
impl Default for HttpLister {
    fn default() -> Self {
        HttpLister::new()
    }
}

impl ModelLister for HttpLister {
    async fn list(&self, provider: Provider, key: &Key) -> Observation {
        let mut observation = Observation::default();
        let Some(client) = &self.client else {
            observation.transport_failed = true;
            return observation;
        };
        // No error text is kept: a transport error can quote the request.
        let Ok(request) = list_request(provider, key) else {
            observation.transport_failed = true;
            return observation;
        };
        let Ok(mut response) = client.execute(request).await else {
            observation.transport_failed = true;
            return observation;
        };
        let status = response.status().as_u16();
        let mut body = Vec::new();
        let mut cut_short = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if body.len() + chunk.len() > MAX_BODY_BYTES => {
                    cut_short = true;
                    break;
                }
                Ok(Some(chunk)) => body.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(_) => {
                    observation.transport_failed = true;
                    return observation;
                }
            }
        }
        observation.responses.push(ObservedResponse {
            status,
            body,
            cut_short,
        });
        observation
    }
}
