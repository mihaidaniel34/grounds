//! Doubt: where something unknown was turned into a definite-looking answer.
//!
//! What the analyzer cannot resolve (a missing jar, an unresolved supertype)
//! must not make a rule report. Many queries answer in a type that has no
//! room for "unknown": a `bool`, an `Option`, a `Vec`. When such a query
//! answers `false`, `None` or empty because something was unknown, it calls
//! [`note`] with the kind of what was unknown. A note means: an answer was
//! given that more information could have changed.
//!
//! A query that answers in a type that keeps the unknown (`Tri::Unknown`,
//! `Ty::Unknown`, a result that says it is a guess) does not note: whoever
//! asked is handling it. Reading that answer as a definite one (`is_yes` on
//! `Tri::Unknown`, `is_string` on `Ty::Unknown`) is a collapse, and notes.
//! Asking whether something is unknown or complete (`is_unknown`,
//! `hierarchy_complete`) is handling it, and does not note.
//!
//! The runner runs each rule visit in a [`scope`], and a finding carries
//! what was noted while its rule derived it. A finding with nothing noted is
//! grounded: no answer it rested on was a collapsed unknown, so with more
//! information the semantic layer would have given the same answers, and the
//! rule the same finding.
//!
//! Notes are thread-local: rules run per file on one worker thread, and one
//! rule visit is synchronous. Noting is one `Cell` update.
//!
//! Memoized computations ([`compute`]) store what they observed with their
//! value and [`replay`] it on every hit, so a cache does not lose doubt.
//! What is observed inside such a computation is recorded as *deep*;
//! everything else (the query a rule asked, its direct answer) as *surface*.
//!
//! A rule's stance (`RuleMeta::tolerates` in grounds-rules) decides what is
//! reported: a strict rule, the default, reports only grounded findings; a
//! rule may declare kinds of doubt it reports in spite of, each with its
//! reason, and those declarations are the whole list of rules that report
//! without full grounds.
//!
//! # Assumptions about what cannot be seen
//!
//! A few unknowns are not noted because the engine assumes something about
//! them, written down here.
//!
//! - **An unseen supertype declares no variable that obscures a visible type
//!   or package.** A simple name used as a qualifier (`Objects` in
//!   `Objects.requireNonNull(x)`, `org` in `org.junit.Assert`) is, by JLS
//!   6.4.2, a variable before it is a type, and a type before it is a
//!   package. When the name resolves to a visible type or package and no
//!   visible variable (local, field of an enclosing class or its visible
//!   supertypes, static import) has it, an unresolved supertype of an
//!   enclosing class could still declare a field of that name and obscure
//!   it. Java's naming conventions (fields start lower case and are not named
//!   like packages; types start upper case) make that negligible, and not
//!   assuming it would put every qualified call in a class with an
//!   unresolved supertype in doubt. Only that case is assumed: a method or
//!   field of the name that an unseen supertype may declare beside a static
//!   import (the shadowing of JLS 15.12.1) stays in doubt.
//!
//! Computations made once for every file and thread do not reach any visit:
//! declaration lowering in the index runs [`quiet`], and the DI model that
//! the wiring rules read is built before any rule runs. Those rules are
//! judged by what their own visit observes, which is not what the model
//! collapsed.

use std::cell::Cell;
use std::fmt;

/// Kinds of collapse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Doubt {
    /// A predicate or accessor was asked of `Ty::Unknown`, or a type query
    /// answered `Ty::Unknown`.
    UnknownType,
    /// A call (method or constructor) did not resolve.
    UnresolvedCall,
    /// A call resolved to a guess among overloads (`ambiguous`), or to one
    /// whose erasure only is known (`erasure_only`).
    GuessedCall,
    /// A name (identifier, field access, qualified name) did not resolve.
    UnresolvedName,
    /// A class lookup found nothing.
    MissingClass,
    /// A three-valued `Unknown` was read as `false` (`is_yes`/`is_no`).
    TriCollapsed,
    /// A walk over supertypes skipped a class it could not find, or stopped.
    IncompleteHierarchy,
    /// An unresolved annotation was matched by its simple name.
    AnnotationBySimpleName,
}

impl Doubt {
    pub const ALL: [Doubt; 8] = [
        Doubt::UnknownType,
        Doubt::UnresolvedCall,
        Doubt::GuessedCall,
        Doubt::UnresolvedName,
        Doubt::MissingClass,
        Doubt::TriCollapsed,
        Doubt::IncompleteHierarchy,
        Doubt::AnnotationBySimpleName,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Doubt::UnknownType => "UnknownType",
            Doubt::UnresolvedCall => "UnresolvedCall",
            Doubt::GuessedCall => "GuessedCall",
            Doubt::UnresolvedName => "UnresolvedName",
            Doubt::MissingClass => "MissingClass",
            Doubt::TriCollapsed => "TriCollapsed",
            Doubt::IncompleteHierarchy => "IncompleteHierarchy",
            Doubt::AnnotationBySimpleName => "AnnotationBySimpleName",
        }
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// Bit offset of the deep kinds in [`Doubts`].
const DEEP: u32 = 16;

/// A set of [`Doubt`] kinds, each observed at the surface (by the code that
/// asked) or deep (inside a memoized computation whose answer it used).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Doubts(u32);

impl Doubts {
    pub const NONE: Doubts = Doubts(0);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `d` was observed, at the surface or deep.
    pub fn contains(self, d: Doubt) -> bool {
        self.0 & (d.bit() | d.bit() << DEEP) != 0
    }

    /// Whether `d` was observed at the surface.
    pub fn at_surface(self, d: Doubt) -> bool {
        self.0 & d.bit() != 0
    }

