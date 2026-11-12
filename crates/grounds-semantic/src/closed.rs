//! The kernel of the claims that code does not compile.
//!
//! Each such claim is a claim of absence: no method of this name, no field,
//! no constructor taking these arguments, no supertype method this one
//! overrides, no supertype that makes these types assignable. Absence is
//! only true in a closed world: the class and all its supertypes fully
//! visible, each read from the copy this file compiles against and from a
//! file that parses, with no annotation processor or language extension
//! adding members the model does not reproduce. What grounds does not see
//! may be there.
//!
//! This module is the one place that decides it. [`FileModel::closed`]
//! checks every condition and gives a [`Closed`] proof, or the reason the
//! world is [`Open`]. The questions of absence are asked of a `Closed`, and
//! each answers with a [`Claim`] when the thing is absent. Nothing outside
//! this module can make a `Closed` or a `Claim`, and the rules report a
//! compile-break finding only with a `Claim` (`Ctx::report_claim` in
//! grounds-rules): a rule cannot report an absence it read from an ordinary
//! lookup, on a class nothing proved closed.
//!
//! A claim's text is what `grounds changed` compares before and after an
//! edit (`Diagnostic::claim` in grounds-rules): qualified names where the
//! message shows simple ones, whole sets where the message cuts them.
//!
//! Two claims rest on other proofs, made here too: that a type cannot be
//! assigned to another needs every supertype of both known, not every
//! member ([`FileModel::unassignable`]); that a JDK class is not in the
//! Java release needs the release stated exactly ([`FileModel::java_release`]).

use crate::model::{FileModel, Symbol};
use grounds_index::doubt;
use grounds_index::hierarchy::{apply_subst, is_subsignature, substitution};
use grounds_index::*;
use grounds_syntax::{Node, NodeExt};
use std::sync::Arc;

#[cfg(test)]
mod build_tests;
#[cfg(test)]
mod tests;

/// What a compile-break finding asserts, in the words `grounds changed`
/// compares before and after an edit. Only this module makes one, each from
/// a proof that the world it speaks of is closed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Claim(String);

impl Claim {
    fn new(text: String) -> Claim {
        Claim(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

/// Where the versions of the jars on the classpath come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jars {
    /// The classpath the build uses, dumped (`classpath.exact`).
    Exact,
    /// Guessed: the newest version in the Gradle and Maven caches. A member
    /// missing there may exist in the version the project builds against.
    Guessed,
}

/// Why the world around a class is not closed, so that no claim of absence
/// is made about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// This file uses Lombok's `@ExtensionMethod`: static methods of other
    /// classes are callable as instance methods of any receiver.
    ExtensionMethods,
    /// A javac plugin that changes the language (Manifold) compiles this
    /// file: members, access and types are not the ones the model reads.
    LanguageExtended,
    /// This file does not parse as Java (a template, a file mid-edit).
    FileHasErrors,
    /// No visible class has this name.
    NotFound(Name),
    /// The class is read from a jar: what it lacks there it may have in the
    /// version the project compiles against.
    NotProjectClass(Name),
    /// A class declaration this file names no class for.
    UnnamedClass,
    /// The code is in no class.
    NotInClass,
    /// No JDK is loaded: `java.lang.Object`, above every type, is unknown.
    NoJdk,
    /// A JDK class of the hierarchy may be newer than the one loaded.
    NewerJdk(Name),
    /// A class of the hierarchy is not visible.
    MissingClass(Name),
    /// A supertype that this class names does not resolve.
    UnresolvedSupertype(Name),
    /// Lombok adds members the model does not reproduce to this class of
    /// the hierarchy, or to a class enclosing it.
    MembersUnknown(Name),
    /// A class of the hierarchy is declared more than once, and the copy
    /// this file compiles against is not certain.
    CopyUncertain(Name),
    /// A class is declared more than once: the index holds one copy.
    DeclaredMoreThanOnce(Name),
    /// A project class of the hierarchy is in a JDK package: it is not
    /// compiled with the JDK's.
    InJdkPackage(Name),
    /// This file's module is built against a published release of the
    /// project, which has its own copy of this class from another module.
    BuiltAgainstRelease(Name),
    /// This class of the hierarchy is read from a file with syntax errors.
    SyntaxErrors(Name),
    /// This library class of the hierarchy is read from a guessed jar
    /// version (see [`Jars::Guessed`]).
    GuessedJarVersion(Name),
    /// The walk over the hierarchy stopped (past 200 classes): what it did
    /// not see is unknown.
    WalkCutOff,
    /// The build does not state the Java release exactly, or grounds cannot
    /// read that release's API: the API is that of whichever JDK runs it.
    ReleaseNotExact,
    /// The file is in a multi-release source set (`src/main/java24`), which
    /// compiles for its own, newer release.
    NewerReleaseSources(u32),
    /// The build does not compile this file: its module was built and its
    /// source root is not part of what was compiled (test data, samples, an
    /// old snapshot of the sources). Nothing compiles it, and what it names
    /// may resolve to a class it was never written against.
    NotInBuild,
}

impl From<Gap> for Open {
    fn from(g: Gap) -> Open {
        match g {
            Gap::NoJdk => Open::NoJdk,
            Gap::NewerJdk(n) => Open::NewerJdk(n),
            Gap::Missing(n) => Open::MissingClass(n),
            Gap::UnresolvedSupertype(n) => Open::UnresolvedSupertype(n),
            Gap::MembersUnknown(n) => Open::MembersUnknown(n),
        }
    }
}

/// The proof that a project class is closed: it and every class of its
/// hierarchy are fully visible, as this file sees them. Made only by
/// [`FileModel::closed`].
pub struct Closed<'m, 't> {
    fm: &'m FileModel<'t>,
    class: Arc<ClassInfo>,
}

impl<'t> FileModel<'t> {
    /// Whether this file is one in which absence can be claimed at all.
    fn file_closed(&self) -> Result<(), Open> {
        if self.index.in_build(&self.path) == InBuild::No {
            return Err(Open::NotInBuild);
        }
        if self.stub.classes.iter().any(|k| k.annotations.iter().any(|a| lombok::processor_annotation(a, &self.stub.imports).as_deref() == Some("lombok.experimental.ExtensionMethod"))) {
            return Err(Open::ExtensionMethods);
        }
        if self.index.language_extended(&self.path) {
            return Err(Open::LanguageExtended);
        }
        if self.file.has_errors() {
            return Err(Open::FileHasErrors);
        }
        Ok(())
    }

