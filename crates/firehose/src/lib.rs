//! Firehose: Mastodon WSS client + strict 1024B lossy bounded mpsc (§2.2).
//!
//! Implements M2 of `PRD.md`:
//!   - WSS connect to a chaos-heavy instance; **`mastodon.social` is rejected
//!     at config validation** (§2.1).
//!   - Strict 1024-byte bounded `mpsc` between the socket reader and the FSM
//!     consumer; on overflow we **drop** bytes (no backpressure). This is the
//!     Avalanche source.
//!   - HTTP 429 does **not** retry; it flips [`Health`] to [`Health::Starved`]
//!     so the FSM falls back to bus-only feeding.
//!   - [`Health`] is a first-class strategy toggle (§5), not an implicit
//!     fallback.
//!
//! Per §4 the firehose layer must not depend on `serde_json`, `regex`, or any
//! tokenizer/parser. The WSS payload is treated as an opaque byte stream and
//! handed to the Ephemeral Layer one byte at a time.

use futures_util::StreamExt;
use std::fmt;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tracing::{error, info, warn};

/// Strict bounded buffer between the socket reader and the FSM consumer.
/// One slot == one byte, so capacity is literally 1024 bytes (§2.2).
pub const BUFFER_BYTES: usize = 1024;

/// Forbidden by §2.1 — its over-filtered firehose lacks irritation density.
pub const FORBIDDEN_INSTANCE: &str = "mastodon.social";

/// Health of the firehose, consumed by the kernel as an explicit strategy
/// toggle. The two feeding modes are first-class branches in the FSM loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Socket is up; FSM should feed on raw Mastodon bytes.
    FirehoseFeeding,
    /// Socket is down (non-429); FSM should feed parasitically on the
    /// Societal Bus until the firehose recovers (§5).
    BusFeeding,
    /// HTTP 429 starvation event (§2.1). No retry — the FSM is forced into
    /// pure bus feeding for the remainder of its life.
    Starved,
}

#[derive(Debug)]
pub enum ConfigError {
    ForbiddenInstance(String),
    EmptyInstance,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::ForbiddenInstance(h) => write!(
                f,
                "instance `{h}` is forbidden by §2.1 (over-filtered firehose lacks irritation density)"
            ),
            ConfigError::EmptyInstance => write!(f, "empty instance host"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Validated firehose configuration.
#[derive(Debug, Clone)]
pub struct FirehoseConfig {
    pub instance: String,
}

impl FirehoseConfig {
    /// Validate an instance host. Strips any scheme/trailing slash and
    /// rejects [`FORBIDDEN_INSTANCE`].
    pub fn validate(instance: impl Into<String>) -> Result<Self, ConfigError> {
        let raw = instance.into();
        let host = raw
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("wss://")
            .trim_start_matches("ws://")
            .trim_end_matches('/');
        if host.is_empty() {
            return Err(ConfigError::EmptyInstance);
        }
        if host.eq_ignore_ascii_case(FORBIDDEN_INSTANCE) {
            return Err(ConfigError::ForbiddenInstance(host.to_string()));
        }
        Ok(FirehoseConfig {
            instance: host.to_string(),
        })
    }
}

/// Handle returned by [`spawn`]: a single-consumer byte stream plus a watch
/// channel carrying the current [`Health`].
pub struct FirehoseHandle {
    /// Strict 1024B lossy mpsc. The kernel is the sole consumer.
    pub bytes: mpsc::Receiver<u8>,
    /// First-class health toggle. Cloneable to all FSMs.
    pub health: watch::Receiver<Health>,
}

/// Spawn the firehose ingestion task and return its handle.
pub fn spawn(cfg: FirehoseConfig) -> FirehoseHandle {
    let (byte_tx, byte_rx) = mpsc::channel::<u8>(BUFFER_BYTES);
    let (health_tx, health_rx) = watch::channel(Health::FirehoseFeeding);
    tokio::spawn(run(cfg, byte_tx, health_tx));
    FirehoseHandle {
        bytes: byte_rx,
        health: health_rx,
    }
}

async fn run(
    cfg: FirehoseConfig,
    byte_tx: mpsc::Sender<u8>,
    health_tx: watch::Sender<Health>,
) {
    let url = format!("wss://{}/api/v1/streaming?stream=public", cfg.instance);
    info!(url = %url, "firehose connecting");

    match tokio_tungstenite::connect_async(&url).await {
        Ok((mut ws, _resp)) => {
            let _ = health_tx.send(Health::FirehoseFeeding);
            info!("firehose connected → FirehoseFeeding");
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(Message::Text(t)) => push_lossy(&byte_tx, t.as_bytes()),
                    Ok(Message::Binary(b)) => push_lossy(&byte_tx, &b),
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(frame)) => {
                        warn!(?frame, "firehose closed by remote");
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error!(error = %e, "firehose stream error");
                        break;
                    }
                }
            }
            let _ = health_tx.send(Health::BusFeeding);
            warn!("firehose stream ended → BusFeeding");
        }
        Err(WsError::Http(resp)) if resp.status() == 429 => {
            warn!("firehose 429 → Starved (no retry, §2.1)");
            let _ = health_tx.send(Health::Starved);
        }
        Err(e) => {
            error!(error = %e, "firehose connect failed → BusFeeding");
            let _ = health_tx.send(Health::BusFeeding);
        }
    }
}

/// Push bytes into the bounded mpsc, **dropping** any byte that does not fit.
/// This drop is intentional — it is the Avalanche source (§2.2).
fn push_lossy(tx: &mpsc::Sender<u8>, data: &[u8]) {
    let mut dropped = 0usize;
    for &b in data {
        if tx.try_send(b).is_err() {
            dropped += 1;
        }
    }
    if dropped > 0 {
        // Avalanche jitter — log at trace-ish level. `warn!` is fine for v0.
        warn!(dropped, "firehose buffer overflow → bytes dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mastodon_social() {
        assert!(matches!(
            FirehoseConfig::validate("mastodon.social"),
            Err(ConfigError::ForbiddenInstance(_))
        ));
        assert!(matches!(
            FirehoseConfig::validate("https://Mastodon.Social/"),
            Err(ConfigError::ForbiddenInstance(_))
        ));
    }

    #[test]
    fn accepts_pawoo() {
        let cfg = FirehoseConfig::validate("pawoo.net").unwrap();
        assert_eq!(cfg.instance, "pawoo.net");
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(
            FirehoseConfig::validate(""),
            Err(ConfigError::EmptyInstance)
        ));
    }
}
