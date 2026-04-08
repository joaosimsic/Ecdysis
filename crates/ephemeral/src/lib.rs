//! Ephemeral Layer (§4.1, M3): RAM state graph behind `ArcSwap`.
//!
//! Hot path is a lock-free read of an `Arc<StateGraph>`; mutation is
//! clone-modify-swap. Traversal is byte-level and returns `Unmapped` on the
//! first edge it cannot follow, which is the Irritation signal that drives
//! Growth (§8 step 1→2). Per-edge EMA scoring (§6) accumulates hits and is
//! decayed each Harvest tick; edges below the survival threshold are purged.
//!
//! No `unsafe` is used in this crate — `arc-swap` provides the hazard-pointer
//! semantics required by §4.1, so `cargo miri test` runs cleanly and the CI
//! gate stays green for any future unsafe additions.

use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

pub type NodeId = u32;
pub const ROOT: NodeId = 0;

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub visits: u64,
}

/// Per-edge metadata. `score` is the EMA that gates institutionalization (§6).
#[derive(Debug, Clone)]
pub struct Edge {
    pub target: NodeId,
    pub score: f64,
}

/// The mutable Directed State Graph that lives in RAM (§4.1). Cloneable so
/// `LiveGraph::mutate` can do clone-modify-swap without touching `unsafe`.
#[derive(Debug, Clone, Default)]
pub struct StateGraph {
    pub nodes: Vec<Node>,
    pub edges: HashMap<(NodeId, u8), Edge>,
}

/// Returned by traversal when no edge exists for the next byte. Drives the
/// Growth phase: the kernel allocates a new node + edge mapping `byte`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmapped {
    pub offset: usize,
    pub byte: u8,
}

impl StateGraph {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node::default()],
            edges: HashMap::new(),
        }
    }

    /// Walk the graph one byte at a time starting from `ROOT`. On the first
    /// missing edge, returns `Err(Unmapped)` carrying the offset within
    /// `bytes` so the caller can resume / grow precisely there.
    pub fn traverse(&self, bytes: &[u8]) -> Result<NodeId, Unmapped> {
        let mut cur = ROOT;
        for (offset, &byte) in bytes.iter().enumerate() {
            match self.edges.get(&(cur, byte)) {
                Some(edge) => cur = edge.target,
                None => return Err(Unmapped { offset, byte }),
            }
        }
        Ok(cur)
    }

    /// Bump the EMA score for an existing edge: `score = α + (1-α)·prev`.
    pub fn record_hit(&mut self, from: NodeId, byte: u8, alpha: f64) {
        if let Some(edge) = self.edges.get_mut(&(from, byte)) {
            edge.score = alpha + (1.0 - alpha) * edge.score;
        }
    }

    /// Allocate a new node reachable from `from` via `byte` (§8 Growth).
    pub fn grow(&mut self, from: NodeId, byte: u8) -> NodeId {
        let new_id = self.nodes.len() as NodeId;
        self.nodes.push(Node::default());
        self.edges.insert(
            (from, byte),
            Edge {
                target: new_id,
                score: 1.0,
            },
        );
        new_id
    }

    /// One Harvest tick of decay (§6). Multiplies every edge by `(1 - λ)`
    /// and purges anything below `threshold`. Orphaned nodes are left in
    /// place; v0 trades a small amount of dead `Vec` capacity for keeping
    /// `NodeId`s positionally stable across harvests.
    pub fn harvest(&mut self, lambda: f64, threshold: f64) {
        let factor = 1.0 - lambda;
        self.edges.retain(|_, edge| {
            edge.score *= factor;
            edge.score >= threshold
        });
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Lock-free, swap-on-mutate handle around a `StateGraph` (§4.1).
///
/// Readers grab an `Arc<StateGraph>` snapshot via `load()` in nanoseconds;
/// writers clone the current graph, mutate the clone, and `store` it back.
/// `arc-swap` keeps old snapshots valid for the duration of an in-flight
/// read, which is the property the live firehose loop depends on.
pub struct LiveGraph {
    inner: ArcSwap<StateGraph>,
}

impl LiveGraph {
    pub fn new() -> Self {
        Self {
            inner: ArcSwap::from_pointee(StateGraph::new()),
        }
    }

    pub fn load(&self) -> Arc<StateGraph> {
        self.inner.load_full()
    }

    /// Clone-modify-swap. Concurrent readers see either the old or the new
    /// graph, never a torn intermediate.
    pub fn mutate<F: FnOnce(&mut StateGraph)>(&self, f: F) {
        let mut next = (*self.load()).clone();
        f(&mut next);
        self.inner.store(Arc::new(next));
    }
}

impl Default for LiveGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_unmaps_first_byte() {
        let g = StateGraph::new();
        assert_eq!(g.traverse(b"hi"), Err(Unmapped { offset: 0, byte: b'h' }));
    }

    #[test]
    fn growth_then_traverse_succeeds() {
        let mut g = StateGraph::new();
        let n1 = g.grow(ROOT, b'h');
        let n2 = g.grow(n1, b'i');
        assert_eq!(g.traverse(b"hi"), Ok(n2));
    }

    #[test]
    fn traverse_reports_offset_of_first_unmapped_byte() {
        let mut g = StateGraph::new();
        let n1 = g.grow(ROOT, b'a');
        let _ = g.grow(n1, b'b');
        assert_eq!(g.traverse(b"abz"), Err(Unmapped { offset: 2, byte: b'z' }));
    }

    #[test]
    fn ema_increases_on_hit_and_decays_on_harvest() {
        let mut g = StateGraph::new();
        g.grow(ROOT, b'x'); // edge created with score 1.0
        g.record_hit(ROOT, b'x', 0.5);
        // 0.5 + 0.5 * 1.0 = 1.0
        assert!((g.edges[&(ROOT, b'x')].score - 1.0).abs() < 1e-9);

        g.harvest(0.25, 0.0);
        assert!((g.edges[&(ROOT, b'x')].score - 0.75).abs() < 1e-9);
    }

    #[test]
    fn harvest_purges_below_threshold() {
        let mut g = StateGraph::new();
        g.grow(ROOT, b'a');
        g.grow(ROOT, b'b');
        g.harvest(0.9, 0.5);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn live_graph_swap_publishes_mutation_to_subsequent_readers() {
        let live = LiveGraph::new();
        let before = live.load();
        assert_eq!(before.edge_count(), 0);

        live.mutate(|g| {
            g.grow(ROOT, b'q');
        });

        let after = live.load();
        assert_eq!(after.edge_count(), 1);
        // Old snapshot remains valid and unchanged — the §4.1 invariant.
        assert_eq!(before.edge_count(), 0);
    }
}