    /// The proof that project class `name` is closed, or why it is not. A
    /// complete hierarchy (no unresolved supertype, no members Lombok adds
    /// unmodelled, no JDK class newer than the one loaded) is not enough:
    /// every project class of it must be read from a file that parses, from
    /// the copy this file compiles against when its name is declared more
    /// than once (the build's module dependencies are not known), and library
    /// classes from the jar versions the project builds against. Nothing is
    /// claimed in a file the build does not compile.
    pub fn closed(&self, name: &str, jars: Jars) -> Result<Closed<'_, 't>, Open> {
        self.file_closed()?;
        let c = self.class(name).ok_or_else(|| Open::NotFound(Name::from(name)))?;
        if !c.is_source() {
            return Err(Open::NotProjectClass(c.name.clone()));
        }
        // asking whether it is complete is handling the unknown
        if let Some(gap) = doubt::quiet(|| self.hierarchy_gap(name, true)) {
            return Err(gap.into());
        }
        let supertypes = self.all_supertypes(&c.this_type());
        if supertypes.len() > 200 {
            return Err(Open::WalkCutOff);
        }
        let release_module = self.index.built_against_release(&self.path);
        for t in &supertypes {
            let Some(n) = t.class_name() else { return Err(Open::UnresolvedSupertype(c.name.clone())) };
            if !self.copy_is_certain(n) {
                return Err(Open::CopyUncertain(Name::from(n)));
            }
            if self.index.in_jdk_package(n) {
                return Err(Open::InJdkPackage(Name::from(n)));
            }
            let file = self.class(n).and_then(|k| k.source_file().map(|f| f.to_string()));
            if let (Some(module), Some(f)) = (release_module, &file) {
                if !self.index.in_module(f, module) {
                    return Err(Open::BuiltAgainstRelease(Name::from(n)));
                }
            }
            if file.is_some_and(|f| self.index.file(&f).is_some_and(|s| s.has_syntax_errors)) {
                return Err(Open::SyntaxErrors(Name::from(n)));
            }
        }
        if jars == Jars::Guessed {
            if let Some(n) = supertypes.iter().filter_map(|t| t.class_name()).find(|n| !n.starts_with("java.") && self.class(n).is_some_and(|k| !k.is_source())) {
                return Err(Open::GuessedJarVersion(Name::from(n)));
            }
        }
        Ok(Closed { fm: self, class: c })
    }

    /// The classes enclosing `at`, innermost first, each closed: what a name
    /// used without a qualifier there can be a member of.
    pub fn closed_scope(&self, at: Node<'t>, jars: Jars) -> Result<Scope<'_, 't>, Open> {
        let mut classes = Vec::new();
        let mut cur = self.enclosing_class_node(at);
        while let Some(c) = cur {
            let Some(cn) = self.class_name_of_decl(c).cloned() else { return Err(Open::UnnamedClass) };
            classes.push(self.closed(&cn, jars)?);
            cur = self.enclosing_class_node(c);
        }
        if classes.is_empty() {
            return Err(Open::NotInClass);
        }
        Ok(Scope { classes })
    }

    /// The class that a single static import's name `segs` (the member last:
    /// `a.b.Outer.Inner.member`) names, closed. Which segments are the
    /// package is the split that names a visible class; a split naming none
    /// is no package, and no claim rests on it.
    pub fn closed_import_owner(&self, segs: &[&str], jars: Jars) -> Result<Closed<'_, 't>, Open> {
        if segs.len() < 2 {
            return Err(Open::NotFound(Name::from(segs.join("."))));
        }
        let owner = (1..segs.len() - 1).rev().find_map(|k| {
            let name = format!("{}.{}", segs[..k].join("."), segs[k..segs.len() - 1].join("$"));
            class_found(self, &name).then_some(name)
        });
        match owner {
            Some(owner) => self.closed(&owner, jars),
            None => Err(Open::NotFound(Name::from(segs[..segs.len() - 1].join(".")))),
        }
    }

