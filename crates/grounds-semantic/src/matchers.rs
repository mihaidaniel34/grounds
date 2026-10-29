//! Declarative matchers for calls, in the spirit of Error Prone's
//! MethodMatchers. Rules describe *which* API they care about; the matcher
//! takes care of resolution, inheritance and static/instance distinctions.

use grounds_syntax::NodeExt;
use crate::model::{FileModel, ResolvedCall};
use grounds_index::*;
use grounds_syntax::Node;

#[derive(Clone, Copy, Debug)]
pub enum OwnerMatch {
    Any,
    /// Declaring class exactly.
    Exact(&'static str),
    /// Receiver/declaring class is this type or a subtype.
    SubtypeOf(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct MethodMatcher {
    pub owner: OwnerMatch,
    pub names: &'static [&'static str],
    /// Erased parameter type names; `None` = any.
    pub params: Option<&'static [&'static str]>,
    pub is_static: Option<bool>,
    pub arity: Option<usize>,
}

impl MethodMatcher {
    pub const fn instance(owner: &'static str) -> MethodMatcher {
        MethodMatcher { owner: OwnerMatch::SubtypeOf(owner), names: &[], params: None, is_static: Some(false), arity: None }
    }
    pub const fn exact(owner: &'static str) -> MethodMatcher {
        MethodMatcher { owner: OwnerMatch::Exact(owner), names: &[], params: None, is_static: None, arity: None }
    }
    pub const fn statics(owner: &'static str) -> MethodMatcher {
        MethodMatcher { owner: OwnerMatch::Exact(owner), names: &[], params: None, is_static: Some(true), arity: None }
    }
    pub const fn any() -> MethodMatcher {
        MethodMatcher { owner: OwnerMatch::Any, names: &[], params: None, is_static: None, arity: None }
    }
    pub const fn named(mut self, names: &'static [&'static str]) -> MethodMatcher {
        self.names = names;
        self
    }
    pub const fn params(mut self, params: &'static [&'static str]) -> MethodMatcher {
        self.params = Some(params);
        self
    }
    pub const fn arity(mut self, n: usize) -> MethodMatcher {
        self.arity = Some(n);
        self
    }

    pub fn matches_resolved(&self, lk: &dyn ClassLookup, c: &ResolvedCall) -> bool {
        if !self.names.is_empty() && !self.names.contains(&&*c.method.name) {
            return false;
        }
        if let Some(s) = self.is_static {
            if c.method.flags.is_static() != s {
                return false;
            }
        }
        if let Some(n) = self.arity {
            if c.method.params.len() != n {
                return false;
            }
        }
        if let Some(ps) = self.params {
            if ps.len() != c.method.params.len() {
                return false;
            }
            for (want, got) in ps.iter().zip(&c.method.params) {
                let e = got.ty.erasure();
                let s = match &e {
                    Ty::Class { name, .. } => name.to_string(),
                    Ty::Prim(p) => p.keyword().to_string(),
                    Ty::Array(_) => format!("{}", e),
                    Ty::TypeVar(_) => "java.lang.Object".into(),
                    _ => return false,
                };
                if *want != "*" && *want != s {
                    return false;
                }
            }
        }
        match self.owner {
            OwnerMatch::Any => true,
            OwnerMatch::Exact(o) => &*c.owner == o,
            OwnerMatch::SubtypeOf(o) => {
                if &*c.owner == o {
                    return true;
                }
                if lk.is_subclass(&c.owner, o).is_yes() {
                    return true;
                }
                // inherited from a supertype of `o` but called on a subtype of `o`
                match &c.receiver {
                    Some(r) => r.class_name().is_some_and(|rn| lk.is_subclass(rn, o).is_yes()),
                    None => false,
                }
            }
        }
    }
}

impl<'t> FileModel<'t> {
    /// Does this call (method invocation or constructor) match?
    pub fn call_matches(&self, call: Node<'t>, m: &MethodMatcher) -> bool {
        if call.kind() == "method_invocation" && !m.names.is_empty() {
            // cheap name pre-check before resolution
            match call.field("name") {
                Some(n) if m.names.contains(&self.text(n)) => {}
                _ => return false,
            }
        }
        match self.select_method(call) {
            Some(c) => m.matches_resolved(self, &c),
            None => false,
        }
    }

    pub fn call_matches_any(&self, call: Node<'t>, ms: &[MethodMatcher]) -> bool {
        ms.iter().any(|m| self.call_matches(call, m))
    }

    /// Name of the invoked method, without resolution.
    pub fn call_name(&self, call: Node<'t>) -> Option<&'t str> {
        call.field("name").map(|n| self.text(n))
    }
}
