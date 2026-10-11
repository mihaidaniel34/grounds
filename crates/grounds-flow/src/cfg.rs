//! Control-flow graph for one callable body (method, constructor, lambda or
//! initializer).
//!
//! Granularity is expression level: every expression node is emitted as an
//! element in evaluation order (post-order), so an analysis only needs to
//! interpret a node's *own* effect. Short-circuit operators, ternaries,
//! conditions and switches split blocks; condition outcomes are carried on
//! edges so analyses can refine facts (`x != null` true-edge => non-null).
//! Lambda bodies and local/anonymous class bodies are not inlined.

use grounds_syntax::{Node, NodeExt};

pub type BlockId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind<'t> {
    Normal,
    /// Taken when the condition evaluated to true.
    True(Node<'t>),
    False(Node<'t>),
    /// Exceptional control flow (to a catch, finally or exceptional exit),
    /// with the element that throws when it is a single one.
    Exception(Option<Node<'t>>),
}

#[derive(Clone, Debug, Default)]
pub struct Block<'t> {
    pub elems: Vec<Node<'t>>,
    pub succs: Vec<(BlockId, EdgeKind<'t>)>,
    pub preds: Vec<BlockId>,
}

#[derive(Debug)]
pub struct Cfg<'t> {
    pub blocks: Vec<Block<'t>>,
    pub entry: BlockId,
    /// Normal completion (return or falling off the end).
    pub exit: BlockId,
    /// Uncaught exception.
    pub exc_exit: BlockId,
    pub owner: Node<'t>,
    /// For each element node id: (block, index in block) of its first copy
    pub positions: rustc_hash::FxHashMap<usize, (BlockId, usize)>,
    /// Further copies of elements lowered more than once (finally bodies)
    pub copies: rustc_hash::FxHashMap<usize, Vec<(BlockId, usize)>>,
}

struct Jump {
    label: Option<String>,
    break_to: BlockId,
    continue_to: Option<BlockId>,
    /// `yield` target for switch expressions
    is_switch_expr: bool,
    /// `tries.len()` when the statement was entered: try statements above
    /// this depth are left by a jump to this target
    try_depth: usize,
}

struct TryCtx<'t> {
    catches: Vec<BlockId>,
    /// finally copy entered by exceptions (continues to outer handlers)
    finally: Option<BlockId>,
    /// finally copy entered by `return` (continues to the outer return target)
    finally_ret: Option<BlockId>,
    /// the finally body, inlined before every break/continue/yield leaving it
    finally_body: Option<Node<'t>>,
}

struct Builder<'t, 's> {
    src: &'s str,
    /// Calls known never to return normally (helpers that always throw).
    never_returns: Option<&'s dyn Fn(Node<'t>) -> bool>,
    blocks: Vec<Block<'t>>,
    cur: BlockId,
    exit: BlockId,
    exc_exit: BlockId,
    jumps: Vec<Jump>,
    tries: Vec<TryCtx<'t>>,
    pending_label: Option<String>,
}