    /// The proof that this file compiles for one Java release exactly, whose
    /// API grounds reads: what a claim that a JDK class is not public API
    /// there rests on. Only where the build states the release exactly and
    /// the code agrees: with -source/-target alone, or a release grounds
    /// cannot read, the API is that of whichever JDK runs the build.
    pub fn java_release(&self) -> Result<Release<'_, 't>, Open> {
        if self.index.in_build(&self.path) == InBuild::No {
            return Err(Open::NotInBuild);
        }
        let Some(release) = self.index.exact_release(&self.path) else { return Err(Open::ReleaseNotExact) };
        // multi-release sources (`src/main/java24`) compile for their own, newer release
        let own = self.path.split(['/', '\\']).filter_map(|d| d.strip_prefix("java")).filter_map(|v| v.strip_prefix('-').unwrap_or(v).parse::<u32>().ok()).max();
        if let Some(v) = own.filter(|v| *v > release) {
            return Err(Open::NewerReleaseSources(v));
        }
        Ok(Release { fm: self, release })
    }

    /// Whether every supertype of class `name` is known, from files that
    /// parse and a single declaration of it: what a claim that no supertype
    /// connects two types rests on. Members the model lacks do not matter.
    fn supertypes_closed(&self, name: &str) -> Result<(), Open> {
        if self.class(name).is_none() {
            return Err(Open::NotFound(Name::from(name)));
        }
        if let Some(gap) = doubt::quiet(|| self.hierarchy_gap(name, false)) {
            return Err(gap.into());
        }
        if self.index.is_duplicated(name) {
            return Err(Open::DeclaredMoreThanOnce(Name::from(name)));
        }
        if !parses(self, name) {
            return Err(Open::SyntaxErrors(Name::from(name)));
        }
        Ok(())
    }

    /// The call `value`, of a project method, returns a class that cannot be
    /// assigned to the plain class type `target`. Only a call to a method
    /// without rivals is judged (overload selection is approximate: type
    /// arguments, lambdas, inaccessible overloads), with a class result
    /// without type arguments, both classes' supertypes known (an unknown one
    /// could connect them; members could not), and every class read from a
    /// file that parses (a broken stub may miss a supertype or an overload).
    /// The claim is the message, which names the classes by simple name.
    pub fn unassignable(&self, value: Node<'t>, target: &Ty) -> Option<Claim> {
        // a file that does not parse is no ground for a compile-break claim,
        // nor one a language-changing javac plugin compiles (Manifold's
        // structural interfaces make unrelated classes assignable), nor one
        // the build does not compile
        if self.file.has_errors() || self.index.language_extended(&self.path) || self.index.in_build(&self.path) == InBuild::No {
            return None;
        }
        if value.kind() != "method_invocation" {
            return None;
        }
        let Ty::Class { name: want, args } = target else { return None };
        if !args.is_empty() {
            return None;
        }
        let c = self.select_method(value)?;
        if c.ambiguous || !self.class(&c.owner).is_some_and(|k| k.is_source()) {
            return None;
        }
        // a class defined in several modules: the index holds one of them
        let receiver_class = c.receiver.as_ref().and_then(|r| r.class_name().map(str::to_string));
        if self.index.is_duplicated(&c.owner) || receiver_class.as_deref().is_some_and(|r| self.index.is_duplicated(r)) {
            return None;
        }
        // a private method is never inherited, nestmates or not: found through
        // another class, the lookup may have put it in front of an interface
        // default that class has
        if c.method.flags.is_private() {
            let through = receiver_class.or_else(|| self.enclosing_class_name(value).map(|e| e.to_string()));
            if through.as_deref() != Some(&*c.owner) {
                return None;
            }
        }
        let nargs = self.args_of(value).len();
        // rivals the lookup below does not see: the other bounds of a type
        // variable (`T extends A & B`), methods a static import brings in
        match value.field("object") {
            Some(o) if matches!(self.type_of(o), Ty::TypeVar(_)) => return None,
            None if self.stub.imports.iter().any(|i| i.is_static && (i.on_demand || i.is_named(&c.method.name))) => return None,
            _ => {}
        }
        let rivals = match &c.receiver {
            Some(r) => self.methods_named(r, &c.method.name).iter().filter(|m| m.method.accepts_arity(nargs)).count(),
            None => self.class(&c.owner).map(|k| self.methods_named(&k.this_type(), &c.method.name).iter().filter(|m| m.method.accepts_arity(nargs)).count()).unwrap_or(2),
        };
        if rivals != 1 {
            return None;
        }
        let Ty::Class { name: got, args: gargs } = &c.method.ret else { return None };
        if !gargs.is_empty() || got == want {
            return None;
        }
        if !parses(self, &c.owner) {
            return None;
        }
        if self.supertypes_closed(got).is_err() || self.supertypes_closed(want).is_err() {
            return None;
        }
        self.is_assignable(&c.method.ret, target)
            .is_no()
            .then(|| Claim::new(format!("`{}.{}()` returns `{}`, which cannot be assigned to `{}`", simple_name_of(&c.owner), c.method.name, simple_name_of(got), simple_name_of(want))))
    }
}

/// The class `name` is visible. Finding nothing is no collapse here: the
/// caller asks which name is a class, and notes nothing for the others.
fn class_found(fm: &FileModel<'_>, name: &str) -> bool {
    let (c, d) = doubt::observe(|| fm.class(name));
    if c.is_some() {
        doubt::replay(d);
    }
    c.is_some()
}

/// The class `n`, if a project class, is read from a file that parses.
fn parses(fm: &FileModel<'_>, n: &str) -> bool {
    fm.class(n).and_then(|k| k.source_file().map(|f| f.to_string())).is_none_or(|f| !fm.index.file(&f).is_some_and(|s| s.has_syntax_errors))
}

/// A method's identity for a claim: qualified owner and erased, qualified
/// parameter types (the message shows simple names).
fn method_id(owner: &str, m: &MethodInfo) -> String {
    format!("{owner}.{}({})", m.name, m.params.iter().map(|p| p.ty.erasure().to_string()).collect::<Vec<_>>().join(","))
}

impl<'m, 't> Closed<'m, 't> {
    /// The closed class.
    pub fn class(&self) -> &Arc<ClassInfo> {
        &self.class
    }

