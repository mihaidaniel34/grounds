//! Generic monotone dataflow framework over [`Cfg`]: forward or backward,
//! with per-element transfer functions and edge refinement for conditions.

use crate::cfg::{BlockId, Cfg, EdgeKind};
use grounds_syntax::Node;
use std::collections::VecDeque;

pub trait Analysis<'t> {
    type Fact: Clone + PartialEq;
    const FORWARD: bool = true;
    /// Fact for blocks not yet reached (identity for join).
    fn bottom(&self) -> Self::Fact;
    /// Fact at the boundary (entry for forward, exit for backward).
    fn boundary(&self) -> Self::Fact;
    fn join(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact;
    /// Apply the effect of one element.
    fn transfer(&self, elem: Node<'t>, fact: &mut Self::Fact);
    /// Refine a fact flowing along an edge (forward analyses only).
    fn edge(&self, _kind: &EdgeKind<'t>, fact: &Self::Fact) -> Option<Self::Fact> {
        Some(fact.clone())
    }
}

pub struct Results<F> {
    /// Fact at block entry (forward) / block exit (backward: fact *after*
    /// the block in program order is `out`).
    pub block_in: Vec<F>,
    pub block_out: Vec<F>,
    /// Whether the block was reached by propagation.
    pub reached: Vec<bool>,
}

pub fn solve<'t, A: Analysis<'t>>(cfg: &Cfg<'t>, a: &A) -> Results<A::Fact> {
    let n = cfg.blocks.len();
    let mut inp: Vec<A::Fact> = vec![a.bottom(); n];
    let mut out: Vec<A::Fact> = vec![a.bottom(); n];
    let mut reached = vec![false; n];
    let mut queue: VecDeque<BlockId> = VecDeque::new();
    let mut queued = vec![false; n];
    let budget = n * 50 + 1000;
    let mut steps = 0;

    if A::FORWARD {
        inp[cfg.entry] = a.boundary();
        reached[cfg.entry] = true;
        queue.push_back(cfg.entry);
        queued[cfg.entry] = true;
        while let Some(b) = queue.pop_front() {
            queued[b] = false;
            steps += 1;
            if steps > budget {
                break;
            }
            let mut f = inp[b].clone();
            for e in &cfg.blocks[b].elems {
                a.transfer(*e, &mut f);
            }
            out[b] = f;
            for (s, k) in &cfg.blocks[b].succs {
                let Some(ef) = a.edge(k, &out[b]) else { continue };
                let new_in = if reached[*s] { a.join(&inp[*s], &ef) } else { ef };
                if !reached[*s] || new_in != inp[*s] {
                    reached[*s] = true;
                    inp[*s] = new_in;
                    if !queued[*s] {
                        queued[*s] = true;
                        queue.push_back(*s);
                    }
                }
            }
        }
    } else {
        // backward: out = join of succ ins; in = transfer(out) in reverse
        for bnd in [cfg.exit, cfg.exc_exit] {
            out[bnd] = a.boundary();
            reached[bnd] = true;
        }
        for b in 0..n {
            queue.push_back(b);
            queued[b] = true;
        }
        while let Some(b) = queue.pop_front() {
            queued[b] = false;
            steps += 1;
            if steps > budget {
                break;
            }
            let mut o = if b == cfg.exit || b == cfg.exc_exit { a.boundary() } else { a.bottom() };
            for (s, _) in &cfg.blocks[b].succs {
                o = a.join(&o, &inp[*s]);
            }
            out[b] = o.clone();
            let mut f = o;
            for e in cfg.blocks[b].elems.iter().rev() {
                a.transfer(*e, &mut f);
            }
            if f != inp[b] || !reached[b] {
                reached[b] = true;
                inp[b] = f;
                for p in &cfg.blocks[b].preds {
                    if !queued[*p] {
                        queued[*p] = true;
                        queue.push_back(*p);
                    }
                }
            }
        }
    }
    Results { block_in: inp, block_out: out, reached }
}

impl<F: Clone> Results<F> {
    /// Fact immediately before `elem` executes (forward analyses).
    pub fn before<'t, A: Analysis<'t, Fact = F>>(&self, cfg: &Cfg<'t>, a: &A, elem: Node<'t>) -> Option<F> {
        let (b, i) = cfg.position(elem)?;
        if !self.reached[b] {
            return None;
        }
        let mut f = self.block_in[b].clone();
        for e in &cfg.blocks[b].elems[..i] {
            a.transfer(*e, &mut f);
        }
        Some(f)
    }

    /// Fact immediately after `elem` in program order, for backward analyses
    /// (i.e. the fact flowing *into* the element from its successors).
    pub fn after_backward<'t, A: Analysis<'t, Fact = F>>(&self, cfg: &Cfg<'t>, a: &A, elem: Node<'t>) -> Option<F> {
        let (b, i) = cfg.position(elem)?;
        let mut f = self.block_out[b].clone();
        for e in cfg.blocks[b].elems[i + 1..].iter().rev() {
            a.transfer(*e, &mut f);
        }
        Some(f)
    }

    /// Visit every element with the fact before it (forward).
    pub fn for_each_before<'t, A: Analysis<'t, Fact = F>>(&self, cfg: &Cfg<'t>, a: &A, mut f: impl FnMut(Node<'t>, &F)) {
        for (b, bl) in cfg.blocks.iter().enumerate() {
            if !self.reached[b] {
                continue;
            }
            let mut fact = self.block_in[b].clone();
            for e in &bl.elems {
                f(*e, &fact);
                a.transfer(*e, &mut fact);
            }
        }
    }
}