impl<'t, 's> Builder<'t, 's> {
    fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block::default());
        self.blocks.len() - 1
    }
    fn edge(&mut self, from: BlockId, to: BlockId, k: EdgeKind<'t>) {
        if !self.blocks[from].succs.iter().any(|(t, kk)| *t == to && *kk == k) {
            self.blocks[from].succs.push((to, k));
        }
    }
    fn emit(&mut self, n: Node<'t>) {
        if may_throw(n) {
            // the exception leaves with the state *before* `n`: end the block
            // here so the solver does not apply `n` or anything after it
            // (`x = f()` where f throws never assigns x)
            let before = self.cur;
            self.exceptional_edges_from(before, Some(n));
            let next = self.new_block();
            self.edge(before, next, EdgeKind::Normal);
            self.cur = next;
        }
        let c = self.cur;
        self.blocks[c].elems.push(n);
    }
    fn exceptional_edges(&mut self, from: BlockId) {
        self.exceptional_edges_from(from, None)
    }
    fn exceptional_edges_from(&mut self, from: BlockId, thrower: Option<Node<'t>>) {
        // a try-with-resources without catch/finally handles nothing itself:
        // the exception goes on to the enclosing handlers
        let targets: Vec<BlockId> = match self.tries.iter().rev().find(|t| !t.catches.is_empty() || t.finally.is_some()) {
            Some(t) if !t.catches.is_empty() => {
                let mut v = t.catches.clone();
                if let Some(f) = t.finally {
                    v.push(f);
                }
                v
            }
            Some(t) if t.finally.is_some() => vec![t.finally.unwrap()],
            _ => vec![self.exc_exit],
        };
        for t in targets {
            self.edge(from, t, EdgeKind::Exception(thrower));
        }
    }
    /// Start a fresh (unreachable) block after an abrupt completion.
    fn dead(&mut self) {
        self.cur = self.new_block();
    }
    fn goto(&mut self, to: BlockId) {
        let c = self.cur;
        self.edge(c, to, EdgeKind::Normal);
    }

    /// Jump out of every try statement entered after `depth`, running their
    /// finally bodies innermost first (as javac inlines them), then to `to`.
    fn jump_through_finally(&mut self, depth: usize, to: BlockId) {
        let saved = std::mem::take(&mut self.tries);
        let mut i = saved.len();
        while i > depth {
            i -= 1;
            if let Some(body) = saved[i].finally_body {
                // exceptions inside the finally body go to the handlers outside it
                self.tries = saved[..i].iter().map(|t| TryCtx { catches: t.catches.clone(), finally: t.finally, finally_ret: t.finally_ret, finally_body: t.finally_body }).collect();
                self.stmt(body);
            }
        }
        self.tries = saved;
        self.goto(to);
    }

    /// Where a `return` goes: the innermost finally's return copy.
    fn finally_target(&self) -> Option<BlockId> {
        self.tries.iter().rev().find_map(|t| t.finally_ret)
    }

    // ----- expressions -----

    fn expr(&mut self, n: Node<'t>) {
        match n.kind() {
            "parenthesized_expression" => {
                if let Some(k) = n.first_kid() {
                    self.expr(k);
                }
            }
            "binary_expression" if matches!(n.field("operator").map(|o| o.kind()), Some("&&") | Some("||")) => {
                let t = self.new_block();
                let f = self.new_block();
                let join = self.new_block();
                self.cond(n, t, f);
                self.cur = t;
                self.goto(join);
                self.cur = f;
                self.goto(join);
                self.cur = join;
                self.emit(n);
            }
            "ternary_expression" => {
                let (Some(c), Some(a), Some(b)) = (n.field("condition"), n.field("consequence"), n.field("alternative")) else {
                    self.emit(n);
                    return;
                };
                let t = self.new_block();
                let f = self.new_block();
                let join = self.new_block();
                self.cond(c, t, f);
                self.cur = t;
                self.expr(a);
                self.goto(join);
                self.cur = f;
                self.expr(b);
                self.goto(join);
                self.cur = join;
                self.emit(n);
            }
            "switch_expression" => {
                self.switch(n, true);
                self.emit(n);
            }
            "lambda_expression" | "class_body" => self.emit(n),
            "object_creation_expression" => {
                for k in n.kids() {
                    match k.kind() {
                        "argument_list" => self.expr_list(k),
                        "class_body" => self.emit(k),
                        _ if k.field("x").is_none() && n.field("type") != Some(k) && n.field("type_arguments") != Some(k)
                            // qualified creation: outer.new Inner()
                            && is_expression(k.kind()) => {
                                self.expr(k);
                            }
                        _ => {}
                    }
                }
                self.emit(n);
            }
            "method_invocation" => {
                if let Some(o) = n.field("object") {
                    if is_expression(o.kind()) {
                        self.expr(o);
                    }
                }
                if let Some(a) = n.field("arguments") {
                    self.expr_list(a);
                }
                self.emit(n);
            }
            "assignment_expression" => {
                if let Some(l) = n.field("left") {
                    // evaluate subexpressions of the target, not the target itself
                    match l.kind() {
                        "field_access" => {
                            if let Some(o) = l.field("object") {
                                if is_expression(o.kind()) {
                                    self.expr(o);
                                }
                            }
                        }
                        "array_access" => {
                            if let Some(a) = l.field("array") {
                                self.expr(a);
                            }
                            if let Some(i) = l.field("index") {
                                self.expr(i);
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(r) = n.field("right") {
                    self.expr(r);
                }
                self.emit(n);
            }
            "field_access" => {
                if let Some(o) = n.field("object") {
                    if is_expression(o.kind()) {
                        self.expr(o);
                    }
                }
                self.emit(n);
            }
            "method_reference" => {
                if let Some(f) = n.first_kid() {
                    if is_expression(f.kind()) && f.kind() != "identifier" {
                        self.expr(f);
                    } else if f.kind() == "identifier" {
                        self.emit(f);
                    }
                }
                self.emit(n);
            }
            "array_creation_expression" => {
                for k in n.kids() {
                    match k.kind() {
                        "dimensions_expr" => {
                            for e in k.kids() {
                                if is_expression(e.kind()) {
                                    self.expr(e);
                                }
                            }
                        }
                        "array_initializer" => self.expr(k),
                        _ => {}
                    }
                }
                self.emit(n);
            }
            "array_initializer" => {
                for k in n.kids() {
                    self.expr(k);
                }
                self.emit(n);
            }
            "cast_expression" => {
                if let Some(v) = n.field("value") {
                    self.expr(v);
                }
                self.emit(n);
            }
            "instanceof_expression" => {
                if let Some(l) = n.field("left") {
                    self.expr(l);
                }
                self.emit(n);
            }
            "class_literal" | "this" | "super" | "identifier" => self.emit(n),
            k if is_literal(k) => self.emit(n),
            _ => {
                for k in n.kids() {
                    if is_expression(k.kind()) {
                        self.expr(k);
                    }
                }
                self.emit(n);
            }
        }
    }

    fn expr_list(&mut self, n: Node<'t>) {
        for k in n.kids() {
            self.expr(k);
        }
    }

    /// Lower a condition branching to `t` / `f`.
    fn cond(&mut self, c: Node<'t>, t: BlockId, f: BlockId) {
        match c.kind() {
            "parenthesized_expression" => {
                if let Some(k) = c.first_kid() {
                    self.cond(k, t, f);
                }
            }
            "binary_expression" => match c.field("operator").map(|o| o.kind()) {
                Some("&&") => {
                    let mid = self.new_block();
                    let (Some(l), Some(r)) = (c.field("left"), c.field("right")) else { return };
                    self.cond(l, mid, f);
                    self.cur = mid;
                    self.cond(r, t, f);
                }
                Some("||") => {
                    let mid = self.new_block();
                    let (Some(l), Some(r)) = (c.field("left"), c.field("right")) else { return };
                    self.cond(l, t, mid);
                    self.cur = mid;
                    self.cond(r, t, f);
                }
                _ => self.simple_cond(c, t, f),
            },
            "unary_expression" if c.field("operator").map(|o| o.kind()) == Some("!") => {
                if let Some(o) = c.field("operand") {
                    // edges are labeled with the inner condition, swapped
                    self.cond(o, f, t);
                }
            }
            _ => self.simple_cond(c, t, f),
        }
    }

    fn simple_cond(&mut self, c: Node<'t>, t: BlockId, f: BlockId) {
        self.expr(c);
        let cur = self.cur;
        // a literal condition has one outcome (`while (true)` only exits by break)
        if c.kind() != "false" {
            self.edge(cur, t, EdgeKind::True(c));
        }
        if c.kind() != "true" {
            self.edge(cur, f, EdgeKind::False(c));
        }
    }

    // ----- statements -----

    fn stmts(&mut self, n: Node<'t>) {
        for k in n.kids() {
            self.stmt(k);
        }
    }

    fn stmt(&mut self, n: Node<'t>) {
        let label = self.pending_label.take();
        match n.kind() {
            "block" | "constructor_body" => self.stmts(n),
            "expression_statement" => {
                if let Some(e) = n.first_kid() {
                    self.expr(e);
                    // `Asserts.fail("...")`: the path ends here, like a throw
                    if e.kind() == "method_invocation" && self.never_returns.is_some_and(|f| f(e)) {
                        let c = self.cur;
                        self.exceptional_edges(c);
                        self.dead();
                    }
                }
            }
            "local_variable_declaration" => {
                for d in n.fields("declarator") {
                    if let Some(v) = d.field("value") {
                        self.expr(v);
                    }
                    self.emit(d);
                }
            }
            "explicit_constructor_invocation" => {
                if let Some(a) = n.field("arguments") {
                    self.expr_list(a);
                }
                self.emit(n);
            }
            "if_statement" => {
                let then_b = self.new_block();
                let else_b = self.new_block();
                let join = self.new_block();
                if let Some(c) = n.field("condition") {
                    self.cond(c, then_b, else_b);
                }
                self.cur = then_b;
                if let Some(s) = n.field("consequence") {
                    self.stmt(s);
                }
                self.goto(join);
                self.cur = else_b;
                if let Some(s) = n.field("alternative") {
                    self.stmt(s);
                }
                self.goto(join);
                self.cur = join;
            }
            "while_statement" => {
                let head = self.new_block();
                let body = self.new_block();
                let exit = self.new_block();
                self.goto(head);
                self.cur = head;
                if let Some(c) = n.field("condition") {
                    self.cond(c, body, exit);
                }
                self.jumps.push(Jump { label, break_to: exit, continue_to: Some(head), is_switch_expr: false, try_depth: self.tries.len() });
                self.cur = body;
                if let Some(s) = n.field("body") {
                    self.stmt(s);
                }
                self.goto(head);
                self.jumps.pop();
                self.cur = exit;
            }
            "do_statement" => {
                let body = self.new_block();
                let cond_b = self.new_block();
                let exit = self.new_block();
                self.goto(body);
                self.jumps.push(Jump { label, break_to: exit, continue_to: Some(cond_b), is_switch_expr: false, try_depth: self.tries.len() });
                self.cur = body;
                if let Some(s) = n.field("body") {
                    self.stmt(s);
                }
                self.goto(cond_b);
                self.jumps.pop();
                self.cur = cond_b;
                if let Some(c) = n.field("condition") {
                    self.cond(c, body, exit);
                }
                self.cur = exit;
            }
            "for_statement" => {
                for i in n.fields("init") {
                    if i.kind() == "local_variable_declaration" {
                        self.stmt(i);
                    } else {
                        self.expr(i);
                    }
                }
                let head = self.new_block();
                let body = self.new_block();
                let update = self.new_block();
                let exit = self.new_block();
                self.goto(head);
                self.cur = head;
                match n.field("condition") {
                    Some(c) => self.cond(c, body, exit),
                    None => self.goto(body),
                }
                self.jumps.push(Jump { label, break_to: exit, continue_to: Some(update), is_switch_expr: false, try_depth: self.tries.len() });
                self.cur = body;
                if let Some(s) = n.field("body") {
                    self.stmt(s);
                }
                self.goto(update);
                self.jumps.pop();
                self.cur = update;
                for u in n.fields("update") {
                    self.expr(u);
                }
                self.goto(head);
                self.cur = exit;
            }
            "enhanced_for_statement" => {
                if let Some(v) = n.field("value") {
                    self.expr(v);
                }
                let head = self.new_block();
                let body = self.new_block();
                let exit = self.new_block();
                self.goto(head);
                self.cur = head;
                self.edge(head, exit, EdgeKind::Normal);
                self.edge(head, body, EdgeKind::Normal);
                self.cur = body;
                // the loop variable is (re)defined here
                self.emit(n);
                self.jumps.push(Jump { label, break_to: exit, continue_to: Some(head), is_switch_expr: false, try_depth: self.tries.len() });
                if let Some(s) = n.field("body") {
                    self.stmt(s);
                }
                self.goto(head);
                self.jumps.pop();
                self.cur = exit;
            }
            "labeled_statement" => {
                let name = n.kids().into_iter().find(|k| k.kind() == "identifier").map(|i| i.text(self.src).to_string());
                if let Some(s) = n.kids().into_iter().find(|k| k.kind() != "identifier") {
                    let is_loop = matches!(s.kind(), "while_statement" | "do_statement" | "for_statement" | "enhanced_for_statement");
                    if is_loop {
                        self.pending_label = name;
                        self.stmt(s);
                    } else {
                        let exit = self.new_block();
                        self.jumps.push(Jump { label: name, break_to: exit, continue_to: None, is_switch_expr: false, try_depth: self.tries.len() });
                        self.stmt(s);
                        self.jumps.pop();
                        self.goto(exit);
                        self.cur = exit;
                    }
                }
            }
            "switch_expression" => {
                // switch statement
                self.switch(n, false);
            }
            "return_statement" => {
                if let Some(e) = n.first_kid() {
                    self.expr(e);
                }
                self.emit(n);
                let target = self.finally_target().unwrap_or(self.exit);
                self.goto(target);
                self.dead();
            }
            "throw_statement" => {
                if let Some(e) = n.first_kid() {
                    self.expr(e);
                }
                let c = self.cur;
                self.blocks[c].elems.push(n);
                self.exceptional_edges(c);
                self.dead();
            }
            "break_statement" => {
                let lbl = n.kids().into_iter().find(|k| k.kind() == "identifier").map(|i| i.text(self.src).to_string());
                let target = match &lbl {
                    Some(l) => self.jumps.iter().rev().find(|j| j.label.as_deref() == Some(l)).map(|j| (j.break_to, j.try_depth)),
                    None => self.jumps.iter().rev().find(|j| !j.is_switch_expr).map(|j| (j.break_to, j.try_depth)),
                };
                self.emit(n);
                if let Some((t, depth)) = target {
                    self.jump_through_finally(depth, t);
                }
                self.dead();
            }
            "continue_statement" => {
                let lbl = n.kids().into_iter().find(|k| k.kind() == "identifier").map(|i| i.text(self.src).to_string());
                let target = match &lbl {
                    Some(l) => self.jumps.iter().rev().find(|j| j.label.as_deref() == Some(l)).and_then(|j| j.continue_to.map(|c| (c, j.try_depth))),
                    None => self.jumps.iter().rev().find_map(|j| j.continue_to.map(|c| (c, j.try_depth))),
                };
                self.emit(n);
                if let Some((t, depth)) = target {
                    self.jump_through_finally(depth, t);
                }
                self.dead();
            }
            "yield_statement" => {
                if let Some(e) = n.first_kid() {
                    self.expr(e);
                }
                self.emit(n);
                if let Some((t, depth)) = self.jumps.iter().rev().find(|j| j.is_switch_expr).map(|j| (j.break_to, j.try_depth)) {
                    self.jump_through_finally(depth, t);
                }
                self.dead();
            }
            "synchronized_statement" => {
                if let Some(p) = n.kid_of_kind("parenthesized_expression") {
                    self.expr(p);
                }
                self.emit(n);
                if let Some(b) = n.field("body") {
                    self.stmt(b);
                }
            }
            "try_statement" | "try_with_resources_statement" => self.try_stmt(n),
            "assert_statement" => {
                // assertions may be disabled: both paths continue
                let kids = n.kids();
                if let Some(c) = kids.first() {
                    let t = self.new_block();
                    let f = self.new_block();
                    let join = self.new_block();
                    let skip = self.cur;
                    self.cond(*c, t, f);
                    self.cur = f;
                    if let Some(m) = kids.get(1) {
                        self.expr(*m);
                    }
                    let fc = self.cur;
                    self.blocks[fc].elems.push(n);
                    self.exceptional_edges(fc);
                    // analyses assume assertions are enabled: only the passing edge continues
                    let _ = skip;
                    self.cur = t;
                    self.goto(join);
                    self.cur = join;
                }
            }
            ";" => {}
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" => {}
            _ => {
                if is_expression(n.kind()) {
                    self.expr(n);
                }
            }
        }
    }

    fn switch(&mut self, n: Node<'t>, is_expr: bool) {
        if let Some(c) = n.field("condition") {
            self.expr(c);
        }
        let dispatch = self.cur;
        let exit = self.new_block();
        let Some(body) = n.field("body") else {
            self.goto(exit);
            self.cur = exit;
            return;
        };
        self.jumps.push(Jump { label: None, break_to: exit, continue_to: None, is_switch_expr: is_expr, try_depth: self.tries.len() });
        let mut has_default = false;
        let mut prev_end: Option<BlockId> = None;
        for group in body.kids() {
            let is_rule = group.kind() == "switch_rule";
            let labels: Vec<Node> = group.kids_of_kind("switch_label");
            if labels.iter().any(|l| l.text(self.src).trim_start().starts_with("default") || l.kids().is_empty()) {
                has_default = true;
            }
            // case patterns with `default` keyword in `case null, default`
            if labels.iter().any(|l| l.text(self.src).contains("default")) {
                has_default = true;
            }
            let start = self.new_block();
            self.edge(dispatch, start, EdgeKind::Normal);
            if let Some(p) = prev_end {
                // fallthrough from previous group
                self.edge(p, start, EdgeKind::Normal);
            }
            self.cur = start;
            for l in &labels {
                self.emit(*l);
            }
            for k in group.kids() {
                if k.kind() == "switch_label" {
                    continue;
                }
                if is_rule {
                    match k.kind() {
                        "expression_statement" => {
                            if let Some(e) = k.first_kid() {
                                self.expr(e);
                                if is_expr {
                                    self.emit(k);
                                }
                            }
                        }
                        _ => self.stmt(k),
                    }
                } else {
                    self.stmt(k);
                }
            }
            if is_rule {
                self.goto(exit);
                prev_end = None;
            } else {
                prev_end = Some(self.cur);
            }
        }
        if let Some(p) = prev_end {
            self.edge(p, exit, EdgeKind::Normal);
        }
        // enum/sealed exhaustiveness is unknown: assume a path to exit unless default
        if !has_default && !is_expr {
            self.edge(dispatch, exit, EdgeKind::Normal);
        }
        self.jumps.pop();
        self.cur = exit;
    }

    fn try_stmt(&mut self, n: Node<'t>) {
        let catches: Vec<Node> = n.kids_of_kind("catch_clause");
        let finally = n.kid_of_kind("finally_clause").and_then(|f| f.kid_of_kind("block"));
        let catch_blocks: Vec<BlockId> = catches.iter().map(|_| self.new_block()).collect();
        // The finally body is lowered once per way of leaving the statement,
        // so their states never mix: normal completion (continues after the
        // statement), exception (continues to the outer handlers) and return
        // (continues to the outer return target). break/continue/yield
        // inline their own copy (see jump_through_finally).
        let finally_normal = finally.map(|_| self.new_block());
        let finally_abrupt = finally.map(|_| self.new_block());
        let finally_ret = finally.map(|_| self.new_block());
        let after = self.new_block();
        self.tries.push(TryCtx { catches: catch_blocks.clone(), finally: finally_abrupt, finally_ret, finally_body: finally });
        if let Some(rs) = n.field("resources") {
            for r in rs.kids() {
                if let Some(v) = r.field("value") {
                    self.expr(v);
                }
                self.emit(r);
            }
        }
        if let Some(b) = n.field("body") {
            self.stmt(b);
        }
        self.tries.pop();
        let body_end = self.cur;
        self.edge(body_end, finally_normal.unwrap_or(after), EdgeKind::Normal);
        // catch clauses: exceptions inside them go to the abrupt finally
        self.tries.push(TryCtx { catches: Vec::new(), finally: finally_abrupt, finally_ret, finally_body: finally });
        for (c, b) in catches.iter().zip(&catch_blocks) {
            self.cur = *b;
            if let Some(p) = c.kid_of_kind("catch_formal_parameter") {
                self.emit(p);
            }
            if let Some(body) = c.field("body") {
                self.stmt(body);
            }
            let end = self.cur;
            self.edge(end, finally_normal.unwrap_or(after), EdgeKind::Normal);
        }
        self.tries.pop();
        if let (Some(fb), Some(fnb), Some(fab), Some(frb)) = (finally, finally_normal, finally_abrupt, finally_ret) {
            // normal copy
            self.cur = fnb;
            self.stmt(fb);
            let end = self.cur;
            self.edge(end, after, EdgeKind::Normal);
            // exception copy: the exception keeps propagating
            self.cur = fab;
            self.stmt(fb);
            let end = self.cur;
            self.exceptional_edges(end);
            // return copy
            self.cur = frb;
            self.stmt(fb);
            let end = self.cur;
            let outer = self.finally_target().unwrap_or(self.exit);
            self.edge(end, outer, EdgeKind::Normal);
        }
        self.cur = after;
    }
}

pub fn is_literal(k: &str) -> bool {
    matches!(
        k,
        "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "character_literal"
            | "string_literal"
            | "true"
            | "false"
            | "null_literal"
            | "template_expression"
    )
}

pub fn is_expression(k: &str) -> bool {
    is_literal(k)
        || matches!(
            k,
            "assignment_expression"
                | "binary_expression"
                | "cast_expression"
                | "instanceof_expression"
                | "lambda_expression"
                | "ternary_expression"
                | "unary_expression"
                | "update_expression"
                | "switch_expression"
                | "array_access"
                | "array_creation_expression"
                | "class_literal"
                | "field_access"
                | "identifier"
                | "method_invocation"
                | "method_reference"
                | "object_creation_expression"
                | "parenthesized_expression"
                | "this"
                | "array_initializer"
        )
}

fn may_throw(n: Node) -> bool {
    matches!(n.kind(), "method_invocation" | "object_creation_expression" | "explicit_constructor_invocation" | "array_access" | "cast_expression")
}

/// Build the CFG for a callable: method/constructor/lambda declaration or an
/// initializer block.
pub fn build<'t>(owner: Node<'t>, src: &str) -> Cfg<'t> {
    build_with(owner, src, None)
}

/// Like [`build`], with a predicate for calls that never return normally.
pub fn build_with<'t, 's>(owner: Node<'t>, src: &'s str, never_returns: Option<&'s dyn Fn(Node<'t>) -> bool>) -> Cfg<'t> {
    let mut b = Builder { src, never_returns, blocks: Vec::new(), cur: 0, exit: 0, exc_exit: 0, jumps: Vec::new(), tries: Vec::new(), pending_label: None };
    let entry = b.new_block();
    b.exit = b.new_block();
    b.exc_exit = b.new_block();
    b.cur = entry;
    let body = match owner.kind() {
        "lambda_expression" => owner.field("body"),
        "static_initializer" => owner.kid_of_kind("block"),
        "block" => Some(owner),
        _ => owner.field("body"),
    };
    if let Some(body) = body {
        if owner.kind() == "lambda_expression" && body.kind() != "block" {
            b.expr(body);
        } else {
            b.stmt(body);
        }
    }
    let end = b.cur;
    let exit = b.exit;
    b.edge(end, exit, EdgeKind::Normal);
    let mut blocks = b.blocks;
    for i in 0..blocks.len() {
        let succs: Vec<BlockId> = blocks[i].succs.iter().map(|(s, _)| *s).collect();
        for s in succs {
            if !blocks[s].preds.contains(&i) {
                blocks[s].preds.push(i);
            }
        }
    }
    let mut positions = rustc_hash::FxHashMap::default();
    let mut copies: rustc_hash::FxHashMap<usize, Vec<(BlockId, usize)>> = rustc_hash::FxHashMap::default();
    for (bi, bl) in blocks.iter().enumerate() {
        for (ei, e) in bl.elems.iter().enumerate() {
            if positions.contains_key(&e.id()) {
                copies.entry(e.id()).or_default().push((bi, ei));
            } else {
                positions.insert(e.id(), (bi, ei));
            }
        }
    }
    Cfg { blocks, entry, exit: b.exit, exc_exit: b.exc_exit, owner, positions, copies }
}

