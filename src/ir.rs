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
}
pub fn lower(s: &Stmt) -> Vec<Ir> {
    let mut l = Lower {
        code: vec![],
        next: 0,
        label: 0,
        scopes: vec![],
    };
    l.stmt(s);
    l.code
}
struct Lower {
    code: Vec<Ir>,
    next: usize,
    label: usize,
    scopes: Vec<HashMap<String, usize>>,
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
                self.code.push(Ir::Print(f.clone(), regs, d));
            }
        }
        d
    }
    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(v) => {
                self.scopes.push(HashMap::new());
                for s in v {
                    self.stmt(s);
                }
                self.scopes.pop();
            }
            Stmt::Decl(t, e) => {
                let d = self.temp();
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
                self.stmt(b);
                self.code.push(Ir::Jump(start));
                self.code.push(Ir::Label(end));
            }
            Stmt::For(a, e, b, body) => {
                self.scopes.push(HashMap::new());
                self.stmt(a);
                let start = self.label();
                let end = self.label();
                self.code.push(Ir::Label(start));
                let r = self.expr(e);
                self.code.push(Ir::Zero(r, end));
                self.stmt(body);
                self.stmt(b);
                self.code.push(Ir::Jump(start));
                self.code.push(Ir::Label(end));
                self.scopes.pop();
            }
            Stmt::Return(..) => self.code.push(Ir::Halt),
            Stmt::Empty => (),
        }
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
