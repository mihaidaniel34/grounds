//! Memoized queries of a file model.
//!
//! Types, symbols, call resolutions and constants are computed on demand and
//! remembered. The computations call each other recursively, and some of
//! these calls cycle (a lambda's parameter type needs the call it is an
//! argument of, whose resolution needs the lambda's type). A query already
//! in progress on a node answers "unknown" instead of recursing: a cut-off.
//! What was computed while a cut-off happened is provisional, and is not
//! remembered as the answer.
//!
//! [`Memo::get_or_compute`] is the one way to compute a remembered answer. It
//! marks the query in progress for as long as it runs (unmarked when it
//! returns, also by an early return or a panic), never stores a value
//! computed under a cut-off, and stores the doubts observed while computing
//! (see `grounds_index::doubt`) with the value, to replay them on every hit.

use crate::model::Unknowable;
use grounds_index::doubt::{self, Doubts};
use grounds_index::MethodInfo;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::{Cell, RefCell};

/// The kinds of query that guard against cycles: a node may be in progress
/// for several kinds at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Query {
    /// `type_of`
    Type,
    /// `local_type`
    Local,
    /// `expected_type` (guarded, not remembered)
    Expected,
    /// `symbol`
    Symbol,
    /// `select_method`
    Select,
    /// `resolve_call`
    Resolve,
    /// `partial_inference`
    Partial,
    /// `arg_type`: an argument's invocation type as the enclosing call sees it
    Standalone(crate::calls::Target),
    /// `constant_value`: bounded by nesting depth, not by a mark per node
    Constant,
}

/// Nested `constant_value` calls: operands of an operator and final locals
/// defined by other final locals. Far above real constants (a few levels; a
/// generated string concatenation of a few hundred parts); the CLI analyzes
/// on threads with large stacks (see grounds-cli `STACK_SIZE`).
const MAX_CONSTANT_NESTING: u32 = 1000;

// Ordinary files stay below ~50k (largest seen across the corpus); nested
// Hamcrest matcher trees reach millions.
const WORK_BUDGET: u64 = 300_000;

/// The queries in progress on one file, and what they cut off.
#[derive(Default)]
pub(crate) struct Guards {
    in_progress: RefCell<FxHashSet<(usize, Query)>>,
    /// Cut-offs so far, of every kind: answers computed while this changed
    /// are provisional.
    cut_offs: Cell<u64>,
    /// Guarded computations so far; past `WORK_BUDGET` results that hit a
    /// cycle are remembered as unknown, so pathological files stay polynomial.
    work: Cell<u64>,
    /// `Query::Constant` calls in progress
    nesting: Cell<u32>,
    /// How often `Query::Constant` was cut off at its nesting bound.
    nesting_cut_offs: Cell<u64>,
}

/// A query marked in progress; dropping it removes the mark.
pub(crate) struct InProgress<'a> {
    guards: &'a Guards,
    mark: Option<(usize, Query)>,
}

impl Drop for InProgress<'_> {
    fn drop(&mut self) {
        match self.mark {
            Some(m) => {
                self.guards.in_progress.borrow_mut().remove(&m);
            }
            None => self.guards.nesting.set(self.guards.nesting.get() - 1),
        }
    }
}

impl Guards {
    /// Mark query `q` on node `id` in progress, or count a cut-off when it
    /// already is (or, for constants, when nesting is at its bound).
    fn enter(&self, id: usize, q: Query) -> Option<InProgress<'_>> {
        if q == Query::Constant {
            if self.nesting.get() >= MAX_CONSTANT_NESTING {
                self.nesting_cut_offs.set(self.nesting_cut_offs.get() + 1);
                self.cut_offs.set(self.cut_offs.get() + 1);
                return None;
            }
            self.nesting.set(self.nesting.get() + 1);
            return Some(InProgress { guards: self, mark: None });
        }
        self.work.set(self.work.get() + 1);
        if !self.in_progress.borrow_mut().insert((id, q)) {
            self.cut_offs.set(self.cut_offs.get() + 1);
            return None;
        }
        Some(InProgress { guards: self, mark: Some((id, q)) })
    }

    /// `f` guarded against cycles, not remembered: `default` when `q` is
    /// already in progress on `id`.
    pub(crate) fn guarded<R>(&self, id: usize, q: Query, default: R, f: impl FnOnce() -> R) -> R {
        match self.enter(id, q) {
            Some(_mark) => f(),
            None => default,
        }
    }

    /// Guarded computations so far.
    #[cfg(test)]
    pub(crate) fn work(&self) -> u64 {
        self.work.get()
    }

    /// How often constant evaluation gave up at its nesting bound.
    pub(crate) fn nesting_cut_offs(&self) -> u64 {
        self.nesting_cut_offs.get()
    }
}