    /// Whether anything was observed at the surface.
    pub fn any_surface(self) -> bool {
        self.0 & ((1 << DEEP) - 1) != 0
    }

    /// The kinds observed, at the surface or deep.
    pub fn kinds(self) -> impl Iterator<Item = Doubt> {
        Doubt::ALL.into_iter().filter(move |d| self.contains(*d))
    }

    pub fn union(self, o: Doubts) -> Doubts {
        Doubts(self.0 | o.0)
    }

    /// The same kinds, all deep.
    pub fn deepen(self) -> Doubts {
        Doubts((self.0 | self.0 << DEEP) & !((1 << DEEP) - 1))
    }
}

impl fmt::Debug for Doubts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Doubts({self})")
    }
}

/// `Kind,Kind`; a kind observed only deep is written `deep:Kind`.
impl fmt::Display for Doubts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for d in self.kinds() {
            if !first {
                write!(f, ",")?;
            }
            first = false;
            if !self.at_surface(d) {
                write!(f, "deep:")?;
            }
            write!(f, "{}", d.name())?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct State {
    observed: u32,
    /// nesting of `compute`
    depth: u32,
}

thread_local! {
    static STATE: Cell<State> = const { Cell::new(State { observed: 0, depth: 0 }) };
}

/// Record that an unknown was collapsed into a definite answer.
#[inline]
pub fn note(d: Doubt) {
    STATE.with(|s| {
        let mut st = s.get();
        st.observed |= if st.depth > 0 { d.bit() << DEEP } else { d.bit() };
        s.set(st);
    });
}

/// Record what a computation observed (a memoized value's doubts, on a hit).
#[inline]
pub fn replay(d: Doubts) {
    if d.is_empty() {
        return;
    }
    STATE.with(|s| {
        let mut st = s.get();
        st.observed |= if st.depth > 0 { d.deepen().0 } else { d.0 };
        s.set(st);
    });
}

/// Restores the enclosing scope when a scope ends, also by unwinding (the
/// runner catches a rule's panic and goes on with the file).
struct Restore {
    outer: u32,
    depth: u32,
    keep: bool,
}

impl Drop for Restore {
    fn drop(&mut self) {
        STATE.with(|s| {
            let st = s.get();
            let inner = if self.keep { st.observed } else { 0 };
            s.set(State { observed: self.outer | inner, depth: self.depth });
        });
    }
}

fn run<R>(f: impl FnOnce() -> R, keep: bool, deeper: bool) -> (R, Doubts) {
    let st = STATE.with(|s| s.get());
    STATE.with(|s| s.set(State { observed: 0, depth: st.depth + deeper as u32 }));
    let restore = Restore { outer: st.observed, depth: st.depth, keep };
    let r = f();
    let observed = Doubts(STATE.with(|s| s.get().observed));
    drop(restore);
    (r, observed)
}

/// Run `f` and return what it observed. What the scope observed is also
/// observed by the enclosing one.
pub fn scope<R>(f: impl FnOnce() -> R) -> (R, Doubts) {
    run(f, true, false)
}

/// Run `f` as a memoized computation: what it observes is deep, and is
/// returned to be stored with its value (and also observed by the
/// enclosing scope).
pub fn compute<R>(f: impl FnOnce() -> R) -> (R, Doubts) {
    run(f, true, true)
}

/// Run `f` without its observations reaching the enclosing scope: for code
/// that handles an unknown itself (answers `Tri::Unknown`, or asks whether
/// something is complete).
pub fn quiet<R>(f: impl FnOnce() -> R) -> R {
    run(f, false, false).0
}

/// Run `f` and return what it observed without passing it to the enclosing
/// scope: the caller decides from `f`'s answer whether it [`replay`]s it
/// (a query that hands an unresolved case back as a value notes nothing for
/// it, and replays what it observed otherwise).
pub fn observe<R>(f: impl FnOnce() -> R) -> (R, Doubts) {
    run(f, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_nest_and_computations_are_deep() {
        let ((), outer) = scope(|| {
            note(Doubt::UnknownType);
            let ((), inner) = scope(|| note(Doubt::MissingClass));
            assert!(inner.at_surface(Doubt::MissingClass) && !inner.contains(Doubt::UnknownType));
            let ((), c) = compute(|| note(Doubt::GuessedCall));
            assert!(c.contains(Doubt::GuessedCall) && !c.any_surface());
            quiet(|| note(Doubt::TriCollapsed));
        });
        assert!(outer.at_surface(Doubt::UnknownType) && outer.at_surface(Doubt::MissingClass));
        assert!(outer.contains(Doubt::GuessedCall) && !outer.at_surface(Doubt::GuessedCall));
        assert!(!outer.contains(Doubt::TriCollapsed));
        assert_eq!(outer.to_string(), "UnknownType,deep:GuessedCall,MissingClass");
        let ((), r) = scope(|| replay(outer));
        assert_eq!(r, outer);
        let ((), r) = compute(|| replay(outer));
        assert_eq!(r, outer.deepen());
    }

    #[test]
    fn a_panic_restores_the_enclosing_scope() {
        let ((), outer) = scope(|| {
            note(Doubt::UnknownType);
            let r = std::panic::catch_unwind(|| compute(|| {
                note(Doubt::MissingClass);
                panic!("rule");
            }));
            assert!(r.is_err());
        });
        assert!(outer.at_surface(Doubt::UnknownType) && outer.contains(Doubt::MissingClass));
        let ((), after) = scope(|| note(Doubt::GuessedCall));
        assert!(after.at_surface(Doubt::GuessedCall), "depth restored after the panic");
    }
}
