use crate::lexer::{Kind, Token};
#[derive(Debug, Clone)]
pub struct Expr {
    pub token: Token,
    pub kind: E,
}
#[derive(Debug, Clone)]
pub enum E {
    Int(i32),
    Str(String),
    Var(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}
#[derive(Debug, Clone)]
pub enum Stmt {
    Block(Vec<Stmt>),
    Decl(Token, Option<Expr>),
    Assign(Token, Expr),
    Call(Expr),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    For(Box<Stmt>, Expr, Box<Stmt>, Box<Stmt>),
    Return(Token, Expr),
    Empty,
}
pub fn parse(tokens: Vec<Token>) -> Result<Stmt, String> {
    let mut p = Parser { tokens, i: 0 };
    p.expect("int")?;
    p.expect("main")?;
    p.expect("(")?;
    p.expect(")")?;
    let s = p.block()?;
    if p.peek().kind != Kind::End {
        return p.error("expected end of file");
    }
    Ok(s)
}
struct Parser {
    tokens: Vec<Token>,
    i: usize,
}
impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.i]
    }
    fn error<T>(&self, s: &str) -> Result<T, String> {
        Err(format!(
            "{}:{}: {s}, found {:?}",
            self.peek().line,
            self.peek().column,
            self.peek().text
        ))
    }
    fn take(&mut self) -> Token {
        let t = self.peek().clone();
        if t.kind != Kind::End {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek().text == s {
            self.take();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.eat(s) {
            Ok(())
        } else {
            self.error(&format!("expected {s}"))
        }
    }
    fn block(&mut self) -> Result<Stmt, String> {
        self.expect("{")?;
        let mut v = vec![];
        while !self.eat("}") {
            if self.peek().kind == Kind::End {
                return self.error("expected closing brace");
            }
            v.push(self.stmt()?);
        }
        Ok(Stmt::Block(v))
    }
    fn simple(&mut self) -> Result<Stmt, String> {
        if self.eat("int") {
            let t = self.take();
            if t.kind != Kind::Word
                || ["int", "if", "else", "while", "for", "return"].contains(&t.text.as_str())
            {
                return self.error("expected variable name");
            }
            let e = if self.eat("=") {
                Some(self.expr(0)?)
            } else {
                None
            };
            return Ok(Stmt::Decl(t, e));
        }
        if self.peek().kind == Kind::Word
            && self.tokens.get(self.i + 1).is_some_and(|t| t.text == "=")
        {
            let t = self.take();
            self.expect("=")?;
            return Ok(Stmt::Assign(t, self.expr(0)?));
        }
        let e = self.expr(0)?;
        if !matches!(e.kind, E::Call(..)) {
            return self.error("only calls and assignments are supported as statements");
        }
        Ok(Stmt::Call(e))
    }
    fn stmt(&mut self) -> Result<Stmt, String> {
        if self.peek().text == "{" {
            return self.block();
        }
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat("if") {
            self.expect("(")?;
            let e = self.expr(0)?;
            self.expect(")")?;
            let a = Box::new(self.stmt()?);
            let b = if self.eat("else") {
                Some(Box::new(self.stmt()?))
            } else {
                None
            };
            return Ok(Stmt::If(e, a, b));
        }
        if self.eat("while") {
            self.expect("(")?;
            let e = self.expr(0)?;
            self.expect(")")?;
            return Ok(Stmt::While(e, Box::new(self.stmt()?)));
        }
        if self.eat("for") {
            self.expect("(")?;
            let a = if self.peek().text == ";" {
                Stmt::Empty
            } else {
                self.simple()?
            };
            self.expect(";")?;
            let e = self.expr(0)?;
            self.expect(";")?;
            let b = if self.peek().text == ")" {
                Stmt::Empty
            } else {
                self.simple()?
            };
            self.expect(")")?;
            return Ok(Stmt::For(
                Box::new(a),
                e,
                Box::new(b),
                Box::new(self.stmt()?),
            ));
        }
        if self.peek().text == "return" {
            let t = self.take();
            let e = self.expr(0)?;
            self.expect(";")?;
            return Ok(Stmt::Return(t, e));
        }
        let s = self.simple()?;
        self.expect(";")?;
        Ok(s)
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let t = self.take();
        let k = match &t.kind {
            Kind::Number(n) => E::Int(*n),
            Kind::String(s) => E::Str(s.clone()),
            _ if t.text == "(" => {
                let e = self.expr(0)?;
                self.expect(")")?;
                return self.tail(e, min);
            }
            _ if ["-", "!"].contains(&t.text.as_str()) => {
                E::Unary(t.text.clone(), Box::new(self.expr(7)?))
            }
            Kind::Word => {
                if self.eat("(") {
                    let mut args = vec![];
                    if !self.eat(")") {
                        loop {
                            args.push(self.expr(0)?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    E::Call(t.text.clone(), args)
                } else {
                    E::Var(t.text.clone())
                }
            }
            _ => return self.error("expected expression"),
        };
        self.tail(Expr { token: t, kind: k }, min)
    }
    fn tail(&mut self, mut lhs: Expr, min: u8) -> Result<Expr, String> {
        loop {
            let op = self.peek().text.clone();
            let p = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" => 3,
                "<" | "<=" | ">" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                _ => break,
            };
            if p < min {
                break;
            }
            let t = self.take();
            let rhs = self.expr(p + 1)?;
            lhs = Expr {
                token: t,
                kind: E::Binary(op, Box::new(lhs), Box::new(rhs)),
            };
        }
        Ok(lhs)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    #[test]
    fn constructs() {
        for body in [
            "int x = 1 + 2 * 3;",
            "if(1){printf(\"x\");}else{}",
            "while(0){}",
            "for(int i=0;i<3;i=i+1){}",
            "{int x=0;}",
        ] {
            parse(lex(&format!("int main(){{{body} return 0;}}")).unwrap()).unwrap();
        }
    }
    #[test]
    fn precedence() {
        let s = parse(lex("int main(){int x=1+2*3;return 0;}").unwrap()).unwrap();
        if let Stmt::Block(v) = s
            && let Stmt::Decl(
                _,
                Some(Expr {
                    kind: E::Binary(op, _, rhs),
                    ..
                }),
            ) = &v[0]
        {
            assert_eq!(op, "+");
            assert!(matches!(&rhs.kind,E::Binary(op,_,_) if op=="*"));
            return;
        }
        panic!("wrong AST");
    }
    #[test]
    fn errors() {
        for s in [
            "int main(){",
            "int main(){int x=;}",
            "int main(){int x=1}",
            "int main(){} junk",
        ] {
            assert!(parse(lex(s).unwrap()).is_err());
        }
    }
}