    /// The methods of the class (its own and inherited) named `name`: what
    /// the questions about a call or reference of that name are answered from.
    pub fn methods(&self, name: &str) -> Overloads<'_, 'm, 't> {
        Overloads { closed: self, name: name.to_string(), methods: self.fm.methods_named(&self.class.this_type(), name) }
    }

    /// The field access `access` (`x.f`, `Type.f`) on this class names no
    /// field of it, nor a member type (`Outer.Inner` is a type, not a field).
    pub fn no_field(&self, access: Node<'t>) -> Option<Claim> {
        let fm = self.fm;
        let (Some(o), Some(f)) = (access.field("object"), access.field("field")) else { return None };
        if access.kind() != "field_access" || f.kind() != "identifier" {
            return None;
        }
        let fname = fm.text(f);
        // the field resolves (what was observed finding it counts as usual;
        // nothing is noted for a name that resolves to no field)
        let (s, d) = doubt::observe(|| fm.symbol(access));
        if s.is_some() {
            doubt::replay(d);
        }
        if matches!(s, Some(Symbol::Field(_))) {
            return None;
        }
        // the access is on this class, through its name or a value of it
        match fm.symbol(o) {
            Some(Symbol::Type(t)) => {
                if *t != *self.class.name || fm.index.member_type(&t, fname).is_some() || class_found(fm, &format!("{t}${fname}")) {
                    return None;
                }
            }
            Some(Symbol::Package(_)) => return None,
            _ => {
                let t = fm.type_of(o);
                if t.is_array() || t.class_name() != Some(&*self.class.name) {
                    return None;
                }
            }
        }
        Some(Claim::new(format!("`{}` has no field `{fname}`", self.class.name)))
    }

    /// No constructor of the class takes the arguments of `creation`
    /// (`new C(..)`, not an anonymous class), counting them.
    pub fn no_constructor(&self, creation: Node<'t>) -> Option<NoConstructor> {
        let cls = &self.class;
        if creation.kind() != "object_creation_expression" || creation.kid_of_kind("class_body").is_some() || cls.is_interface() || cls.is_enum() {
            return None;
        }
        // `outer.new Inner()` names a member type of the qualifier's type
        if self.fm.created_class(creation).class_name() != Some(&*cls.name) {
            return None;
        }
        let nargs = self.fm.args_of(creation).len();
        let constructors: Vec<MethodInfo> = cls.constructors().cloned().collect();
        if constructors.is_empty() || constructors.iter().any(|c| c.accepts_arity(nargs)) {
            return None;
        }
        Some(NoConstructor { claim: Claim::new(format!("No constructor of `{}` takes {nargs} argument(s)", cls.name)), nargs, constructors })
    }

    /// The method declared by `decl` in this class overrides no method of a
    /// supertype: no supertype declares an instance method, not private,
    /// whose signature it has or erases to, nor is it one of `Object`'s or a
    /// record's accessor. A parameter type not known, or the same simple
    /// name from another package (javax vs jakarta when the cached jars do
    /// not match the project), may still be the same parameter.
    pub fn overrides_nothing(&self, decl: Node<'t>) -> Option<Claim> {
        let fm = self.fm;
        let cls = &self.class;
        let m = fm.method_of_decl(decl)?;
        if fm.enclosing_class(decl).is_none_or(|c| c.name != cls.name) || m.params.iter().any(|p| p.ty.has_unknown()) {
            return None;
        }
        // explicit record accessors may carry @Override
        if cls.is_record() && m.params.is_empty() && cls.fields.iter().any(|f| f.flags.has(Flags::RECORD_COMPONENT) && f.name == m.name) {
            return None;
        }
        // Object methods are always overridable
        if matches!((&*m.name, m.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0) | ("clone", 0) | ("finalize", 0)) {
            return None;
        }
        for st in fm.all_supertypes(&cls.this_type()).into_iter().skip(1) {
            let sc = fm.class(st.class_name()?)?;
            let map = substitution(&sc, st.args());
            let found = sc.methods.iter().any(|x| {
                x.name == m.name
                    && !x.flags.is_static()
                    && !x.flags.is_private()
                    && x.params.len() == m.params.len()
                    && (!is_subsignature(&m, x, &sc, st.args()).is_no()
                        || x.params.iter().zip(&m.params).all(|(a, b)| {
                            let at = apply_subst(&a.ty, &map);
                            let skew = match (at.erasure().class_name(), b.ty.erasure().class_name()) {
                                (Some(x), Some(y)) => {
                                    let from_jar = |n: &str| fm.class(n).is_some_and(|c| !c.is_source());
                                    x != y && simple_name_of(x) == simple_name_of(y) && (from_jar(x) || from_jar(y))
                                }
                                _ => false,
                            };
                            skew || !fully_known(fm, &at) || !fully_known(fm, &b.ty)
                        })
                        || same_erasure(&m, cls, x, &map))
            });
            if found {
                return None;
            }
        }
        Some(Claim::new(format!("`{}` is marked @Override but overrides no supertype method", method_id(&cls.name, &m))))
    }

