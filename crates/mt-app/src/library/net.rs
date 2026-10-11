//! HTTP for the model library: `ureq` (blocking, pure-Rust TLS with `rustls`) on
//! background threads only, never on the UI thread.
//!
//! - Every request carries [`USER_AGENT`] (Poly Haven's API terms ask for a user
//!   agent naming the software; Sketchfab's do not, it does no harm there).
//! - Proxies come from `HTTPS_PROXY` / `ALL_PROXY`; certificates from the system
//!   (`rustls-platform-verifier`; on Linux the system store or `SSL_CERT_FILE`).
//! - API requests go one at a time through [`Throttle`] (Poly Haven and Sketchfab
//!   publish no numbers; one request per [`MIN_INTERVAL`] keeps well clear of
//!   "degrading the performance of the API", Poly Haven ToS 2.6).

use std::io::Read;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Sent with every request, as the Poly Haven API terms (2.4) ask.
pub(crate) const USER_AGENT: &str = concat!(
    "Meshtard/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/noxqs/meshtard)"
);

/// Shortest time between two requests to an API (not to its file CDN).
pub(crate) const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// Largest JSON answer read (Poly Haven's full model list is about 1 MB).
const MAX_JSON: u64 = 32 << 20;
/// Largest file downloaded (a model, a texture or a thumbnail).
pub(crate) const MAX_FILE: u64 = 512 << 20;

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

/// Keeps API requests at least [`MIN_INTERVAL`] apart (shared by the worker threads).
pub(crate) struct Throttle {
    last: Mutex<Option<Instant>>,
}

impl Throttle {
    pub(crate) const fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    pub(crate) fn wait(&self) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < MIN_INTERVAL {
                std::thread::sleep(MIN_INTERVAL - since);
            }
        }
        *last = Some(Instant::now());
    }
}

pub(crate) static API_THROTTLE: Throttle = Throttle::new();

/// Downloaded and total bytes of a running download, and its Cancel button.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    /// Fraction done (0 when the size is unknown).
    pub(crate) fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

/// GETs an API URL (throttled) and returns its body as text.
pub(crate) fn get_json(url: &str, token: Option<&str>) -> Result<String, String> {
    API_THROTTLE.wait();
    let mut req = agent().get(url);
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Token {t}"));
    }
    let mut resp = req.call().map_err(|e| describe(url, &e))?;
    resp.body_mut()
        .with_config()
        .limit(MAX_JSON)
        .read_to_string()
        .map_err(|e| describe(url, &e))
}

/// GETs a file into memory, adding to `progress` as it comes in.
pub(crate) fn get_bytes(url: &str, progress: Option<&Progress>) -> Result<Vec<u8>, String> {
    let mut resp = agent().get(url).call().map_err(|e| describe(url, &e))?;
    let mut reader = resp.body_mut().with_config().limit(MAX_FILE).reader();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if progress.is_some_and(|p| p.cancel.load(Ordering::Relaxed)) {
            return Err("Download canceled.".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if let Some(p) = progress {
            p.done.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    Ok(out)
}

fn describe(url: &str, e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(401 | 403) => {
            format!("{url}: not allowed (HTTP {e}); check the API token.")
        }
        _ => format!("{url}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_the_app_and_project() {
        assert!(USER_AGENT.starts_with("Meshtard/"));
        assert!(USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
        assert!(USER_AGENT.ends_with("(+https://github.com/noxqs/meshtard)"));
    }

    #[test]
    fn throttle_spaces_requests() {
        let t = Throttle::new();
        let start = Instant::now();
        t.wait();
        t.wait();
        assert!(start.elapsed() >= MIN_INTERVAL);
    }

    #[test]
    fn progress_fraction() {
        let p = Progress::default();
        assert_eq!(p.fraction(), 0.0);
        p.total.store(200, Ordering::Relaxed);
        p.done.store(50, Ordering::Relaxed);
        assert_eq!(p.fraction(), 0.25);
    }
}
