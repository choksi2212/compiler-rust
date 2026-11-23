use crate::parser::{E, Expr, Stmt};
use std::collections::HashSet;
pub fn check(s: &Stmt) -> Result<(), String> {
    let mut c = Checker {
        scopes: vec![HashSet::new()],
        loops: 0,
        breaks: 0,
        switches: vec![],
        labels: HashSet::new(),
        gotos: vec![],
    };
    c.stmt(s)?;
    for token in &c.gotos {
        if !c.labels.contains(&token.text) {
            return Err(format!(
                "{}:{}: undefined label {}",
                token.line, token.column, token.text
            ));
        }
    }
    Ok(())
}
struct Checker {
    scopes: Vec<HashSet<String>>,
    loops: usize,
    breaks: usize,
    switches: Vec<(HashSet<i32>, bool)>,
    labels: HashSet<String>,
    gotos: Vec<crate::lexer::Token>,
}
impl Checker {
    fn fail<T>(&self, e: &Expr, msg: &str) -> Result<T, String> {
        Err(format!("{}:{}: {msg}", e.token.line, e.token.column))
    }
    fn expr(&self, e: &Expr) -> Result<bool, String> {
        match &e.kind {
            E::Int(_) => Ok(false),
            E::Str(_) => Ok(true),
            E::Var(n) => {
                if !self.scopes.iter().rev().any(|s| s.contains(n)) {
                    return self.fail(e, "undeclared variable");
                }
                Ok(false)
            }
            E::Assign(_, a, b) => {
                self.lvalue(a)?;
                self.integer(b)?;
                Ok(false)
            }
            E::Update(_, a, _) => {
                self.lvalue(a)?;
                Ok(false)
            }
            E::Unary(_, a) => {
                if self.expr(a)? {
                    return self.fail(e, "integer operand required");
                }
                Ok(false)
            }
            E::Binary(_, a, b) => {
                if self.expr(a)? || self.expr(b)? {
                    return self.fail(e, "integer operands required");
                }
                Ok(false)
            }
            E::Call(n, _) if self.scopes.iter().rev().any(|s| s.contains(n)) => {
                self.fail(e, "called name is a variable")
            }
            E::Call(n, args) if n == "strlen" => {
                if args.len() != 1 || !matches!(&args[0].kind, E::Str(_)) {
                    return self.fail(e, "strlen requires one string literal");
                }
                Ok(false)
            }
            E::Call(n, args) if n == "printf" => {
                let Some(Expr {
                    kind: E::Str(f), ..
                }) = args.first()
                else {
                    return self.fail(e, "printf requires a literal format");
                };
                let mut specs = vec![];
                let mut chars = f.split('\0').next().unwrap().chars();
                while let Some(c) = chars.next() {
                    if c == '%' {
                        match chars.next() {
                            Some('d') => specs.push(false),
                            Some('s') => specs.push(true),
                            _ => return self.fail(e, "supported conversions are %d and %s"),
                        }
                    }
                }
                if specs.len() != args.len() - 1 {
                    return self.fail(e, "printf argument count mismatch");
                }
                for (arg, string) in args[1..].iter().zip(specs) {
                    if self.expr(arg)? != string {
                        return self.fail(arg, "printf argument type mismatch");
                    }
                }
                Ok(false)
            }
            E::Call(..) => self.fail(e, "unsupported function"),
        }
    }
    fn lvalue(&self, e: &Expr) -> Result<(), String> {
        if !matches!(e.kind, E::Var(_)) {
            return self.fail(e, "modifiable int variable required");
        }
        self.integer(e)
    }
    fn integer(&self, e: &Expr) -> Result<(), String> {
        if self.expr(e)? {
            self.fail(e, "integer expression required")
        } else {
            Ok(())
        }
    }
    fn stmt(&mut self, s: &Stmt) -> Result<(), String> {
        match s {
            Stmt::Block(v) => {
                self.scopes.push(HashSet::new());
                for s in v {
                    self.stmt(s)?;
                }
                self.scopes.pop();
            }
            Stmt::Decl(t, e) => {
                if self.scopes.last().unwrap().contains(&t.text) {
                    return Err(format!("{}:{}: duplicate declaration", t.line, t.column));
                }
                self.scopes.last_mut().unwrap().insert(t.text.clone());
                if let Some(e) = e {
                    self.integer(e)?;
                }
            }
            Stmt::Assign(t, e) => {
                if !self.scopes.iter().rev().any(|s| s.contains(&t.text)) {
                    return Err(format!(
                        "{}:{}: assignment to undeclared variable",
                        t.line, t.column
                    ));
                }
                self.integer(e)?;
            }
            Stmt::Call(e) => {
                self.expr(e)?;
            }
            Stmt::If(e, a, b) => {
                self.integer(e)?;
                self.stmt(a)?;
                if let Some(b) = b {
                    self.stmt(b)?;
                }
            }
            Stmt::While(e, b) => {
                self.integer(e)?;
                self.loops += 1;
                self.breaks += 1;
                self.stmt(b)?;
                self.loops -= 1;
                self.breaks -= 1;
            }
            Stmt::For(a, e, b, body) => {
                self.scopes.push(HashSet::new());
                self.stmt(a)?;
                self.integer(e)?;
                self.stmt(b)?;
                self.loops += 1;
                self.breaks += 1;
                self.stmt(body)?;
                self.loops -= 1;
                self.breaks -= 1;
                self.scopes.pop();
            }
            Stmt::Return(_, e) => self.integer(e)?,
            Stmt::Do(body, e) => {
                self.loops += 1;
                self.breaks += 1;
                self.stmt(body)?;
                self.integer(e)?;
                self.loops -= 1;
                self.breaks -= 1;
            }
            Stmt::Break(t) if self.breaks == 0 => {
                return Err(format!(
                    "{}:{}: break outside loop/switch",
                    t.line, t.column
                ));
            }
            Stmt::Continue(t) if self.loops == 0 => {
                return Err(format!("{}:{}: continue outside loop", t.line, t.column));
            }
            Stmt::Break(_) | Stmt::Continue(_) => (),
            Stmt::Switch(e, body) => {
                self.integer(e)?;
                self.breaks += 1;
                self.switches.push((HashSet::new(), false));
                self.stmt(body)?;
                self.switches.pop();
                self.breaks -= 1;
            }
            Stmt::Case(t, e, body) => {
                let value = e.as_ref().map(constant).transpose()?;
                let context = self.switches.last_mut().ok_or_else(|| {
                    format!("{}:{}: case/default outside switch", t.line, t.column)
                })?;
                if let Some(value) = value {
                    if !context.0.insert(value) {
                        return Err(format!("{}:{}: duplicate case value", t.line, t.column));
                    }
                } else {
                    if context.1 {
                        return Err(format!("{}:{}: duplicate default", t.line, t.column));
                    }
                    context.1 = true;
                }
                self.stmt(body)?;
            }
            Stmt::Label(t, body) => {
                if !self.labels.insert(t.text.clone()) {
                    return Err(format!("{}:{}: duplicate label", t.line, t.column));
                }
                self.stmt(body)?;
            }
            Stmt::Goto(t) => self.gotos.push(t.clone()),
            Stmt::Empty => (),
        };
        Ok(())
    }
}
/// Current int-only integer constant expression evaluator for case labels.
pub fn constant(e: &Expr) -> Result<i32, String> {
    let result = match &e.kind {
        E::Int(n) => Ok(*n),
        E::Unary(op, a) => {
            let value = crate::optimize::unary(op, crate::ir::Value::Int(constant(a)?))?;
            if let crate::ir::Value::Int(value) = value {
                Ok(value)
            } else {
                Err("integer constant required".into())
            }
        }
        E::Binary(op, a, b) => {
            // Both operands must be constant expressions even when short circuited.
            constant_shape(a)?;
            constant_shape(b)?;
            let a = constant(a)?;
            if op == "&&" {
                if a == 0 {
                    Ok(0)
                } else {
                    Ok(i32::from(constant(b)? != 0))
                }
            } else if op == "||" {
                if a != 0 {
                    Ok(1)
                } else {
                    Ok(i32::from(constant(b)? != 0))
                }
            } else {
                crate::optimize::binary(op, a, constant(b)?)
            }
        }
        _ => Err("integer constant expression required".into()),
    };
    result.map_err(|message: String| format!("{}:{}: {message}", e.token.line, e.token.column))
}
fn constant_shape(e: &Expr) -> Result<(), String> {
    match &e.kind {
        E::Int(_) => Ok(()),
        E::Unary(_, a) => constant_shape(a),
        E::Binary(_, a, b) => {
            constant_shape(a)?;
            constant_shape(b)
        }
        _ => Err("integer constant expression required".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn run(s: &str) -> Result<(), String> {
        check(&crate::parser::parse(crate::lexer::lex(s)?)?)
    }
    #[test]
    fn valid() {
        run("int main(){int x=1;{int x=2;printf(\"%d\",x);}printf(\"%s%d\",\"ok\",strlen(\"abc\"));return 0;}").unwrap();
    }
    #[test]
    fn rejects() {
        for body in [
            "x=1;",
            "int x;int x;",
            "int x=\"hi\";",
            "printf(\"%d\",\"hi\");",
            "printf(\"%d\");",
            "unknown();",
            "int x=strlen(1);",
            "return \"str\";",
        ] {
            assert!(
                run(&format!("int main(){{{body}return 0;}}")).is_err(),
                "{body}"
            );
        }
        assert!(run("int main(){return \"str\";}").is_err());
    }
}