    /// The class declared by `decl`, not abstract, leaves abstract methods
    /// it inherits unimplemented: none of it, its superclasses or the
    /// default methods of its interfaces implements them. An enum whose
    /// constants have bodies implements them there. The claim is every
    /// missing method (its signature: which of several supertypes declaring
    /// it was kept depends on the order they are listed in), sorted.
    pub fn unimplemented(&self, decl: Node<'t>) -> Option<Unimplemented> {
        let fm = self.fm;
        let cls = &self.class;
        if decl.has_modifier("abstract") || cls.is_abstract() || fm.class_of_decl(decl).is_none_or(|c| c.name != cls.name) {
            return None;
        }
        // enum constants with bodies implement abstract (or interface)
        // methods themselves
        let constant_bodies = decl.field("body").is_some_and(|b| b.kids().iter().any(|k| k.kind() == "enum_constant" && k.kid_of_kind("class_body").is_some()));
        if cls.is_enum() && (constant_bodies || fm.text(decl).contains("abstract ")) {
            return None;
        }
        let mut missing = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for m in fm.all_methods(&cls.this_type()) {
            if !m.method.flags.is_abstract() || m.method.flags.is_static() {
                continue;
            }
            // Object's methods (re-declared abstract by Record, Comparator...) are always implemented
            if matches!((&*m.method.name, m.method.params.len()), ("equals", 1) | ("hashCode", 0) | ("toString", 0)) {
                continue;
            }
            if m.method.params.iter().any(|p| p.ty.has_unknown()) {
                return None;
            }
            if !implements(fm, cls, &m.method.name, m.method.params.len()) {
                let id = method_id(&m.owner, &m.method);
                ids.push(id[m.owner.len() + 1..].to_string());
                missing.push(m);
            }
        }
        if missing.is_empty() {
            return None;
        }
        // two supertypes may require one signature (`<T> f(T)` and
        // `f(Object)` erase alike): each requirement once
        ids.sort();
        ids.dedup();
        Some(Unimplemented { claim: Claim::new(format!("`{}` does not implement {}", cls.name, ids.join(", "))), missing })
    }

    /// The class, its supertypes and their member types have no member
    /// named `member`, which a static import of it names. The claim is the
    /// message, which names the class by simple name.
    pub fn no_static_member(&self, member: &str) -> Option<Claim> {
        let fm = self.fm;
        let mut types: Vec<String> = vec![self.class.name.to_string()];
        types.extend(fm.all_supertypes(&self.class.this_type()).iter().filter_map(|t| t.class_name().map(str::to_string)));
        let exists = types.iter().any(|t| fm.class(t).is_some_and(|c| c.field(member).is_some() || c.methods_named(member).next().is_some()) || class_found(fm, &format!("{t}${member}")));
        (!exists).then(|| Claim::new(format!("`{}` has no static member `{member}`", self.class.simple_name())))
    }
}

/// The methods named one name of a closed class (see [`Closed::methods`]).
pub struct Overloads<'c, 'm, 't> {
    closed: &'c Closed<'m, 't>,
    name: String,
    methods: Vec<MethodRef>,
}

/// A call whose argument types no overload accepts.
pub struct Mismatch {
    pub claim: Claim,
    /// The types of the call's arguments.
    pub args: Vec<Ty>,
}

/// A method name whose every fitting overload is private to another
/// top-level class.
pub struct Private {
    pub claim: Claim,
    /// The method the call would reach.
    pub method: MethodRef,
}

/// A `new` whose arguments no constructor takes.
pub struct NoConstructor {
    pub claim: Claim,
    pub nargs: usize,
    pub constructors: Vec<MethodInfo>,
}

/// The abstract methods a concrete class does not implement.
pub struct Unimplemented {
    pub claim: Claim,
    /// In the order the hierarchy lists them, a signature more than once
    /// when two supertypes require it.
    pub missing: Vec<MethodRef>,
}

impl<'c, 'm, 't> Overloads<'c, 'm, 't> {
    /// Every method of the name.
    pub fn all(&self) -> &[MethodRef] {
        &self.methods
    }

    fn class_name(&self) -> &str {
        &self.closed.class.name
    }

    /// The class has no method of the name.
    pub fn none(&self) -> Option<Claim> {
        self.methods.is_empty().then(|| Claim::new(format!("`{}` has no method `{}`", self.class_name(), self.name)))
    }

    /// The class has methods of the name, and none takes `nargs` arguments.
    pub fn none_taking(&self, nargs: usize) -> Option<Claim> {
        (!self.methods.is_empty() && !self.methods.iter().any(|m| m.method.accepts_arity(nargs))).then(|| Claim::new(format!("No `{}` in `{}` takes {nargs} argument(s)", self.name, self.class_name())))
    }