impl<'t> Cfg<'t> {
    pub fn succs(&self, b: BlockId) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks[b].succs.iter().map(|(s, _)| *s)
    }

    /// Blocks reachable from entry following all edges.
    pub fn reachable(&self) -> Vec<bool> {
        self.reachable_with(|_| true)
    }

    /// Reachability where `follow(edge)` decides whether an edge can be taken.
    pub fn reachable_with(&self, follow: impl Fn(&EdgeKind<'t>) -> bool) -> Vec<bool> {
        let mut seen = vec![false; self.blocks.len()];
        let mut stack = vec![self.entry];
        while let Some(b) = stack.pop() {
            if seen[b] {
                continue;
            }
            seen[b] = true;
            for (s, k) in &self.blocks[b].succs {
                if follow(k) && !seen[*s] {
                    stack.push(*s);
                }
            }
        }
        seen
    }

    pub fn contains(&self, n: Node<'t>) -> bool {
        self.positions.contains_key(&n.id())
    }

    pub fn position(&self, n: Node<'t>) -> Option<(BlockId, usize)> {
        self.positions.get(&n.id()).copied()
    }

    /// Every element in block order.
    pub fn elements(&self) -> impl Iterator<Item = Node<'t>> + '_ {
        self.blocks.iter().flat_map(|b| b.elems.iter().copied())
    }
}
