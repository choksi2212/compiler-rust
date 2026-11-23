use crate::parser::{E, Expr, Stmt};
use std::collections::HashMap;
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    Str(String),
}
#[derive(Debug, Clone)]
pub enum Ir {
    Reset(usize),
    Const(usize, Value),
    Copy(usize, usize),
    Unary(usize, String, usize),
    Binary(usize, String, usize, usize),
    Label(usize),
    Jump(usize),
    Zero(usize, usize),
    Print(String, Vec<usize>, usize),
    Halt,
    Return(usize),
}
pub fn lower(s: &Stmt) -> Vec<Ir> {
    let mut layout = Layout::default();
    layout.collect(s, &mut vec![]);
    let mut l = Lower {
        code: (0..layout.next_slot).map(Ir::Reset).collect(),
        next: layout.next_slot,
        label: layout.next_label,
        scopes: vec![HashMap::new()],
        path: vec![],
        controls: vec![],
        layout,
    };
    l.stmt(s);
    l.code.push(Ir::Halt); // Falling off main is return0.
    l.code
}
fn key(s: &Stmt) -> usize {
    s as *const Stmt as usize
}
#[derive(Default)]
struct Layout {
    next_slot: usize,
    next_label: usize,
    declarations: HashMap<usize, usize>,
    scope_slots: HashMap<usize, Vec<usize>>,
    paths: HashMap<usize, Vec<usize>>,
    names: HashMap<String, usize>,
    targets: HashMap<usize, usize>,
}
impl Layout {
    fn collect(&mut self, s: &Stmt, path: &mut Vec<usize>) {
        let scoped = matches!(s, Stmt::Block(_) | Stmt::For(..));
        if scoped {
            path.push(key(s));
        }
        self.paths.insert(key(s), path.clone());
        if matches!(s, Stmt::Decl(..)) {
            self.declarations.insert(key(s), self.next_slot);
            if let Some(scope) = path.last() {
                self.scope_slots
                    .entry(*scope)
                    .or_default()
                    .push(self.next_slot);
            }
            self.next_slot += 1;
        }
        if let Stmt::Label(token, _) = s {
            self.names.insert(token.text.clone(), key(s));
        }
        if matches!(s, Stmt::Label(..) | Stmt::Case(..)) {
            self.targets.insert(key(s), self.next_label);
            self.next_label += 1;
        }
        for child in crate::parser::children(s) {
            self.collect(child, path);
        }
        if scoped {
            path.pop();
        }
    }
}
struct Lower {
    code: Vec<Ir>,
    next: usize,
    label: usize,
    scopes: Vec<HashMap<String, usize>>,
    path: Vec<usize>,
    controls: Vec<(usize, Option<usize>)>,
    layout: Layout,
}

