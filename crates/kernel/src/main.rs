//! Kernel: orchestrates N FSM tasks sharing one Societal Bus (§2.1).
//! M1 scope: skeleton only — dummy FSM tasks log heartbeats over the bus.

use tokio::sync::broadcast;
use tracing::{info, warn};

const N_FSMS: usize = 4;
const BUS_CAPACITY: usize = 1024;

/// A frame on the Societal Bus. In v0 these are raw byte excretions (§5).
#[derive(Clone, Debug)]
pub struct BusFrame {
    pub origin: usize,
    pub bytes: Vec<u8>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("kernel boot: spawning {N_FSMS} FSM tasks");

    let (tx, _rx) = broadcast::channel::<BusFrame>(BUS_CAPACITY);

    let mut handles = Vec::with_capacity(N_FSMS);
    for id in 0..N_FSMS {
        let tx = tx.clone();
        let rx = tx.subscribe();
        handles.push(tokio::spawn(dummy_fsm(id, tx, rx)));
    }
    drop(tx);

    for h in handles {
        let _ = h.await;
    }
}

/// M1 placeholder FSM. Real ingestion + ephemeral graph land in M2/M3.
async fn dummy_fsm(
    id: usize,
    tx: broadcast::Sender<BusFrame>,
    mut rx: broadcast::Receiver<BusFrame>,
) {
    info!(fsm = id, "online");
    let mut tick = 0u64;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                tick += 1;
                let frame = BusFrame { origin: id, bytes: vec![tick as u8] };
                if tx.send(frame).is_err() {
                    warn!(fsm = id, "bus has no receivers");
                }
                if tick >= 4 { break; }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(frame) if frame.origin != id => {
                        info!(fsm = id, peer = frame.origin, n = frame.bytes.len(), "bus rx");
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(fsm = id, dropped = n, "bus lag (avalanche jitter)");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    info!(fsm = id, "offline");
}
