//! Back-reference graph resolution by modal-base voting.
//!
//! Component-definition geometry numbers its entities with *local* map indices;
//! top-level geometry uses the *global* archive map. Each (back-ref,
//! expected-type) pair votes for every base that would land it on an object of
//! that type; the modal base wins (robust to resync-perturbed constraints, and
//! recovers whichever base a run actually uses). Real geometry is
//! over-constrained, so the base is unique.
//!
//! Port of `_resolve_refs` in `tools/skpwalk.py`.

use std::collections::HashMap;

use crate::carchive::{Child, Slot};
use crate::entity::Entity;

/// The outcome of back-reference resolution for one run.
pub struct Resolution {
    pub satisfied: usize,
    pub constraints: usize,
    /// The winning base offset: a back-ref `r` targets `map[r - base + 1]`
    /// (the resolver's 0-based pool slice is `map[1..]`). `None` when the
    /// run had nothing to vote with.
    pub base: Option<i64>,
}

/// Back-reference constraints for a run.
///
/// `map` is the archive store map (index 0 = sentinel; objects at `1..`);
/// `objs` are the top-level objects. Constraints are collected by traversing the
/// object tree from those roots (edge endpoints → `CVertex`, edge-use → `CEdge`,
/// edge-use parent → `CLoop`) — crucially *not* descending into an edge-use's
/// edge, so edges reachable only via a back-ref contribute no constraints
/// (matching the Python reference).
pub fn resolve(map: &[Slot], objs: &[Child]) -> Resolution {
    let pool = map;
    let mut cons: Vec<(usize, &'static str)> = Vec::new();
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut stack: Vec<usize> = objs
        .iter()
        .filter_map(|c| {
            if let Child::Obj(i) = c {
                Some(*i)
            } else {
                None
            }
        })
        .collect();
    while let Some(idx) = stack.pop() {
        if !seen.insert(idx) {
            continue;
        }
        let e = match map.get(idx) {
            Some(Slot::Object(e)) => e,
            _ => continue,
        };
        match e {
            Entity::Edge { v0, v1, curve, .. } => {
                for c in [v0, v1] {
                    if let Child::Ref(i) = c {
                        cons.push((*i, "CVertex"));
                    }
                }
                // descend into inline endpoints/curve (dicts), but leave
                // edge.curve back-refs unconstrained (a curve groups many edges).
                for c in [v0, v1, curve] {
                    if let Child::Obj(i) = c {
                        stack.push(*i);
                    }
                }
            }
            Entity::Face { loops, .. } => {
                for c in loops {
                    if let Child::Obj(i) = c {
                        stack.push(*i);
                    }
                }
            }
            Entity::Loop { edge_uses } => {
                for c in edge_uses {
                    if let Child::Obj(i) = c {
                        stack.push(*i);
                    }
                }
            }
            Entity::EdgeUse { edge, parent, .. } => {
                if let Child::Ref(i) = edge {
                    cons.push((*i, "CEdge"));
                }
                if let Child::Ref(i) = parent {
                    cons.push((*i, "CLoop"));
                }
            }
            _ => {}
        }
    }

    // pool positions by class (0-based over map[1..], matching the Python slice).
    let slice = &pool[1.min(pool.len())..];
    let mut pos_by_cls: HashMap<&str, Vec<usize>> = HashMap::new();
    for (k, slot) in slice.iter().enumerate() {
        if let Slot::Object(e) = slot {
            pos_by_cls.entry(e.class_name()).or_default().push(k);
        }
    }

    // Vote for the base offset. Counter.most_common(1) breaks ties by first
    // insertion; we replicate by tracking first-seen order.
    //
    // Voting is O(voters × pool positions of the voted class) — quadratic on
    // a ~70k-entity run (pid-stress: 138k constraints × 21k vertices ≈ 3
    // billion bumps, ~50 s). Cap the VOTERS at an evenly-spaced sample: the
    // base is decided by the mode, which a few thousand voters pin just as
    // hard (even spacing keeps voters from all sitting in one resync-damaged
    // stretch), while `satisfied` below still verifies EVERY constraint.
    // Runs at or under the cap — the whole corpus except pid-stress — vote
    // with all constraints in insertion order, bit-identical to before.
    const MAX_VOTERS: usize = 4096;
    let stride = cons.len().div_ceil(MAX_VOTERS).max(1);
    let mut votes: HashMap<i64, u32> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    for (r, t) in cons.iter().step_by(stride) {
        if let Some(ks) = pos_by_cls.get(t) {
            for &k in ks {
                let base = *r as i64 - k as i64;
                let e = votes.entry(base).or_insert_with(|| {
                    order.push(base);
                    0
                });
                *e += 1;
            }
        }
    }
    // pick max votes; ties resolve to the earliest-inserted base (Counter parity).
    let base = if order.is_empty() {
        None
    } else {
        let mut best = order[0];
        for &b in &order[1..] {
            if votes[&b] > votes[&best] {
                best = b;
            }
        }
        Some(best)
    };

    let mut satisfied = 0usize;
    if let Some(base) = base {
        for (r, t) in &cons {
            let j = *r as i64 - base;
            if j >= 0 && (j as usize) < slice.len() {
                if let Slot::Object(e) = &slice[j as usize] {
                    if e.class_name() == *t {
                        satisfied += 1;
                    }
                }
            }
        }
    }
    Resolution {
        satisfied,
        constraints: cons.len(),
        base,
    }
}
