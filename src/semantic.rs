use crate::parser::{E, Expr, Stmt};
use std::collections::HashSet;
pub fn check(s: &Stmt) -> Result<(), String> {
    let mut c = Checker { scopes: vec![] };
    c.stmt(s)?;
    if !matches!(s,Stmt::Block(v) if matches!(v.last(),Some(Stmt::Return(_,Expr{kind:E::Int(0),..}))))
    {
        return Err("1:1: main must end with return 0;".into());
    }
    Ok(())
}
struct Checker {
    scopes: Vec<HashSet<String>>,
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
                let mut chars = f.chars();
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
                if let Some(e) = e {
                    self.integer(e)?;
                }
                self.scopes.last_mut().unwrap().insert(t.text.clone());
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
                self.stmt(b)?;
            }
            Stmt::For(a, e, b, body) => {
                self.scopes.push(HashSet::new());
                self.stmt(a)?;
                self.integer(e)?;
                self.stmt(b)?;
                self.stmt(body)?;
                self.scopes.pop();
            }
            Stmt::Return(t, e) => {
                if !matches!(e.kind, E::Int(0)) {
                    return Err(format!(
                        "{}:{}: only return 0 is supported",
                        t.line, t.column
                    ));
                }
            }
            Stmt::Empty => (),
        };
        Ok(())
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
            "return 1;",
        ] {
            assert!(
                run(&format!("int main(){{{body}return 0;}}")).is_err(),
                "{body}"
            );
        }
        assert!(run("int main(){}").is_err());
    }
}
