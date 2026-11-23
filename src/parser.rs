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
    Assign(String, Box<Expr>, Box<Expr>),
    Update(String, Box<Expr>, bool),
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
    Do(Box<Stmt>, Expr),
    Break(Token),
    Continue(Token),
    Switch(Expr, Box<Stmt>),
    Case(Token, Option<Expr>, Box<Stmt>),
    Label(Token, Box<Stmt>),
    Goto(Token),
}
pub(crate) fn children(s: &Stmt) -> Vec<&Stmt> {
    match s {
        Stmt::Block(v) => v.iter().collect(),
        Stmt::If(_, a, Some(b)) => vec![a, b],
        Stmt::If(_, a, None)
        | Stmt::While(_, a)
        | Stmt::Do(a, _)
        | Stmt::Switch(_, a)
        | Stmt::Label(_, a)
        | Stmt::Case(_, _, a) => vec![a],
        Stmt::For(a, _, update, body) => vec![a, body, update],
        _ => vec![],
    }
}
pub fn parse(tokens: Vec<Token>) -> Result<Stmt, String> {
    let mut p = Parser {
        tokens,
        i: 0,
        depth: 0,
    };
    p.expect("int")?;
    p.expect("main")?;
    p.expect("(")?;
    p.eat("void");
    p.expect(")")?;
    let s = p.block()?;
    if p.peek().kind != Kind::End {
        return p.error("expected end of file");
    }
    Ok(s)
}
fn reserved(s: &str) -> bool {
    matches!(
        s,
        "auto"
            | "break"
            | "case"
            | "char"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extern"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "register"
            | "restrict"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "typedef"
            | "union"
            | "unsigned"
            | "void"
            | "volatile"
            | "while"
            | "_Bool"
            | "_Complex"
            | "_Imaginary"
            | "_Alignas"
            | "_Alignof"
            | "_Atomic"
            | "_Generic"
            | "_Noreturn"
            | "_Static_assert"
            | "_Thread_local"
    )
}
struct Parser {
    tokens: Vec<Token>,
    i: usize,
    depth: usize,
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
                || reserved(&t.text)
                || ["main", "printf", "strlen"].contains(&t.text.as_str())
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
        let e = self.expr(0)?;
        Ok(Stmt::Call(e))
    }
    fn controlled(&mut self) -> Result<Stmt, String> {
        if self.peek().text == "int" {
            return self.error("a declaration requires a block here");
        }
        self.stmt()
    }
    fn stmt(&mut self) -> Result<Stmt, String> {
        if self.depth >= 64 {
            return self.error("syntax nesting limit exceeded");
        }
        self.depth += 1;
        let result = self.stmt_inner();
        self.depth -= 1;
        result
    }
    fn stmt_inner(&mut self) -> Result<Stmt, String> {
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
            let a = Box::new(self.controlled()?);
            let b = if self.eat("else") {
                Some(Box::new(self.controlled()?))
            } else {
                None
            };
            return Ok(Stmt::If(e, a, b));
        }
        if self.eat("while") {
            self.expect("(")?;
            let e = self.expr(0)?;
            self.expect(")")?;
            return Ok(Stmt::While(e, Box::new(self.controlled()?)));
        }
        if self.eat("for") {
            self.expect("(")?;
            let a = if self.peek().text == ";" {
                Stmt::Empty
            } else {
                self.simple()?
            };
            self.expect(";")?;
            let e = if self.peek().text == ";" {
                Expr {
                    token: self.peek().clone(),
                    kind: E::Int(1),
                }
            } else {
                self.expr(0)?
            };
            self.expect(";")?;
            let b = if self.peek().text == ")" {
                Stmt::Empty
            } else {
                self.simple()?
            };
            self.expect(")")?;
            if matches!(b, Stmt::Decl(..)) {
                return self.error("declaration is invalid in for update");
            }
            return Ok(Stmt::For(
                Box::new(a),
                e,
                Box::new(b),
                Box::new(self.controlled()?),
            ));
        }
        if self.peek().kind == Kind::Word
            && !reserved(&self.peek().text)
            && self.tokens.get(self.i + 1).is_some_and(|t| t.text == ":")
        {
            let name = self.take();
            self.expect(":")?;
            return Ok(Stmt::Label(name, Box::new(self.controlled()?)));
        }
        if self.peek().text == "goto" {
            self.take();
            let name = self.take();
            if name.kind != Kind::Word || reserved(&name.text) {
                return self.error("expected label name");
            }
            self.expect(";")?;
            return Ok(Stmt::Goto(name));
        }
        if self.peek().text == "break" || self.peek().text == "continue" {
            let token = self.take();
            self.expect(";")?;
            return Ok(if token.text == "break" {
                Stmt::Break(token)
            } else {
                Stmt::Continue(token)
            });
        }
        if self.eat("do") {
            let body = self.controlled()?;
            self.expect("while")?;
            self.expect("(")?;
            let e = self.expr(0)?;
            self.expect(")")?;
            self.expect(";")?;
            return Ok(Stmt::Do(Box::new(body), e));
        }
        if self.eat("switch") {
            self.expect("(")?;
            let e = self.expr(0)?;
            self.expect(")")?;
            return Ok(Stmt::Switch(e, Box::new(self.controlled()?)));
        }
        if self.peek().text == "case" || self.peek().text == "default" {
            let token = self.take();
            let e = if token.text == "case" {
                Some(self.expr(1)?)
            } else {
                None
            };
            self.expect(":")?;
            return Ok(Stmt::Case(token, e, Box::new(self.controlled()?)));
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
        if self.depth >= 64 {
            return self.error("syntax nesting limit exceeded");
        }
        self.depth += 1;
        let result = self.expr_inner(min);
        self.depth -= 1;
        result
    }
    fn expr_inner(&mut self, min: u8) -> Result<Expr, String> {
        let t = self.take();
        let k = match &t.kind {
            Kind::Number(n) => E::Int(*n),
            Kind::Integer(n) if n.suffix.is_empty() && n.magnitude <= i32::MAX as u64 => {
                E::Int(n.magnitude as i32)
            }
            Kind::Integer(_) => {
                return self.error("wide or suffixed integer requires the P11 type system");
            }
            Kind::String(s) => E::Str(s.clone()),
            _ if t.text == "(" => {
                let e = self.expr(0)?;
                self.expect(")")?;
                return self.tail(e, min);
            }
            _ if ["+", "-", "!"].contains(&t.text.as_str()) => {
                E::Unary(t.text.clone(), Box::new(self.expr(7)?))
            }
            _ if t.text == "++" || t.text == "--" => {
                E::Update(t.text.clone(), Box::new(self.expr(7)?), false)
            }
            Kind::Word if !reserved(&t.text) => {
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
        if self.peek().text == "++" || self.peek().text == "--" {
            let token = self.take();
            lhs = Expr {
                token: token.clone(),
                kind: E::Update(token.text, Box::new(lhs), true),
            };
        }
        let mut operators = 0;
        loop {
            let op = self.peek().text.clone();
            let assignment = ["=", "+=", "-=", "*=", "/=", "%="].contains(&op.as_str());
            let p = match op.as_str() {
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" => 0,
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
            operators += 1;
            if operators > 128 {
                return self.error("expression size limit exceeded");
            }
            let t = self.take();
            let rhs = self.expr(if assignment { p } else { p + 1 })?;
            lhs = Expr {
                token: t,
                kind: if assignment {
                    E::Assign(op, Box::new(lhs), Box::new(rhs))
                } else {
                    E::Binary(op, Box::new(lhs), Box::new(rhs))
                },
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
