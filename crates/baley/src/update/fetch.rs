//! Bounded HTTPS downloads of unsigned development artifacts (design 0012
//! section 5). Only a new claim permits a request; interpreting what came
//! back is separate from gathering it.

use std::fmt;
use std::time::Duration;

use super::claim::FetchPermit;
use super::events::FailureCode;
use super::manifest::Addresses;

const MANIFEST_BOUND: usize = 64 * 1024;
const BINARY_BOUND: usize = 256 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const BINARY_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What one request returned, including any bytes kept before it failed.
#[derive(Debug, PartialEq, Eq)]
pub struct Observation {
    /// The address requested, before any redirects.
    pub address: String,
    /// A transport or setup failure's short cause, or none.
    pub transport_error: Option<String>,
    /// The final response's HTTP status, when headers arrived.
    pub status: Option<u16>,
    /// The body bytes kept, at most `bound` bytes.
    pub body: Vec<u8>,
    /// The maximum body size for this request, in bytes.
    pub bound: usize,
    /// More bytes arrived than the bound permits keeping.
    pub exceeded_bound: bool,
}

/// A download that cannot be passed on to parsing or verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadFailure {
    /// The requested address.
    pub address: String,
    /// What prevented a complete HTTP 200 response within the bound.
    pub cause: String,
}

impl DownloadFailure {
    /// The code recorded for every failed download.
    pub fn code(&self) -> FailureCode {
        FailureCode::NetworkUnavailable
    }
}

impl fmt::Display for DownloadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}: {}",
            self.code().as_str(),
            self.address,
            self.cause
        )
    }
}

impl std::error::Error for DownloadFailure {}

/// Accepts only a complete HTTP 200 body within its bound.
pub fn interpret(observation: Observation) -> Result<Vec<u8>, DownloadFailure> {
    let cause = if let Some(cause) = observation.transport_error {
        cause
    } else if observation.status != Some(200) {
        match observation.status {
            Some(status) => format!("HTTP status {status}"),
            None => "no HTTP response".into(),
        }
    } else if observation.exceeded_bound {
        format!("body exceeded the {} byte bound", observation.bound)
    } else {
        return Ok(observation.body);
    };
    Err(DownloadFailure {
        address: observation.address,
        cause,
    })
}

/// One HTTPS client and current-thread runtime for both files of a check.
/// Call from outside a Tokio runtime, as with detection's blocking entry.
pub struct HttpFetcher {
    transport: Result<(tokio::runtime::Runtime, reqwest::Client), String>,
}

impl HttpFetcher {
    /// Builds the transport once. A setup error becomes a failed observation
    /// on either fetch, with the address that could not be requested.
    pub fn new() -> Self {
        let transport = (|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .map_err(|error| format!("download runtime cannot start: {error}"))?;
            let client = reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(CONNECT_TIMEOUT)
                // No proxy credentials or referring address accompany a download.
                .no_proxy()
                .referer(false)
                .build()
                .map_err(|error| error.without_url().to_string())?;
            Ok((runtime, client))
        })();
        Self { transport }
    }

    /// Fetches the manifest within 20 seconds, keeping at most 64 KiB.
    pub fn manifest(&self, permit: &FetchPermit, addresses: &Addresses) -> Observation {
        self.fetch(
            permit,
            &addresses.manifest,
            MANIFEST_TIMEOUT,
            MANIFEST_BOUND,
        )
    }

    /// Fetches the binary within 10 minutes, keeping at most 256 MiB.
    pub fn binary(&self, permit: &FetchPermit, addresses: &Addresses) -> Observation {
        self.fetch(permit, &addresses.binary, BINARY_TIMEOUT, BINARY_BOUND)
    }

    fn fetch(
        &self,
        _permit: &FetchPermit,
        address: &str,
        timeout: Duration,
        bound: usize,
    ) -> Observation {
        let mut observation = Observation {
            address: address.into(),
            transport_error: None,
            status: None,
            body: Vec::new(),
            bound,
            exceeded_bound: false,
        };
        let (runtime, client) = match &self.transport {
            Ok(transport) => transport,
            Err(cause) => {
                observation.transport_error = Some(cause.clone());
                return observation;
            }
        };
        runtime.block_on(async {
            let mut request = match client.get(address).timeout(timeout).build() {
                Ok(request) => request,
                Err(error) => {
                    observation.transport_error = Some(error.without_url().to_string());
                    return observation;
                }
            };
            // Reqwest turns URL userinfo into Basic auth. Updates carry none.
            request.headers_mut().remove(reqwest::header::AUTHORIZATION);
            let mut response = match client.execute(request).await {
                Ok(response) => response,
                Err(error) => {
                    observation.transport_error = Some(error.without_url().to_string());
                    return observation;
                }
            };
            observation.status = Some(response.status().as_u16());
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        let remaining = bound - observation.body.len();
                        observation
                            .body
                            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                        if chunk.len() > remaining {
                            observation.exceeded_bound = true;
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        observation.transport_error = Some(error.without_url().to_string());
                        break;
                    }
                }
            }
            observation
        })
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_or_oversized_response_taken_as_a_download_is_caught() {
        let address = "https://dl.example/dev/linux-x86_64/baley";
        let complete = || Observation {
            address: address.into(),
            transport_error: None,
            status: Some(200),
            body: b"abc".to_vec(),
            bound: 268_435_456,
            exceeded_bound: false,
        };
        assert_eq!(interpret(complete()).unwrap(), b"abc");

        for (observation, cause) in [
            (
                Observation {
                    status: Some(404),
                    ..complete()
                },
                "HTTP status 404",
            ),
            (
                Observation {
                    status: Some(302),
                    ..complete()
                },
                "HTTP status 302",
            ),
            (
                Observation {
                    exceeded_bound: true,
                    ..complete()
                },
                "body exceeded the 268435456 byte bound",
            ),
            (
                Observation {
                    transport_error: Some("connection refused".into()),
                    status: None,
                    body: Vec::new(),
                    ..complete()
                },
                "connection refused",
            ),
        ] {
            let failure = interpret(observation).expect_err(cause);
            assert_eq!(failure.code().as_str(), "update-network-unavailable");
            assert_eq!(failure.address, address);
            assert_eq!(failure.cause, cause);
            let text = failure.to_string();
            assert!(text.contains(address), "{text}");
            assert!(text.contains(cause), "{text}");
            assert!(text.contains("update-network-unavailable"), "{text}");
        }
    }
}
