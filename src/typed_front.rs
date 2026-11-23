//! Scoped declaration grammar and integer typed expressions for the C11 migration.
use crate::{
    integer::{self, Binary, Unary},
    lexer::{self, Kind, Token},
    typed_ir::SymbolId,
    types::{self, IntegerKind, IntegerValue, Qualifiers, Type, TypeContext, TypeId},
};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Typedef,
    Local,
    Parameter,
    Function,
}
#[derive(Debug, Clone)]
pub struct Declaration {
    pub token: Token,
    pub ty: TypeId,
    pub kind: DeclarationKind,
    pub scope: usize,
}
#[derive(Debug, Clone)]
pub struct Unit {
    pub types: TypeContext,
    pub declarations: Vec<Declaration>,
    pub main: Option<Statement>,
    pub main_symbol: Option<SymbolId>,
}
#[derive(Debug, Clone)]
pub struct Expr {
    pub token: Token,
    pub ty: TypeId,
    pub kind: Expression,
}
#[derive(Debug, Clone)]
pub enum Expression {
    Integer(IntegerValue),
    Variable(SymbolId),
    String(String),
    Convert(Box<Expr>),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    Logical(bool, Box<Expr>, Box<Expr>),
    Assign(SymbolId, Box<Expr>),
    Update {
        symbol: SymbolId,
        increment: bool,
        postfix: bool,
    },
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
}
#[derive(Debug, Clone)]
pub struct Statement {
    pub token: Token,
    pub kind: StatementKind,
}
#[derive(Debug, Clone)]
pub enum StatementKind {
    Block {
        scope: usize,
        items: Vec<Statement>,
    },
    Declarations(Vec<(SymbolId, Option<Expr>)>),
    Expression(Expr),
    Empty,
    If(Expr, Box<Statement>, Option<Box<Statement>>),
    While(Expr, Box<Statement>),
    Do(Box<Statement>, Expr),
    For {
        scope: usize,
        initial: Box<Statement>,
        condition: Option<Expr>,
        update: Option<Expr>,
        body: Box<Statement>,
    },
    Break,
    Continue,
    Return(Expr),
    Switch(Expr, Box<Statement>),
    Case(Option<IntegerValue>, Box<Statement>),
    Label(String, Box<Statement>),
    Goto(String),
}
impl Statement {
    pub fn children(&self) -> Vec<&Statement> {
        match &self.kind {
            StatementKind::Block { items, .. } => items.iter().collect(),
            StatementKind::If(_, a, Some(b)) => vec![a, b],
            StatementKind::If(_, a, None)
            | StatementKind::While(_, a)
            | StatementKind::Do(a, _)
            | StatementKind::Switch(_, a)
            | StatementKind::Case(_, a)
            | StatementKind::Label(_, a) => vec![a],
            StatementKind::For { initial, body, .. } => vec![initial, body],
            _ => vec![],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    None,
    Typedef,
    Auto,
    Register,
}
#[derive(Debug)]
enum Declarator {
    Name(Option<Token>),
    Pointer(Qualifiers, Box<Declarator>),
    Array(Box<Declarator>, Option<u64>),
    Function(Box<Declarator>, Vec<TypeId>, bool, bool),
}
impl Declarator {
    fn apply(
        self,
        types: &mut TypeContext,
        base: TypeId,
    ) -> Result<(Option<Token>, TypeId), String> {
        match self {
            Self::Name(name) => Ok((name, base)),
            Self::Pointer(q, child) => {
                let p = types.pointer(base)?;
                let p = types.qualify(p, q)?;
                child.apply(types, p)
            }
            Self::Array(child, n) => {
                let a = types.array(base, n)?;
                child.apply(types, a)
            }
            Self::Function(child, params, variadic, prototype) => {
                let f = types.function(base, params, variadic, prototype)?;
                child.apply(types, f)
            }
        }
    }
}
struct Scope {
    id: usize,
    names: HashMap<String, SymbolId>,
}
struct Switch {
    ty: TypeId,
    cases: HashSet<u64>,
    default: bool,
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    types: TypeContext,
    declarations: Vec<Declaration>,
    scopes: Vec<Scope>,
    next_scope: usize,
    expression_level: usize,
    expression_operators: usize,
    declarator_level: usize,
    declarator_operations: usize,
    loops: usize,
    breaks: usize,
    switches: Vec<Switch>,
    labels: HashSet<String>,
    gotos: Vec<Token>,
}
pub fn parse(source: &str) -> Result<Unit, String> {
    parse_tokens(lexer::lex_typed(source)?)
}
pub fn parse_tokens(tokens: Vec<Token>) -> Result<Unit, String> {
    if tokens.last().is_none_or(|t| t.kind != Kind::End)
        || tokens[..tokens.len().saturating_sub(1)]
            .iter()
            .any(|t| t.kind == Kind::End)
    {
        return Err("typed parser requires a single terminal EOF token".into());
    }
    if tokens.len() > lexer::MAX_TOKENS + 1 {
        return Err("typed token budget exceeded".into());
    }
    Parser {
        tokens,
        at: 0,
        depth: 0,
        types: TypeContext::new(),
        declarations: vec![],
        scopes: vec![Scope {
            id: 0,
            names: HashMap::new(),
        }],
        next_scope: 1,
        expression_level: 0,
        expression_operators: 0,
        declarator_level: 0,
        declarator_operations: 0,
        loops: 0,
        breaks: 0,
        switches: vec![],
        labels: HashSet::new(),
        gotos: vec![],
    }
    .unit()
}
fn error(token: &Token, message: impl std::fmt::Display) -> String {
    format!("{}:{}: {message}", token.line, token.column)
}
fn keyword(name: &str) -> bool {
    crate::parser::reserved(name)
        || matches!(
            name,
            "_Alignof" | "_Atomic" | "_Generic" | "_Static_assert" | "_Thread_local"
        )
}
impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn take(&mut self) -> Token {
        let t = self.peek().clone();
        if t.kind != Kind::End {
            self.at += 1;
        }
        t
    }
    fn eat(&mut self, text: &str) -> bool {
        if self.peek().text == text {
            self.take();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, text: &str) -> Result<(), String> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(error(self.peek(), format!("expected {text}")))
        }
    }
    fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).copied())
    }
    fn typedef(&self, name: &str) -> Option<TypeId> {
        self.lookup(name).and_then(|id| {
            let d = &self.declarations[id.0];
            (d.kind == DeclarationKind::Typedef).then_some(d.ty)
        })
    }
    fn declaration_start(&self) -> bool {
        matches!(
            self.peek().text.as_str(),
            "void"
                | "_Bool"
                | "char"
                | "short"
                | "int"
                | "long"
                | "signed"
                | "unsigned"
                | "const"
                | "volatile"
                | "restrict"
                | "typedef"
                | "auto"
                | "register"
                | "extern"
                | "static"
        ) || self.typedef(&self.peek().text).is_some()
    }
    fn push_scope(&mut self) -> usize {
        let id = self.next_scope;
        self.next_scope += 1;
        self.scopes.push(Scope {
            id,
            names: HashMap::new(),
        });
        id
    }
    fn define(
        &mut self,
        token: Token,
        ty: TypeId,
        kind: DeclarationKind,
    ) -> Result<SymbolId, String> {
        let scope = self.scopes.last().unwrap();
        if let Some(id) = scope.names.get(&token.text).copied() {
            let old = &self.declarations[id.0];
            if old.kind == kind
                && matches!(kind, DeclarationKind::Typedef | DeclarationKind::Function)
                && self.types.compatible(old.ty, ty)?
            {
                if kind == DeclarationKind::Function
                    && matches!(
                        self.types.get(ty)?,
                        Type::Function {
                            prototype: true,
                            ..
                        }
                    )
                {
                    self.declarations[id.0].ty = ty;
                }
                return Ok(id);
            }
            return Err(error(&token, "conflicting declaration in this scope"));
        }
        if self.declarations.len() >= 65535 {
            return Err(error(&token, "declaration budget exceeded"));
        }
        let id = SymbolId(self.declarations.len());
        let scope = self.scopes.last_mut().unwrap();
        scope.names.insert(token.text.clone(), id);
        self.declarations.push(Declaration {
            token,
            ty,
            kind,
            scope: scope.id,
        });
        Ok(id)
    }
    fn qualifiers(&mut self) -> Qualifiers {
        let mut q = Qualifiers::default();
        loop {
            match self.peek().text.as_str() {
                "const" => q.constant = true,
                "volatile" => q.volatile = true,
                "restrict" => q.restrict = true,
                _ => break,
            }
            self.take();
        }
        q
    }
    fn specifiers(&mut self) -> Result<(TypeId, Storage), String> {
        let token = self.peek().clone();
        let mut alias = None;
        let (
            mut signed,
            mut unsigned,
            mut short,
            mut long,
            mut int,
            mut char_,
            mut bool_,
            mut void,
        ) = (0, 0, 0, 0, 0, 0, 0, 0);
        let mut q = Qualifiers::default();
        let mut storage = 0;
        let mut storage_kind = Storage::None;
        loop {
            match self.peek().text.as_str() {
                "const" => q.constant = true,
                "volatile" => q.volatile = true,
                "restrict" => q.restrict = true,
                "typedef" => {
                    storage += 1;
                    storage_kind = Storage::Typedef;
                }
                "auto" | "register" => {
                    storage += 1;
                    storage_kind = if self.peek().text == "auto" {
                        Storage::Auto
                    } else {
                        Storage::Register
                    };
                    if self.scopes.len() == 1 {
                        return Err(error(
                            self.peek(),
                            "automatic storage specifier at file scope",
                        ));
                    }
                }
                "extern" | "static" => {
                    return Err(error(
                        self.peek(),
                        "static/extern storage is not yet supported in typed mode",
                    ));
                }
                "signed" => signed += 1,
                "unsigned" => unsigned += 1,
                "short" => short += 1,
                "long" => long += 1,
                "int" => int += 1,
                "char" => char_ += 1,
                "_Bool" => bool_ += 1,
                "void" => void += 1,
                name if alias.is_none()
                    && signed + unsigned + short + long + int + char_ + bool_ + void == 0
                    && self.typedef(name).is_some() =>
                {
                    alias = self.typedef(name);
                }
                _ => break,
            }
            self.take();
        }
        if storage > 1
            || signed > 1
            || unsigned > 1
            || signed + unsigned > 1
            || short > 1
            || long > 2
            || int > 1
            || char_ > 1
            || bool_ > 1
            || void > 1
        {
            return Err(error(&token, "invalid declaration specifier combination"));
        }
        let ty = if let Some(alias) = alias {
            if signed + unsigned + short + long + int + char_ + bool_ + void != 0 {
                return Err(error(
                    &token,
                    "typedef name cannot be combined with another base type",
                ));
            }
            alias
        } else if void != 0 {
            if signed + unsigned + short + long + int + char_ + bool_ != 0 {
                return Err(error(&token, "invalid void type specifiers"));
            }
            self.types.void()
        } else {
            let kind = if bool_ != 0 {
                if signed + unsigned + short + long + int + char_ != 0 {
                    return Err(error(&token, "invalid Bool type specifiers"));
                }
                IntegerKind::Bool
            } else if char_ != 0 {
                if short + long + int != 0 {
                    return Err(error(&token, "invalid character type specifiers"));
                }
                if unsigned != 0 {
                    IntegerKind::UnsignedChar
                } else if signed != 0 {
                    IntegerKind::SignedChar
                } else {
                    IntegerKind::Char
                }
            } else if short != 0 {
                if long != 0 {
                    return Err(error(&token, "short and long cannot be combined"));
                }
                if unsigned != 0 {
                    IntegerKind::UnsignedShort
                } else {
                    IntegerKind::Short
                }
            } else if long == 2 {
                if unsigned != 0 {
                    IntegerKind::UnsignedLongLong
                } else {
                    IntegerKind::LongLong
                }
            } else if long == 1 {
                if unsigned != 0 {
                    IntegerKind::UnsignedLong
                } else {
                    IntegerKind::Long
                }
            } else if signed + unsigned + int != 0 {
                if unsigned != 0 {
                    IntegerKind::UnsignedInt
                } else {
                    IntegerKind::Int
                }
            } else {
                return Err(error(&token, "explicit type specifier required"));
            };
            self.types.integer(kind)
        };
        self.types
            .qualify(ty, q)
            .map(|ty| (ty, storage_kind))
            .map_err(|e| error(&token, e))
    }
    fn declarator(&mut self, allow_name: bool, abstract_: bool) -> Result<Declarator, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "declarator nesting limit exceeded"));
        }
        if self.declarator_level == 0 {
            self.declarator_operations = 0;
        }
        self.declarator_level += 1;
        self.depth += 1;
        let result = self.declarator_inner(allow_name, abstract_);
        self.depth -= 1;
        self.declarator_level -= 1;
        result
    }
    fn declarator_inner(
        &mut self,
        allow_name: bool,
        abstract_: bool,
    ) -> Result<Declarator, String> {
        let mut stars = vec![];
        while self.eat("*") {
            self.declarator_operations += 1;
            if self.declarator_operations > 128 {
                return Err(error(
                    self.peek(),
                    "aggregate declarator complexity limit exceeded",
                ));
            }
            if stars.len() >= 64 {
                return Err(error(self.peek(), "pointer nesting limit exceeded"));
            }
            stars.push(self.qualifiers());
        }
        let group = self.peek().text == "("
            && !(abstract_
                && self.tokens.get(self.at + 1).is_some_and(|t| {
                    t.text == ")"
                        || matches!(
                            t.text.as_str(),
                            "void"
                                | "int"
                                | "char"
                                | "short"
                                | "long"
                                | "signed"
                                | "unsigned"
                                | "_Bool"
                                | "const"
                                | "volatile"
                                | "restrict"
                        )
                        || self.typedef(&t.text).is_some()
                }));
        let mut direct = if group {
            self.take();
            let inner = self.declarator(allow_name, abstract_)?;
            self.expect(")")?;
            inner
        } else if allow_name && self.peek().kind == Kind::Word && !keyword(&self.peek().text) {
            Declarator::Name(Some(self.take()))
        } else if abstract_ {
            Declarator::Name(None)
        } else {
            return Err(error(self.peek(), "declarator requires an identifier"));
        };
        let mut suffixes = 0;
        loop {
            suffixes += 1;
            if suffixes > 64 {
                return Err(error(
                    self.peek(),
                    "declarator suffix nesting limit exceeded",
                ));
            }
            if self.peek().text == "[" || self.peek().text == "(" {
                self.declarator_operations += 1;
                if self.declarator_operations > 128 {
                    return Err(error(
                        self.peek(),
                        "aggregate declarator complexity limit exceeded",
                    ));
                }
            }
            if self.eat("[") {
                let length = if self.eat("]") {
                    None
                } else {
                    let e = self.expr(1)?;
                    let value = constant(&self.types, &e)?;
                    if value.signed_value(&self.types)? <= 0 {
                        return Err(error(&e.token, "array bound must be positive"));
                    }
                    self.expect("]")?;
                    Some(value.bits)
                };
                direct = Declarator::Array(Box::new(direct), length);
            } else if self.eat("(") {
                let mut parameters = vec![];
                let mut variadic = false;
                let prototype = !self.eat(")");
                self.push_scope();
                if prototype {
                    if self.peek().text == "void"
                        && self.tokens.get(self.at + 1).is_some_and(|t| t.text == ")")
                    {
                        self.take();
                        self.take();
                    } else {
                        loop {
                            if self.eat("...") {
                                variadic = true;
                                self.expect(")")?;
                                break;
                            }
                            let token = self.peek().clone();
                            let (base, storage) = self.specifiers()?;
                            if !matches!(storage, Storage::None | Storage::Register) {
                                return Err(error(&token, "invalid parameter storage class"));
                            }
                            let d = self.declarator(true, true)?;
                            let (name, ty) = d
                                .apply(&mut self.types, base)
                                .map_err(|e| error(&token, e))?;
                            if let Some(name) = name {
                                self.define(name, ty, DeclarationKind::Parameter)?;
                            }
                            parameters.push(ty);
                            if parameters.len() > 1024 {
                                return Err(error(&token, "parameter budget exceeded"));
                            }
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                }
                self.scopes.pop();
                direct = Declarator::Function(Box::new(direct), parameters, variadic, prototype);
            } else {
                break;
            }
        }
        for q in stars.into_iter().rev() {
            direct = Declarator::Pointer(q, Box::new(direct));
        }
        Ok(direct)
    }
    fn type_name(&mut self) -> Result<TypeId, String> {
        let token = self.peek().clone();
        let (base, storage) = self.specifiers()?;
        if storage != Storage::None {
            return Err(error(&token, "storage class in type name"));
        }
        let declarator = self.declarator(false, true)?;
        let (name, ty) = declarator
            .apply(&mut self.types, base)
            .map_err(|e| error(&token, e))?;
        if name.is_some() {
            return Err(error(&token, "identifier in abstract type name"));
        }
        Ok(ty)
    }
    fn unit(mut self) -> Result<Unit, String> {
        let mut main = None;
        let mut main_symbol = None;
        while self.peek().kind != Kind::End {
            let token = self.peek().clone();
            let (base, storage) = self.specifiers()?;
            let typedef = storage == Storage::Typedef;
            loop {
                let declarator = self.declarator(true, false)?;
                let (name, mut ty) = declarator
                    .apply(&mut self.types, base)
                    .map_err(|e| error(&token, e))?;
                let name = name.ok_or_else(|| error(&token, "declaration requires a name"))?;
                let function = matches!(self.types.get(ty)?, Type::Function { .. });
                let kind = if typedef {
                    DeclarationKind::Typedef
                } else if function {
                    DeclarationKind::Function
                } else {
                    return Err(error(
                        &name,
                        "global object storage is not yet supported in typed mode",
                    ));
                };
                if self.peek().text == "{" {
                    if typedef || !function || name.text != "main" || main.is_some() {
                        return Err(error(
                            &name,
                            "typed phase supports one main definition; general functions follow in the call phase",
                        ));
                    }
                    let Type::Function {
                        result,
                        parameters,
                        variadic,
                        ..
                    } = self.types.get(ty)?.clone()
                    else {
                        unreachable!()
                    };
                    if result != self.types.integer(IntegerKind::Int)
                        || !parameters.is_empty()
                        || variadic
                    {
                        return Err(error(&name, "typed main requires int main(void)"));
                    }
                    ty = self.types.function(result, vec![], false, true)?;
                    let id = self.define(name, ty, kind)?;
                    main = Some(self.block()?);
                    main_symbol = Some(id);
                    for target in &self.gotos {
                        if !self.labels.contains(&target.text) {
                            return Err(error(target, "undefined label"));
                        }
                    }
                    break;
                }
                self.define(name, ty, kind)?;
                if !self.eat(",") {
                    self.expect(";")?;
                    break;
                }
            }
        }
        Ok(Unit {
            types: self.types,
            declarations: self.declarations,
            main,
            main_symbol,
        })
    }
    fn declaration(&mut self) -> Result<Statement, String> {
        let token = self.peek().clone();
        let (base, storage) = self.specifiers()?;
        let typedef = storage == Storage::Typedef;
        let mut declarations = vec![];
        loop {
            let d = self.declarator(true, false)?;
            let (name, ty) = d
                .apply(&mut self.types, base)
                .map_err(|e| error(&token, e))?;
            let name = name.ok_or_else(|| error(&token, "missing declaration name"))?;
            let kind = if typedef {
                DeclarationKind::Typedef
            } else if matches!(self.types.get(ty)?, Type::Function { .. }) {
                DeclarationKind::Function
            } else {
                DeclarationKind::Local
            };
            if kind == DeclarationKind::Function
                && matches!(storage, Storage::Auto | Storage::Register)
            {
                return Err(error(&name, "invalid function storage class"));
            }
            if kind == DeclarationKind::Local {
                self.types.layout(ty).map_err(|e| error(&name, e))?;
            }
            let id = self.define(name, ty, kind)?;
            let initial = if self.eat("=") {
                if kind != DeclarationKind::Local {
                    return Err(error(
                        self.peek(),
                        "only local objects can have initializers here",
                    ));
                }
                let e = self.expr(1)?;
                Some(self.convert(e, self.types.unqualified(ty)?)?)
            } else {
                None
            };
            if kind == DeclarationKind::Local {
                declarations.push((id, initial));
            }
            if !self.eat(",") {
                self.expect(";")?;
                break;
            }
        }
        Ok(Statement {
            token,
            kind: StatementKind::Declarations(declarations),
        })
    }
    fn block(&mut self) -> Result<Statement, String> {
        let token = self.peek().clone();
        self.expect("{")?;
        let scope = self.push_scope();
        let mut items = vec![];
        while !self.eat("}") {
            if self.peek().kind == Kind::End {
                return Err(error(self.peek(), "unterminated block"));
            }
            items.push(self.statement()?);
        }
        self.scopes.pop();
        Ok(Statement {
            token,
            kind: StatementKind::Block { scope, items },
        })
    }
    fn controlled(&mut self) -> Result<Statement, String> {
        if self.declaration_start() {
            return Err(error(
                self.peek(),
                "declaration requires a compound block here",
            ));
        }
        self.statement()
    }
    fn statement(&mut self) -> Result<Statement, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "statement nesting limit exceeded"));
        }
        self.depth += 1;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn condition(&mut self) -> Result<Expr, String> {
        let e = self.expr(0)?;
        self.types
            .integer_kind(e.ty)
            .map_err(|e2| error(&e.token, e2))?;
        Ok(e)
    }
    fn statement_inner(&mut self) -> Result<Statement, String> {
        let token = self.peek().clone();
        let kind = if self.peek().text == "{" {
            return self.block();
        } else if self.peek().kind == Kind::Word
            && !keyword(&self.peek().text)
            && self.tokens.get(self.at + 1).is_some_and(|t| t.text == ":")
        {
            let name = self.take();
            self.take();
            if !self.labels.insert(name.text.clone()) {
                return Err(error(&name, "duplicate label"));
            }
            StatementKind::Label(name.text, Box::new(self.controlled()?))
        } else if self.declaration_start() {
            return self.declaration();
        } else if self.eat(";") {
            StatementKind::Empty
        } else if self.eat("if") {
            self.expect("(")?;
            let e = self.condition()?;
            self.expect(")")?;
            let a = Box::new(self.controlled()?);
            let b = if self.eat("else") {
                Some(Box::new(self.controlled()?))
            } else {
                None
            };
            StatementKind::If(e, a, b)
        } else if self.eat("while") {
            self.expect("(")?;
            let e = self.condition()?;
            self.expect(")")?;
            self.loops += 1;
            self.breaks += 1;
            let body = self.controlled()?;
            self.loops -= 1;
            self.breaks -= 1;
            StatementKind::While(e, Box::new(body))
        } else if self.eat("do") {
            self.loops += 1;
            self.breaks += 1;
            let body = self.controlled()?;
            self.loops -= 1;
            self.breaks -= 1;
            self.expect("while")?;
            self.expect("(")?;
            let e = self.condition()?;
            self.expect(")")?;
            self.expect(";")?;
            StatementKind::Do(Box::new(body), e)
        } else if self.eat("for") {
            self.expect("(")?;
            let scope = self.push_scope();
            let initial = if self.declaration_start() {
                self.declaration()?
            } else {
                let e = if self.peek().text == ";" {
                    None
                } else {
                    Some(self.expr(0)?)
                };
                self.expect(";")?;
                Statement {
                    token: token.clone(),
                    kind: e.map_or(StatementKind::Empty, StatementKind::Expression),
                }
            };
            let condition = if self.peek().text == ";" {
                None
            } else {
                Some(self.condition()?)
            };
            self.expect(";")?;
            let update = if self.peek().text == ")" {
                None
            } else {
                Some(self.expr(0)?)
            };
            self.expect(")")?;
            self.loops += 1;
            self.breaks += 1;
            let body = Box::new(self.controlled()?);
            self.loops -= 1;
            self.breaks -= 1;
            self.scopes.pop();
            StatementKind::For {
                scope,
                initial: Box::new(initial),
                condition,
                update,
                body,
            }
        } else if self.eat("return") {
            let e = self.expr(0)?;
            let e = self.convert(e, self.types.integer(IntegerKind::Int))?;
            self.expect(";")?;
            StatementKind::Return(e)
        } else if self.eat("break") {
            if self.breaks == 0 {
                return Err(error(&token, "break outside loop/switch"));
            }
            self.expect(";")?;
            StatementKind::Break
        } else if self.eat("continue") {
            if self.loops == 0 {
                return Err(error(&token, "continue outside loop"));
            }
            self.expect(";")?;
            StatementKind::Continue
        } else if self.eat("switch") {
            self.expect("(")?;
            let e = self.expr(0)?;
            let ty = self
                .types
                .promoted(e.ty)
                .map_err(|e2| error(&e.token, e2))?;
            let e = self.convert(e, ty)?;
            self.expect(")")?;
            self.switches.push(Switch {
                ty,
                cases: HashSet::new(),
                default: false,
            });
            self.breaks += 1;
            let body = Box::new(self.controlled()?);
            self.switches.pop();
            self.breaks -= 1;
            StatementKind::Switch(e, body)
        } else if self.peek().text == "case" || self.peek().text == "default" {
            let case = self.take();
            let e = if case.text == "case" {
                Some(self.expr(1)?)
            } else {
                None
            };
            self.expect(":")?;
            let value = e.as_ref().map(|e| constant(&self.types, e)).transpose()?;
            let switch = self
                .switches
                .last_mut()
                .ok_or_else(|| error(&case, "case/default outside switch"))?;
            let value = if let Some(value) = value {
                let value = value.cast(&self.types, switch.ty)?;
                if !switch.cases.insert(value.bits) {
                    return Err(error(&case, "duplicate converted case value"));
                }
                Some(value)
            } else {
                if std::mem::replace(&mut switch.default, true) {
                    return Err(error(&case, "duplicate default label"));
                }
                None
            };
            StatementKind::Case(value, Box::new(self.controlled()?))
        } else if self.eat("goto") {
            let name = self.take();
            if name.kind != Kind::Word || keyword(&name.text) {
                return Err(error(&name, "expected label name"));
            }
            self.expect(";")?;
            self.gotos.push(name.clone());
            StatementKind::Goto(name.text)
        } else {
            let e = self.expr(0)?;
            self.expect(";")?;
            StatementKind::Expression(e)
        };
        Ok(Statement { token, kind })
    }
    fn convert(&self, e: Expr, ty: TypeId) -> Result<Expr, String> {
        self.types
            .integer_kind(e.ty)
            .map_err(|e2| error(&e.token, e2))?;
        self.types
            .integer_kind(ty)
            .map_err(|e2| error(&e.token, e2))?;
        let ty = self.types.unqualified(ty)?;
        if e.ty == ty {
            Ok(e)
        } else {
            Ok(Expr {
                token: e.token.clone(),
                ty,
                kind: Expression::Convert(Box::new(e)),
            })
        }
    }
    fn lvalue(&self, e: &Expr) -> Result<SymbolId, String> {
        let Expression::Variable(id) = e.kind else {
            return Err(error(&e.token, "modifiable integer variable required"));
        };
        let d = &self.declarations[id.0];
        if d.kind != DeclarationKind::Local || self.types.qualifiers(d.ty)?.constant {
            return Err(error(&e.token, "modifiable integer variable required"));
        }
        self.types
            .integer_kind(d.ty)
            .map_err(|e2| error(&e.token, e2))?;
        Ok(id)
    }
    fn binary(&self, token: Token, op: Binary, a: Expr, b: Expr) -> Result<Expr, String> {
        let (a, b) = if op.shift() {
            let at = self.types.promoted(a.ty)?;
            let bt = self.types.promoted(b.ty)?;
            (self.convert(a, at)?, self.convert(b, bt)?)
        } else {
            let ty = self
                .types
                .common_integer(a.ty, b.ty)
                .map_err(|e| error(&token, e))?;
            (self.convert(a, ty)?, self.convert(b, ty)?)
        };
        let ty = if op.comparison() {
            self.types.integer(IntegerKind::Int)
        } else {
            a.ty
        };
        Ok(Expr {
            token,
            ty,
            kind: Expression::Binary(op, Box::new(a), Box::new(b)),
        })
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "expression nesting limit exceeded"));
        }
        if self.expression_level == 0 {
            self.expression_operators = 0;
        }
        self.expression_level += 1;
        self.depth += 1;
        let result = self.expr_inner(min);
        self.depth -= 1;
        self.expression_level -= 1;
        result
    }
    fn expr_inner(&mut self, min: u8) -> Result<Expr, String> {
        let token = self.take();
        let mut lhs = match &token.kind {
            Kind::Number(n) => {
                let value = IntegerValue::new(
                    &self.types,
                    self.types.integer(IntegerKind::Int),
                    *n as u64,
                )?;
                Expr {
                    token: token.clone(),
                    ty: value.ty,
                    kind: Expression::Integer(value),
                }
            }
            Kind::Integer(n) => {
                let value = types::integer_literal(&self.types, n.magnitude, n.radix, &n.suffix)
                    .map_err(|e| error(&token, e))?;
                Expr {
                    token: token.clone(),
                    ty: value.ty,
                    kind: Expression::Integer(value),
                }
            }
            Kind::String(s) => {
                let char_ = self.types.integer(IntegerKind::Char);
                let ty = self.types.array(char_, Some(s.len() as u64 + 1))?;
                Expr {
                    token: token.clone(),
                    ty,
                    kind: Expression::String(s.clone()),
                }
            }
            _ if token.text == "sizeof" || token.text == "_Alignof" => {
                let align = token.text == "_Alignof";
                let ty = if self.peek().text == "(" {
                    let saved = self.at;
                    self.take();
                    if self.declaration_start() {
                        let ty = self.type_name()?;
                        self.expect(")")?;
                        ty
                    } else {
                        if align {
                            return Err(error(
                                &token,
                                "Alignof requires a parenthesized type name",
                            ));
                        }
                        self.at = saved;
                        self.expr(13)?.ty
                    }
                } else {
                    if align {
                        return Err(error(&token, "Alignof requires a type name"));
                    }
                    self.expr(13)?.ty
                };
                let layout = self.types.layout(ty).map_err(|e| error(&token, e))?;
                let value = IntegerValue::new(
                    &self.types,
                    self.types.integer(IntegerKind::UnsignedLongLong),
                    if align {
                        u64::from(layout.align)
                    } else {
                        layout.size
                    },
                )?;
                Expr {
                    token: token.clone(),
                    ty: value.ty,
                    kind: Expression::Integer(value),
                }
            }
            _ if token.text == "(" => {
                if self.declaration_start() {
                    let ty = self.type_name()?;
                    self.expect(")")?;
                    let rhs = self.expr(13)?;
                    let converted = self.convert(rhs, ty)?;
                    Expr {
                        token: token.clone(),
                        ty: self.types.unqualified(ty)?,
                        kind: Expression::Convert(Box::new(converted)),
                    }
                } else {
                    let inner = self.expr(0)?;
                    self.expect(")")?;
                    inner
                }
            }
            _ if ["+", "-", "!", "~"].contains(&token.text.as_str()) => {
                let e = self.expr(13)?;
                let op = match token.text.as_str() {
                    "+" => Unary::Plus,
                    "-" => Unary::Negate,
                    "!" => Unary::Not,
                    _ => Unary::Complement,
                };
                let promoted = self.types.promoted(e.ty).map_err(|e| error(&token, e))?;
                let e = self.convert(e, promoted)?;
                let ty = if op == Unary::Not {
                    self.types.integer(IntegerKind::Int)
                } else {
                    promoted
                };
                Expr {
                    token: token.clone(),
                    ty,
                    kind: Expression::Unary(op, Box::new(e)),
                }
            }
            _ if token.text == "++" || token.text == "--" => {
                let e = self.expr(13)?;
                let symbol = self.lvalue(&e)?;
                let ty = self.types.unqualified(self.declarations[symbol.0].ty)?;
                Expr {
                    token: token.clone(),
                    ty,
                    kind: Expression::Update {
                        symbol,
                        increment: token.text == "++",
                        postfix: false,
                    },
                }
            }
            Kind::Word if !keyword(&token.text) => {
                let id = self
                    .lookup(&token.text)
                    .ok_or_else(|| error(&token, "undeclared identifier"))?;
                let d = &self.declarations[id.0];
                if d.kind == DeclarationKind::Typedef {
                    return Err(error(&token, "type name used as an expression"));
                }
                Expr {
                    token: token.clone(),
                    ty: self.types.unqualified(d.ty)?,
                    kind: Expression::Variable(id),
                }
            }
            _ => return Err(error(&token, "expected expression")),
        };
        if self.peek().text == "++" || self.peek().text == "--" {
            let op = self.take();
            let symbol = self.lvalue(&lhs)?;
            let ty = self.types.unqualified(self.declarations[symbol.0].ty)?;
            lhs = Expr {
                token: op.clone(),
                ty,
                kind: Expression::Update {
                    symbol,
                    increment: op.text == "++",
                    postfix: true,
                },
            };
        }
        if self.peek().text == "(" {
            return Err(error(
                self.peek(),
                "function calls are not yet supported in typed mode",
            ));
        }
        let mut tail = 0;
        loop {
            let op = self.peek().text.clone();
            let assignment = [
                "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
            ]
            .contains(&op.as_str());
            let precedence = match op.as_str() {
                "," => 0,
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" => 1,
                "?" => 2,
                "||" => 3,
                "&&" => 4,
                "|" => 5,
                "^" => 6,
                "&" => 7,
                "==" | "!=" => 8,
                "<" | "<=" | ">" | ">=" => 9,
                "<<" | ">>" => 10,
                "+" | "-" => 11,
                "*" | "/" | "%" => 12,
                _ => break,
            };
            if precedence < min {
                break;
            }
            tail += 1;
            self.expression_operators += 1;
            if tail > 128 || self.expression_operators > 128 {
                return Err(error(self.peek(), "expression size limit exceeded"));
            }
            let t = self.take();
            if op == "?" {
                self.types.integer_kind(lhs.ty).map_err(|e| error(&t, e))?;
                let a = self.expr(0)?;
                self.expect(":")?;
                let b = self.expr(2)?;
                let ty = self
                    .types
                    .common_integer(a.ty, b.ty)
                    .map_err(|e| error(&t, e))?;
                lhs = Expr {
                    token: t,
                    ty,
                    kind: Expression::Conditional(
                        Box::new(lhs),
                        Box::new(self.convert(a, ty)?),
                        Box::new(self.convert(b, ty)?),
                    ),
                };
                continue;
            }
            let rhs = self.expr(if assignment {
                precedence
            } else {
                precedence + 1
            })?;
            if assignment {
                let symbol = self.lvalue(&lhs)?;
                let target = self.types.unqualified(self.declarations[symbol.0].ty)?;
                let rhs = if op == "=" {
                    rhs
                } else {
                    self.binary(
                        t.clone(),
                        binary_operator(&op[..op.len() - 1])
                            .ok_or_else(|| error(&t, "unsupported compound assignment"))?,
                        lhs,
                        rhs,
                    )?
                };
                lhs = Expr {
                    token: t,
                    ty: target,
                    kind: Expression::Assign(symbol, Box::new(self.convert(rhs, target)?)),
                };
            } else if op == "," {
                self.types.integer_kind(rhs.ty).map_err(|_| {
                    error(
                        &t,
                        "comma result with array/function/pointer type is not yet supported",
                    )
                })?;
                let ty = rhs.ty;
                lhs = Expr {
                    token: t,
                    ty,
                    kind: Expression::Comma(Box::new(lhs), Box::new(rhs)),
                };
            } else if op == "&&" || op == "||" {
                self.types.integer_kind(lhs.ty).map_err(|e| error(&t, e))?;
                self.types.integer_kind(rhs.ty).map_err(|e| error(&t, e))?;
                lhs = Expr {
                    token: t,
                    ty: self.types.integer(IntegerKind::Int),
                    kind: Expression::Logical(op == "&&", Box::new(lhs), Box::new(rhs)),
                };
            } else {
                lhs = self.binary(
                    t,
                    binary_operator(&op).ok_or("unknown typed binary operator")?,
                    lhs,
                    rhs,
                )?;
            }
        }
        Ok(lhs)
    }
}
fn binary_operator(op: &str) -> Option<Binary> {
    Some(match op {
        "+" => Binary::Add,
        "-" => Binary::Subtract,
        "*" => Binary::Multiply,
        "/" => Binary::Divide,
        "%" => Binary::Remainder,
        "&" => Binary::And,
        "|" => Binary::Or,
        "^" => Binary::Xor,
        "<<" => Binary::ShiftLeft,
        ">>" => Binary::ShiftRight,
        "==" => Binary::Equal,
        "!=" => Binary::NotEqual,
        "<" => Binary::Less,
        "<=" => Binary::LessEqual,
        ">" => Binary::Greater,
        ">=" => Binary::GreaterEqual,
        _ => return None,
    })
}
pub fn constant(types: &TypeContext, e: &Expr) -> Result<IntegerValue, String> {
    let value = match &e.kind {
        Expression::Integer(value) => Ok(*value),
        Expression::Convert(a) => constant(types, a)?.cast(types, e.ty),
        Expression::Unary(op, a) => integer::unary(types, *op, constant(types, a)?),
        Expression::Binary(op, a, b) => {
            integer::binary(types, *op, constant(types, a)?, constant(types, b)?)
        }
        Expression::Logical(and, a, b) => {
            constant_shape(a)?;
            constant_shape(b)?;
            let a = constant(types, a)?.bits != 0;
            let result = if *and {
                a && constant(types, b)?.bits != 0
            } else {
                a || constant(types, b)?.bits != 0
            };
            IntegerValue::new(types, e.ty, u64::from(result))
        }
        Expression::Conditional(condition, a, b) => {
            constant_shape(condition)?;
            constant_shape(a)?;
            constant_shape(b)?;
            constant(
                types,
                if constant(types, condition)?.bits != 0 {
                    a
                } else {
                    b
                },
            )
        }
        _ => Err("integer constant expression required".into()),
    };
    value.map_err(|e2| error(&e.token, e2))
}
fn constant_shape(e: &Expr) -> Result<(), String> {
    match &e.kind {
        Expression::Integer(_) => Ok(()),
        Expression::Convert(a) | Expression::Unary(_, a) => constant_shape(a),
        Expression::Binary(_, a, b) | Expression::Logical(_, a, b) => {
            constant_shape(a)?;
            constant_shape(b)
        }
        Expression::Conditional(c, a, b) => {
            constant_shape(c)?;
            constant_shape(a)?;
            constant_shape(b)
        }
        _ => Err(error(&e.token, "integer constant expression required")),
    }
}