    /// Every overload taking as many arguments as `call` has a parameter
    /// that definitely cannot accept its argument. Only decided when every
    /// argument's type is known: not a lambda, a method reference, `null`,
    /// a type not known or one that mentions a type variable.
    pub fn none_accepting(&self, call: Node<'t>) -> Option<Mismatch> {
        let args = definitely_incompatible(self.closed.fm, call, &self.methods)?;
        let qualified = args.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
        Some(Mismatch { claim: Claim::new(format!("No `{}` in `{}` accepts ({qualified})", self.name, self.class_name())), args })
    }

    /// Every overload (taking `nargs` arguments, if given; `None` for a
    /// method reference) is private to a top-level class other than the one
    /// of `site`, and no supertype has a visible one: a private method is
    /// not inherited, and nested classes share their top-level class's
    /// private members.
    pub fn private_elsewhere(&self, site: Node<'t>, nargs: Option<usize>) -> Option<Private> {
        let method = private_elsewhere(self.closed.fm, site, &self.closed.class, &self.name, &self.methods, nargs)?;
        let claim = match nargs {
            Some(k) => format!("`{}` with {k} argument(s) is private in `{}`", self.name, self.class_name()),
            None => format!("`{}` is private in `{}`", self.name, self.class_name()),
        };
        Some(Private { claim: Claim::new(claim), method })
    }

    /// The call through `receiver`, the class's name (`C.m()`), reaches only
    /// instance methods of `nargs` arguments: none can be called on the
    /// type. Inside the class (an inner class calling `Outer.m()`), the
    /// outer instance is reached legally.
    pub fn instance_on_type(&self, receiver: Node<'t>, nargs: usize) -> Option<Claim> {
        let fm = self.closed.fm;
        let cls = &self.closed.class;
        if !matches!(fm.symbol(receiver), Some(Symbol::Type(t)) if *t == *cls.name) {
            return None;
        }
        let mut fitting = self.methods.iter().filter(|m| m.method.accepts_arity(nargs)).peekable();
        if fitting.peek().is_none() || !fitting.all(|m| !m.method.flags.is_static()) {
            return None;
        }
        if fm.enclosing_class_name(receiver).is_some_and(|e| e.starts_with(&*cls.name)) {
            return None;
        }
        Some(Claim::new(format!("`{}.{}()` is an instance method and cannot be called on the type", cls.name, self.name)))
    }
}

/// The closed classes enclosing a place, innermost first (see
/// [`FileModel::closed_scope`]).
pub struct Scope<'m, 't> {
    classes: Vec<Closed<'m, 't>>,
}

/// What a call without a qualifier finds in its enclosing classes.
pub enum InScope {
    /// No method of the name.
    NotDefined(Claim),
    /// Only methods private to other top-level classes.
    Private(Private),
}

impl<'m, 't> Scope<'m, 't> {
    /// The innermost enclosing class.
    pub fn innermost(&self) -> &Closed<'m, 't> {
        &self.classes[0]
    }

    /// The call `call` of `name` with `nargs` arguments and no qualifier
    /// finds no method in any enclosing class, nor through a static import:
    /// none of the name, or only methods private to other top-level classes
    /// (a superclass's private method is no member: the search goes on
    /// outward).
    pub fn method_missing(&self, call: Node<'t>, name: &str, nargs: usize) -> Option<InScope> {
        let fm = self.classes[0].fm;
        if fm.stub.imports.iter().any(|i| i.is_static && (i.on_demand || i.is_named(name))) {
            return None;
        }
        let mut private_hit = None;
        for c in &self.classes {
            let ms = fm.methods_named(&c.class.this_type(), name);
            if !ms.is_empty() {
                match private_elsewhere(fm, call, &c.class, name, &ms, None) {
                    Some(m) => {
                        private_hit.get_or_insert(m);
                    }
                    None => return None,
                }
            }
        }
        let inner = &self.classes[0].class.name;
        // the claim: every overload that fits is private, not the one shown
        Some(match private_hit {
            Some(method) => InScope::Private(Private { claim: Claim::new(format!("`{name}` with {nargs} argument(s) is private in `{inner}` and its enclosing classes")), method }),
            None => InScope::NotDefined(Claim::new(format!("Method `{name}` is not defined in `{inner}` or its enclosing classes"))),
        })
    }
}

/// The proof that this file compiles for one Java release exactly (see
/// [`FileModel::java_release`]).
pub struct Release<'m, 't> {
    fm: &'m FileModel<'t>,
    release: u32,
}

impl<'m, 't> Release<'m, 't> {
    pub fn release(&self) -> u32 {
        self.release
    }