impl Lower {
    fn temp(&mut self) -> usize {
        let n = self.next;
        self.next += 1;
        n
    }
    fn label(&mut self) -> usize {
        let n = self.label;
        self.label += 1;
        n
    }
    fn var(&self, n: &str) -> usize {
        *self.scopes.iter().rev().find_map(|s| s.get(n)).unwrap()
    }
    fn expr(&mut self, e: &Expr) -> usize {
        let d = self.temp();
        match &e.kind {
            E::Int(n) => self.code.push(Ir::Const(d, Value::Int(*n))),
            E::Str(s) => self.code.push(Ir::Const(d, Value::Str(s.clone()))),
            E::Var(n) => {
                let v = self.var(n);
                self.code.push(Ir::Copy(d, v));
            }
            E::Assign(op, a, b) => {
                let E::Var(name) = &a.kind else {
                    unreachable!()
                };
                let target = self.var(name);
                let old = if op != "=" { Some(self.expr(a)) } else { None };
                let value = self.expr(b);
                if let Some(old) = old {
                    self.code.push(Ir::Binary(d, op[..1].into(), old, value));
                } else {
                    self.code.push(Ir::Copy(d, value));
                }
                self.code.push(Ir::Copy(target, d));
            }
            E::Update(op, a, postfix) => {
                let E::Var(name) = &a.kind else {
                    unreachable!()
                };
                let target = self.var(name);
                let old = self.expr(a);
                let one = self.temp();
                let value = self.temp();
                self.code.push(Ir::Const(one, Value::Int(1)));
                self.code.push(Ir::Binary(value, op[..1].into(), old, one));
                self.code.push(Ir::Copy(target, value));
                self.code
                    .push(Ir::Copy(d, if *postfix { old } else { value }));
            }
            E::Unary(op, a) => {
                let a = self.expr(a);
                self.code.push(Ir::Unary(d, op.clone(), a));
            }
            E::Binary(op, a, b) if op == "&&" || op == "||" => {
                let end = self.label();
                let other = self.label();
                let a = self.expr(a);
                self.code.push(Ir::Zero(a, other));
                if op == "||" {
                    self.code.push(Ir::Const(d, Value::Int(1)));
                    self.code.push(Ir::Jump(end));
                    self.code.push(Ir::Label(other));
                    let b = self.expr(b);
                    self.code.push(Ir::Unary(d, "bool".into(), b));
                } else {
                    let b = self.expr(b);
                    self.code.push(Ir::Unary(d, "bool".into(), b));
                    self.code.push(Ir::Jump(end));
                    self.code.push(Ir::Label(other));
                    self.code.push(Ir::Const(d, Value::Int(0)));
                }
                self.code.push(Ir::Label(end));
            }
            E::Binary(op, a, b) => {
                let a = self.expr(a);
                let b = self.expr(b);
                self.code.push(Ir::Binary(d, op.clone(), a, b));
            }
            E::Call(n, args) if n == "strlen" => {
                let a = self.expr(&args[0]);
                self.code.push(Ir::Unary(d, "strlen".into(), a));
            }
            E::Call(_, args) => {
                let E::Str(f) = &args[0].kind else {
                    unreachable!()
                };
                let regs = args[1..].iter().map(|a| self.expr(a)).collect();
                self.code
                    .push(Ir::Print(f.split('\0').next().unwrap().into(), regs, d));
            }
        }
        d
    }
    fn enter_target(&mut self, target: usize) {
        let target_path = &self.layout.paths[&target];
        let common = self
            .path
            .iter()
            .zip(target_path)
            .take_while(|(a, b)| a == b)
            .count();
        for scope in &target_path[common..] {
            if let Some(slots) = self.layout.scope_slots.get(scope) {
                self.code.extend(slots.iter().map(|slot| Ir::Reset(*slot)));
            }
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(v) => {
                self.path.push(key(s));
                self.scopes.push(HashMap::new());
                for s in v {
                    self.stmt(s);
                }
                self.scopes.pop();
                self.path.pop();
            }
            Stmt::Decl(t, e) => {
                let d = self.layout.declarations[&key(s)];
                self.scopes.last_mut().unwrap().insert(t.text.clone(), d);
                self.code.push(Ir::Reset(d));
                if let Some(e) = e {
                    let a = self.expr(e);
                    self.code.push(Ir::Copy(d, a));
                }
                self.scopes.last_mut().unwrap().insert(t.text.clone(), d);
            }
            Stmt::Assign(t, e) => {
                let a = self.expr(e);
                self.code.push(Ir::Copy(self.var(&t.text), a));
            }
            Stmt::Call(e) => {
                self.expr(e);
            }
            Stmt::If(e, a, b) => {
                let no = self.label();
                let end = self.label();
                let r = self.expr(e);
                self.code.push(Ir::Zero(r, no));
                self.stmt(a);
                self.code.push(Ir::Jump(end));
                self.code.push(Ir::Label(no));
                if let Some(b) = b {
                    self.stmt(b);
                }
                self.code.push(Ir::Label(end));
            }
            Stmt::While(e, b) => {
                let start = self.label();
                let end = self.label();
                self.code.push(Ir::Label(start));
                let r = self.expr(e);
                self.code.push(Ir::Zero(r, end));
                self.controls.push((end, Some(start)));
                self.stmt(b);
                self.controls.pop();
                self.code.push(Ir::Jump(start));
                self.code.push(Ir::Label(end));
            }
            Stmt::For(a, e, b, body) => {
                self.path.push(key(s));
                self.scopes.push(HashMap::new());
                self.stmt(a);
                let start = self.label();
                let end = self.label();
                self.code.push(Ir::Label(start));
                let r = self.expr(e);
                self.code.push(Ir::Zero(r, end));
                let update = self.label();
                self.controls.push((end, Some(update)));
                self.stmt(body);
                self.controls.pop();
                self.code.push(Ir::Label(update));
                self.stmt(b);
                self.code.push(Ir::Jump(start));
                self.code.push(Ir::Label(end));
                self.scopes.pop();
                self.path.pop();
            }
            Stmt::Do(body, e) => {
                let start = self.label();
                let condition = self.label();
                let end = self.label();
                self.code.push(Ir::Label(start));
                self.controls.push((end, Some(condition)));
                self.stmt(body);
                self.controls.pop();
                self.code.push(Ir::Label(condition));
                let value = self.expr(e);
                self.code.push(Ir::Zero(value, end));
                self.code.push(Ir::Jump(start));
                self.code.push(Ir::Label(end));
            }
            Stmt::Break(_) => self.code.push(Ir::Jump(self.controls.last().unwrap().0)),
            Stmt::Continue(_) => self.code.push(Ir::Jump(
                self.controls.iter().rev().find_map(|c| c.1).unwrap(),
            )),
            Stmt::Label(_, body) | Stmt::Case(_, _, body) => {
                self.code.push(Ir::Label(self.layout.targets[&key(s)]));
                self.stmt(body);
            }
            Stmt::Goto(token) => {
                let target = self.layout.names[&token.text];
                self.enter_target(target);
                self.code.push(Ir::Jump(self.layout.targets[&target]));
            }
            Stmt::Switch(e, body) => {
                let value = self.expr(e);
                let end = self.label();
                let mut cases = vec![];
                collect_cases(body, &mut cases);
                let mut default = None;
                for case in cases {
                    let Stmt::Case(_, expr, _) = case else {
                        unreachable!()
                    };
                    if let Some(expr) = expr {
                        let next = self.label();
                        let constant = self.temp();
                        let equal = self.temp();
                        self.code.push(Ir::Const(
                            constant,
                            Value::Int(crate::semantic::constant(expr).unwrap()),
                        ));
                        self.code
                            .push(Ir::Binary(equal, "==".into(), value, constant));
                        self.code.push(Ir::Zero(equal, next));
                        self.enter_target(key(case));
                        self.code.push(Ir::Jump(self.layout.targets[&key(case)]));
                        self.code.push(Ir::Label(next));
                    } else {
                        default = Some(key(case));
                    }
                }
                if let Some(default) = default {
                    self.enter_target(default);
                    self.code.push(Ir::Jump(self.layout.targets[&default]));
                } else {
                    self.code.push(Ir::Jump(end));
                }
                self.controls.push((end, None));
                self.stmt(body);
                self.controls.pop();
                self.code.push(Ir::Label(end));
            }
            Stmt::Return(_, e) => {
                if matches!(e.kind, E::Int(0)) {
                    self.code.push(Ir::Halt);
                } else {
                    let value = self.expr(e);
                    self.code.push(Ir::Return(value));
                }
            }
            Stmt::Empty => (),
        }
    }
}
fn collect_cases<'a>(s: &'a Stmt, out: &mut Vec<&'a Stmt>) {
    if matches!(s, Stmt::Switch(..)) {
        return;
    }
    if matches!(s, Stmt::Case(..)) {
        out.push(s);
    }
    for child in crate::parser::children(s) {
        collect_cases(child, out);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowers_control_flow() {
        let a = crate::parser::parse(
            crate::lexer::lex("int main(){int x=0;while(x<3){x=x+1;}return 0;}").unwrap(),
        )
        .unwrap();
        crate::semantic::check(&a).unwrap();
        let ir = lower(&a);
        assert!(
            ir.iter()
                .any(|i| matches!(i,Ir::Binary(_,op,_,_) if op=="+"))
        );
        assert_eq!(ir.iter().filter(|i| matches!(i, Ir::Label(_))).count(), 2);
        assert!(matches!(ir.last(), Some(Ir::Halt)));
    }
}