/// Where remembered answers are kept.
pub(crate) trait Table: Default {
    type Key<'k>;
    type Val: Clone + Unknowable;
    fn get(&self, k: &Self::Key<'_>) -> Option<&(Self::Val, Doubts)>;
    fn put(&mut self, k: Self::Key<'_>, v: Self::Val, d: Doubts);
}

/// Answers by a hashable key.
pub(crate) struct ByKey<K, V>(FxHashMap<K, (V, Doubts)>);

impl<K, V> Default for ByKey<K, V> {
    fn default() -> Self {
        ByKey(FxHashMap::default())
    }
}

impl<K: std::hash::Hash + Eq, V: Clone + Unknowable> Table for ByKey<K, V> {
    type Key<'k> = K;
    type Val = V;
    fn get(&self, k: &K) -> Option<&(V, Doubts)> {
        self.0.get(k)
    }
    fn put(&mut self, k: K, v: V, d: Doubts) {
        self.0.insert(k, (v, d));
    }
}

/// Answers by node and method (a call's inference per candidate method).
pub(crate) struct ByMethod<V>(FxHashMap<usize, Vec<(MethodInfo, (V, Doubts))>>);

impl<V> Default for ByMethod<V> {
    fn default() -> Self {
        ByMethod(FxHashMap::default())
    }
}

impl<V: Clone + Unknowable> Table for ByMethod<V> {
    type Key<'k> = (usize, &'k MethodInfo);
    type Val = V;
    fn get(&self, (id, m): &(usize, &MethodInfo)) -> Option<&(V, Doubts)> {
        self.0.get(id)?.iter().find(|(k, _)| k == *m).map(|(_, e)| e)
    }
    fn put(&mut self, (id, m): (usize, &MethodInfo), v: V, d: Doubts) {
        self.0.entry(id).or_default().push((m.clone(), (v, d)));
    }
}

/// Remembered answers of one query.
#[derive(Default)]
pub(crate) struct Memo<T: Table>(RefCell<T>);

impl<T: Table> Memo<T> {
    /// The answer remembered for `key`, replaying the doubts observed when it
    /// was computed; else `f`'s, run with query `q` on node `id` marked in
    /// progress, and remembered with its doubts unless a cut-off happened
    /// meanwhile. When `q` is already in progress on `id`, the unknown value.
    pub(crate) fn get_or_compute(&self, guards: &Guards, key: T::Key<'_>, id: usize, q: Query, f: impl FnOnce() -> T::Val) -> T::Val {
        if let Some((v, d)) = self.0.borrow().get(&key) {
            doubt::replay(*d);
            return v.clone();
        }
        let before = guards.cut_offs.get();
        let Some(mark) = guards.enter(id, q) else { return T::Val::unknown() };
        let (v, d) = doubt::compute(f);
        drop(mark);
        let v = if guards.cut_offs.get() == before {
            v
        } else if q != Query::Constant && guards.work.get() > WORK_BUDGET {
            // provisional, and the file has used up its budget: unknown,
            // remembered, so the same work is not done again
            T::Val::unknown()
        } else {
            return v;
        };
        self.0.borrow_mut().put(key, v.clone(), d);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grounds_index::doubt::Doubt;
    use grounds_index::Ty;

    #[test]
    fn a_hit_replays_the_doubts_of_its_computation() {
        let (g, m): (Guards, Memo<ByKey<usize, Ty>>) = Default::default();
        let (t, d) = doubt::scope(|| {
            m.get_or_compute(&g, 1, 1, Query::Type, || {
                doubt::note(Doubt::MissingClass);
                Ty::int()
            })
        });
        assert_eq!(t, Ty::int());
        assert!(d.contains(Doubt::MissingClass) && !d.any_surface(), "observed inside the computation: deep");
        let (t, d) = doubt::scope(|| m.get_or_compute(&g, 1, 1, Query::Type, || unreachable!("remembered")));
        assert_eq!(t, Ty::int());
        assert!(d.contains(Doubt::MissingClass), "replayed on the hit");
    }

    #[test]
    fn a_value_computed_under_a_cut_off_is_not_remembered() {
        let (g, m): (Guards, Memo<ByKey<usize, Ty>>) = Default::default();
        // the computation of 1 asks for 1 again: the inner call is cut off
        let t = m.get_or_compute(&g, 1, 1, Query::Type, || {
            let inner = m.get_or_compute(&g, 1, 1, Query::Type, Ty::int);
            assert_eq!(inner, Ty::Unknown, "in progress: unknown");
            Ty::boolean()
        });
        assert_eq!(t, Ty::boolean());
        assert_eq!(m.get_or_compute(&g, 1, 1, Query::Type, Ty::int), Ty::int(), "the provisional answer was not stored");
    }

    #[test]
    fn a_panic_leaves_the_query_unmarked() {
        let (g, m): (Guards, Memo<ByKey<usize, Ty>>) = Default::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.get_or_compute(&g, 1, 1, Query::Type, || panic!("rule bug"))));
        assert!(r.is_err());
        assert_eq!(m.get_or_compute(&g, 1, 1, Query::Type, Ty::int), Ty::int(), "not left in progress");
        // constants count their nesting, also unwound by a panic
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.get_or_compute(&g, 2, 2, Query::Constant, || panic!("rule bug"))));
        assert!(r.is_err());
        assert_eq!(g.nesting.get(), 0);
    }

    #[test]
    fn constants_are_cut_off_at_their_nesting_bound() {
        let g = Guards::default();
        let m: Memo<ByKey<usize, Option<u32>>> = Default::default();
        fn deep(g: &Guards, m: &Memo<ByKey<usize, Option<u32>>>, n: usize) -> Option<u32> {
            m.get_or_compute(g, n, n, Query::Constant, || deep(g, m, n + 1).map(|v| v + 1).or(Some(0)))
        }
        assert_eq!(deep(&g, &m, 0), Some(MAX_CONSTANT_NESTING - 1));
        assert_eq!(g.nesting_cut_offs(), 1);
    }
}