    /// The `java.*` class or package `import` names is not public API in the
    /// release: a class added in a later release, one removed since, or one
    /// the JDK does not export. Not when the sources or a jar have it, or
    /// the release's API cannot be read, or a JDK cut down by jlink lacks
    /// the package with its module. The claim is the message, already
    /// qualified.
    pub fn lacks_import(&self, import: Node<'t>) -> Option<Claim> {
        let fm = self.fm;
        let index = fm.index;
        let cp = &index.classpath;
        let release = self.release;
        // `import module java.base;` (JEP 511) names a module, not a class
        let mut c = import.walk();
        if import.children(&mut c).any(|k| !k.is_named() && k.kind() == "module") {
            return None;
        }
        // `static` and `*` are tokens, not named children
        let words: String = fm.text(import).split_whitespace().collect::<Vec<_>>().join(" ");
        let is_static = words.starts_with("import static ");
        let on_demand = words.contains('*');
        let path = import.kids().into_iter().find(|k| matches!(k.kind(), "scoped_identifier" | "identifier"))?;
        let text: String = fm.text(path).chars().filter(|c| !c.is_whitespace()).collect();
        if !text.starts_with("java.") {
            return None;
        }
        let segs: Vec<&str> = text.split('.').collect();
        // the type the import names: all of it, or all but the static member
        let ty = if is_static && !on_demand { &segs[..segs.len() - 1] } else { &segs[..] };
        // in the sources, a jar, or the release's API; None: that API cannot be read
        let has_class = |name: &str| if index.is_source_class(name) || cp.contains_outside_jdk(name) { Some(true) } else { cp.jdk_has_class(release, name) };
        let has_package = |p: &str| if index.has_source_package(p) || cp.has_package_outside_jdk(p) { Some(true) } else { cp.jdk_has_package(release, p) };
        // `a.b.Outer.Inner`: some prefix is the package
        let class_at = |k: usize| format!("{}.{}", ty[..k].join("."), ty[k..].join("$"));
        for k in 1..ty.len() {
            if has_class(&class_at(k)) != Some(false) {
                return None;
            }
        }
        // `import java.util.*;` names a package
        if on_demand && !is_static && has_package(&ty.join(".")) != Some(false) {
            return None;
        }
        // the package, by convention the lowercase segments: a class missing
        // from a package the JDK has, or a package missing from the release
        let pkg_len = ty.iter().take_while(|s| s.starts_with(|c: char| c.is_ascii_lowercase())).count();
        let what = if on_demand && !is_static { format!("package `{}`", ty.join(".")) } else { format!("`{}`", ty.join(".")) };
        // a JDK cut down by jlink lacks whole modules, so their packages
        match has_package(&ty[..pkg_len].join(".")) {
            None => return None,
            Some(false) if !cp.jdk_is_full() => return None,
            _ => {}
        }
        Some(Claim::new(format!("{what} is not public API in Java {release}")))
    }
}

/// The method `site` would reach in `cls` when every candidate (of `nargs`
/// arguments, if given) is private to another top-level class: nested classes
/// share their top-level class's private members. `methods_named` may hide a
/// supertype's method (an interface default) behind a superclass's private
/// one, which is not inherited.
fn private_elsewhere<'t>(fm: &FileModel<'t>, site: Node<'t>, cls: &ClassInfo, name: &str, ms: &[MethodRef], nargs: Option<usize>) -> Option<MethodRef> {
    let top = |c: &str| c.split('$').next().unwrap_or(c).to_string();
    let here = fm.enclosing_class_name(site).map(|e| top(&e))?;
    let fits = |m: &MethodInfo| nargs.is_none_or(|k| m.accepts_arity(k));
    let cands: Vec<&MethodRef> = ms.iter().filter(|m| fits(&m.method)).collect();
    if cands.is_empty() || !cands.iter().all(|m| m.method.flags.is_private() && top(&m.owner) != here) {
        return None;
    }
    // a class defined in several modules: the index holds one of them
    if fm.index.is_duplicated(&cls.name) || cands.iter().any(|m| fm.index.is_duplicated(&m.owner)) {
        return None;
    }
    let visible_elsewhere = fm.all_supertypes(&cls.this_type()).iter().filter_map(|t| t.class_name().and_then(|n| fm.class(n))).any(|c| c.methods_named(name).any(|m| fits(m) && !m.flags.is_private()));
    (!visible_elsewhere).then(|| cands[0].clone())
}

/// Every class the type mentions (type arguments included) is known.
fn fully_known(fm: &FileModel<'_>, t: &Ty) -> bool {
    match t {
        Ty::Class { name, args } => fm.class(name).is_some() && args.iter().all(|a| fully_known(fm, a)),
        Ty::Array(e) => fully_known(fm, e),
        Ty::Wildcard(Some(b)) => match &**b {
            WildcardBound::Extends(t) | WildcardBound::Super(t) => fully_known(fm, t),
        },
        _ => true,
    }
}

/// If every overload with a matching arity has a parameter that definitely
/// cannot accept the corresponding argument, return the argument types.
fn definitely_incompatible<'t>(fm: &FileModel<'t>, call: Node<'t>, ms: &[MethodRef]) -> Option<Vec<Ty>> {
    let args = fm.args_of(call);
    let arg_tys: Vec<Ty> = args.iter().map(|a| fm.type_of(*a)).collect();
    // lambdas, method refs, unknown or generic argument types: not decidable
    if args.iter().any(|a| matches!(a.unparen().kind(), "lambda_expression" | "method_reference")) || arg_tys.iter().any(|t| t.has_unknown() || t.mentions_type_var() || *t == Ty::Null) {
        return None;
    }
    let mut any = false;
    for m in ms.iter().filter(|m| m.method.accepts_arity(args.len())) {
        any = true;
        let np = m.method.params.len();
        let ok = arg_tys.iter().enumerate().all(|(i, at)| {
            if m.method.is_varargs() && i + 1 >= np {
                let last = &m.method.params[np - 1].ty;
                let Ty::Array(e) = last else { return accepts(fm, at, last) };
                // one argument in the varargs position: the array itself
                // (`of(byte[][])`) or one element of it (`of(byte[])` for
                // `of(byte[]...)`, `int[]` for `Object...`)
                return (args.len() == np && accepts(fm, at, last)) || accepts(fm, at, e);
            }
            accepts(fm, at, &m.method.params[i].ty)
        });
        if ok {
            return None;
        }
    }
    if !any {
        return None;
    }
    Some(arg_tys)
}

/// The argument type may be passed for the parameter type: not provably wrong.
fn accepts(fm: &FileModel<'_>, at: &Ty, pt: &Ty) -> bool {
    if pt.has_unknown() || pt.mentions_type_var() {
        return true;
    }
    // a type whose class is not on the classpath, or the same simple
    // name from another package (javax vs jakarta when cached jars do
    // not match the project's versions), proves nothing
    if !fully_known(fm, pt) || !fully_known(fm, at) {
        return true;
    }
    // skew is only possible for jar classes, not between project classes
    // (arrays of them too, with the same number of dimensions)
    let (mut ab, mut pb) = (at, pt);
    while let (Ty::Array(a), Ty::Array(p)) = (ab, pb) {
        (ab, pb) = (&**a, &**p);
    }
    if let (Some(a), Some(p)) = (ab.class_name(), pb.class_name()) {
        let from_jar = |n: &str| fm.class(n).is_some_and(|c| !c.is_source());
        if a != p && simple_name_of(a) == simple_name_of(p) && (from_jar(a) || from_jar(p)) {
            return true;
        }
    }
    // compile-time narrowing of int constants (byte b = 1 style) is not applied to calls,
    // but keep constants permissive anyway
    !fm.is_assignable(at, pt).is_no()
}

/// Whether `m`, declared in `cls`, has the erased parameter types of `sup`
/// as a member of the supertype (`map` substitutes its type arguments): a
/// method overrides one whose signature's erasure is its own (JLS 8.4.2,
/// 8.4.8.1), as `get(Object[])` does `get(A)` of `Ops<T[]>`, or `save(Book)`
/// does `<S extends E> S save(S)` with `E` = `Book`. Type variables erase to
/// their first bound (JLS 4.6). False when a type is not known.
fn same_erasure(m: &MethodInfo, cls: &ClassInfo, sup: &MethodInfo, map: &[(Name, Ty)]) -> bool {
    fn erase(t: &Ty, vars: &[TypeParam], depth: u32) -> Ty {
        match t {
            Ty::TypeVar(v) => match vars.iter().find(|p| p.name == *v) {
                Some(p) if depth < 8 => p.bounds.first().map(|b| erase(b, vars, depth + 1)).unwrap_or_else(Ty::object),
                _ => Ty::Unknown,
            },
            Ty::Array(e) => match erase(e, vars, depth) {
                Ty::Unknown => Ty::Unknown,
                e => Ty::Array(Box::new(e)),
            },
            t => t.erasure(),
        }
    }
    // the supertype's arguments are in terms of `cls`'s type variables; the
    // supertype method's own type variables keep their (substituted) bounds
    let sup_vars: Vec<TypeParam> = sup
        .type_params
        .iter()
        .map(|p| TypeParam { name: p.name.clone(), bounds: p.bounds.iter().map(|b| apply_subst(b, map)).collect() })
        .chain(cls.type_params.iter().cloned())
        .collect();
    // a type variable of the supertype method substituted by `map` would be wrong
    let shadow: Vec<(Name, Ty)> = map.iter().filter(|(k, _)| !sup.type_params.iter().any(|p| p.name == *k)).cloned().collect();
    sup.params.len() == m.params.len()
        && sup.params.iter().zip(&m.params).all(|(a, b)| {
            // the overrider's own signature against the erased one: `m(T)`
            // with `T extends Date` does not override `m(Date)` (a clash)
            let x = erase(&apply_subst(&a.ty, &shadow), &sup_vars, 0);
            !x.has_unknown() && x == b.ty
        })
}

/// Whether `cls` or a superclass implements `name` with `arity` parameters,
/// or a default method of an interface in its closure does.
fn implements(fm: &FileModel<'_>, cls: &ClassInfo, name: &str, arity: usize) -> bool {
    let concrete = |c: &ClassInfo| c.methods.iter().any(|x| &*x.name == name && x.params.len() == arity && !x.flags.is_abstract() && !x.flags.is_static());
    // superclass chain
    let mut cur = Some(Arc::new(cls.clone()));
    let mut depth = 0;
    while let Some(c) = cur {
        if concrete(&c) {
            return true;
        }
        depth += 1;
        if depth > 30 {
            return true;
        }
        cur = c.superclass.as_ref().and_then(|s| s.class_name()).and_then(|s| fm.class(s));
    }
    // default methods anywhere in the interface closure
    fm.all_supertypes(&cls.this_type()).iter().filter_map(|t| t.class_name()).filter_map(|n| fm.class(n)).any(|c| c.is_interface() && concrete(&c))
}
