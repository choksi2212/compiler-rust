//! Scoped declaration grammar and integer typed expressions for the C11 migration.
use crate::{
    integer::{self, Binary, Unary},
    lexer::{self, Kind, Token},
    typed_ir::SymbolId,
    types::{self, IntegerKind, IntegerValue, Linkage, Qualifiers, Type, TypeContext, TypeId},
};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Typedef,
    Local,
    Parameter,
    Function,
    Global,
}
#[derive(Debug, Clone)]
pub struct Declaration {
    pub token: Token,
    pub ty: TypeId,
    pub kind: DeclarationKind,
    pub scope: usize,
    pub storage: Storage,
    pub linkage: Option<Linkage>,
    pub initial: Option<Expr>,
    pub defined: bool,
    pub tentative: bool,
    pub adjusted_parameter: bool,
}
#[derive(Debug, Clone)]
pub struct Unit {
    pub types: TypeContext,
    pub declarations: Vec<Declaration>,
    pub definitions: Vec<Definition>,
    pub main_symbol: Option<SymbolId>,
}
#[derive(Debug, Clone)]
pub struct Definition {
    pub symbol: SymbolId,
    pub parameters: Vec<SymbolId>,
    pub body: Statement,
}
impl Unit {
    pub fn main(&self) -> Option<&Statement> {
        self.main_symbol.and_then(|id| {
            self.definitions
                .iter()
                .find(|f| f.symbol == id)
                .map(|f| &f.body)
        })
    }
}
#[derive(Debug, Clone)]
struct Parameters {
    scope: usize,
    identities: Vec<Option<SymbolId>>,
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
    ArrayInitial(Vec<Expr>),
    Variable(SymbolId),
    String(String),
    Convert(Box<Expr>),
    Discard(Box<Expr>),
    Decay(Box<Expr>),
    Address(SymbolId),
    AddressLiteral(Box<Expr>),
    Dereference(Box<Expr>),
    PointerOffset {
        base: Box<Expr>,
        index: Box<Expr>,
        subtract: bool,
    },
    PointerDifference(Box<Expr>, Box<Expr>),
    IndirectAssign {
        address: Box<Expr>,
        value: Box<Expr>,
        operation: Option<(Binary, TypeId)>,
    },
    IndirectUpdate {
        address: Box<Expr>,
        increment: bool,
        postfix: bool,
    },
    VaStart {
        list: SymbolId,
        last: SymbolId,
    },
    VaArg {
        list: SymbolId,
    },
    VaCopy {
        destination: SymbolId,
        source: SymbolId,
    },
    VaEnd {
        list: SymbolId,
    },
    IndirectCall {
        callee: Box<Expr>,
        signature: TypeId,
        arguments: Vec<Expr>,
    },
    Call {
        symbol: SymbolId,
        signature: TypeId,
        arguments: Vec<Expr>,
    },
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
    ReturnVoid,
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
pub enum Storage {
    None,
    Typedef,
    Auto,
    Register,
    Extern,
    Static,
}
#[derive(Debug)]
enum Declarator {
    Name(Option<Token>),
    Pointer(Qualifiers, Box<Declarator>),
    Array(Box<Declarator>, Option<u64>),
    Function(Box<Declarator>, Vec<TypeId>, bool, bool, Parameters),
}
impl Declarator {
    fn apply(
        self,
        types: &mut TypeContext,
        base: TypeId,
    ) -> Result<(Option<Token>, TypeId, Option<Parameters>), String> {
        match self {
            Self::Name(name) => Ok((name, base, None)),
            Self::Pointer(q, child) => {
                let p = types.pointer(base)?;
                let p = types.qualify(p, q)?;
                child.apply(types, p)
            }
            Self::Array(child, n) => {
                let a = types.array(base, n)?;
                child.apply(types, a)
            }
            Self::Function(child, params, variadic, prototype, parameters) => {
                let f = types.function(base, params, variadic, prototype)?;
                let (name, ty, existing) = child.apply(types, f)?;
                Ok((name, ty, if ty == f { Some(parameters) } else { existing }))
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
    initializer_elements: usize,
    declarator_level: usize,
    declarator_operations: usize,
    loops: usize,
    breaks: usize,
    switches: Vec<Switch>,
    labels: HashSet<String>,
    gotos: Vec<Token>,
    current_result: Option<TypeId>,
    current_parameters: Vec<SymbolId>,
    current_variadic: bool,
    entities: HashMap<String, SymbolId>,
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
        initializer_elements: 0,
        declarator_level: 0,
        declarator_operations: 0,
        loops: 0,
        breaks: 0,
        switches: vec![],
        labels: HashSet::new(),
        gotos: vec![],
        current_result: None,
        current_parameters: vec![],
        current_variadic: false,
        entities: HashMap::new(),
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
            "_Alignof"
                | "_Atomic"
                | "_Generic"
                | "_Static_assert"
                | "_Thread_local"
                | "__builtin_va_list"
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
                | "__builtin_va_list"
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
        if token.text.starts_with("__builtin_va_") {
            return Err(error(&token, "reserved variadic compiler identifier"));
        }
        if kind == DeclarationKind::Function {
            return self.linked(token, ty, kind, Storage::None);
        }
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
                    && !matches!(
                        (
                            self.types.get(self.declarations[id.0].ty)?,
                            self.types.get(ty)?
                        ),
                        (
                            Type::Array {
                                length: Some(_),
                                ..
                            },
                            Type::Array { length: None, .. }
                        )
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
            storage: Storage::None,
            linkage: None,
            initial: None,
            defined: matches!(kind, DeclarationKind::Local | DeclarationKind::Parameter),
            tentative: false,
            adjusted_parameter: false,
        });
        Ok(id)
    }
    fn linked(
        &mut self,
        token: Token,
        ty: TypeId,
        kind: DeclarationKind,
        storage: Storage,
    ) -> Result<SymbolId, String> {
        if token.text.starts_with("__builtin_va_") {
            return Err(error(&token, "reserved variadic compiler identifier"));
        }
        let visible = self
            .lookup(&token.text)
            .and_then(|id| self.declarations[id.0].linkage);
        let linkage = if storage == Storage::Static {
            Linkage::Internal
        } else if storage == Storage::Extern || kind == DeclarationKind::Function {
            visible.unwrap_or(Linkage::External)
        } else {
            Linkage::External
        };
        if let Some(current) = self.scopes.last().unwrap().names.get(&token.text) {
            let d = &self.declarations[current.0];
            if d.kind != kind || d.linkage.is_none() {
                return Err(error(&token, "conflicting declaration in this scope"));
            }
        }
        let id = if let Some(id) = self.entities.get(&token.text).copied() {
            let d = &self.declarations[id.0];
            if d.kind != kind || d.linkage != Some(linkage) || !self.types.compatible(d.ty, ty)? {
                return Err(error(&token, "conflicting type or linkage declaration"));
            }
            if (kind != DeclarationKind::Function
                || matches!(
                    self.types.get(ty)?,
                    Type::Function {
                        prototype: true,
                        ..
                    }
                ))
                && !matches!(
                    (
                        self.types.get(self.declarations[id.0].ty)?,
                        self.types.get(ty)?
                    ),
                    (
                        Type::Array {
                            length: Some(_),
                            ..
                        },
                        Type::Array { length: None, .. }
                    )
                )
            {
                self.declarations[id.0].ty = ty;
            }
            id
        } else {
            if self.declarations.len() >= 65535 {
                return Err(error(&token, "declaration budget exceeded"));
            }
            let id = SymbolId(self.declarations.len());
            self.entities.insert(token.text.clone(), id);
            self.declarations.push(Declaration {
                token: token.clone(),
                ty,
                kind,
                scope: self.scopes.last().unwrap().id,
                storage,
                linkage: Some(linkage),
                initial: None,
                defined: false,
                tentative: false,
                adjusted_parameter: false,
            });
            id
        };
        self.scopes.last_mut().unwrap().names.insert(token.text, id);
        Ok(id)
    }
    fn static_initial(&self, e: &Expr) -> Result<(), String> {
        if let Expression::ArrayInitial(values) = &e.kind {
            for value in values {
                self.static_initial(value)?;
            }
            return Ok(());
        }

        if matches!(self.types.get(e.ty)?, Type::Pointer(_)) {
            address_constant(self, e)
        } else {
            constant(&self.types, e).map(|_| ())
        }
    }
    fn record_initial(
        &mut self,
        id: SymbolId,
        initial: Option<Expr>,
        definition: bool,
        tentative: bool,
    ) -> Result<(), String> {
        if let Some(e) = &initial {
            self.static_initial(e)?;
        }
        let declaration = &mut self.declarations[id.0];
        if definition && declaration.defined {
            return Err(error(
                &declaration.token,
                "duplicate initialized definition",
            ));
        }
        if definition {
            declaration.defined = true;
            declaration.initial = initial;
        }
        declaration.tentative |= tentative;
        Ok(())
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
                "extern" => {
                    storage += 1;
                    storage_kind = Storage::Extern;
                }
                "static" => {
                    storage += 1;
                    storage_kind = Storage::Static;
                }
                "__builtin_va_list"
                    if alias.is_none()
                        && signed + unsigned + short + long + int + char_ + bool_ + void == 0 =>
                {
                    alias = Some(self.types.pointer(self.types.integer(IntegerKind::Char))?);
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
                                | "__builtin_va_list"
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
                let parameter_scope = self.push_scope();
                let mut parameter_identities = vec![];
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
                            let (name, ty, _) = d
                                .apply(&mut self.types, base)
                                .map_err(|e| error(&token, e))?;
                            let id = if let Some(name) = name {
                                Some(self.define(name, ty, DeclarationKind::Parameter)?)
                            } else {
                                None
                            };
                            if let Some(id) = id {
                                self.declarations[id.0].storage = storage;
                            }
                            parameter_identities.push(id);
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
                direct = Declarator::Function(
                    Box::new(direct),
                    parameters,
                    variadic,
                    prototype,
                    Parameters {
                        scope: parameter_scope,
                        identities: parameter_identities,
                    },
                );
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
        let (name, ty, _) = declarator
            .apply(&mut self.types, base)
            .map_err(|e| error(&token, e))?;
        if name.is_some() {
            return Err(error(&token, "identifier in abstract type name"));
        }
        Ok(ty)
    }
    fn unit(mut self) -> Result<Unit, String> {
        let mut definitions = vec![];
        let mut main_symbol = None;
        let mut defined = HashSet::new();
        while self.peek().kind != Kind::End {
            let token = self.peek().clone();
            let (base, storage) = self.specifiers()?;
            let typedef = storage == Storage::Typedef;
            loop {
                let declarator = self.declarator(true, false)?;
                let (name, mut ty, parameters) = declarator
                    .apply(&mut self.types, base)
                    .map_err(|e| error(&token, e))?;
                let name = name.ok_or_else(|| error(&token, "declaration requires a name"))?;
                let function = matches!(self.types.get(ty)?, Type::Function { .. });
                let kind = if typedef {
                    DeclarationKind::Typedef
                } else if function {
                    DeclarationKind::Function
                } else {
                    DeclarationKind::Global
                };
                if self.peek().text == "{" {
                    if typedef || !function {
                        return Err(error(
                            &name,
                            "function declarator required for a definition",
                        ));
                    }
                    let parameters = parameters.ok_or_else(|| {
                        error(&name, "definition requires its own parameter declarator")
                    })?;
                    let Type::Function {
                        result,
                        parameters: types,
                        variadic,
                        ..
                    } = self.types.get(ty)?.clone()
                    else {
                        unreachable!()
                    };
                    let mut ids = vec![];
                    let mut names = HashMap::new();
                    for (id, ty) in parameters.identities.iter().zip(&types) {
                        let id = id.ok_or_else(|| {
                            error(&name, "function definition requires parameter names")
                        })?;
                        let declaration = &mut self.declarations[id.0];
                        if matches!(
                            self.types.get(self.types.unqualified(declaration.ty)?)?,
                            Type::Array { .. } | Type::Function { .. }
                        ) {
                            declaration.ty = *ty;
                            declaration.adjusted_parameter = true;
                        }
                        names.insert(declaration.token.text.clone(), id);
                        ids.push(id);
                    }
                    if ids.len() != types.len() {
                        return Err(error(&name, "definition parameter identity mismatch"));
                    }
                    if name.text == "main"
                        && (result != self.types.integer(IntegerKind::Int)
                            || !types.is_empty()
                            || variadic)
                    {
                        return Err(error(&name, "typed main requires int main(void)"));
                    }
                    ty = self.types.function(result, types, variadic, true)?;
                    let is_main = name.text == "main";
                    let id = self.linked(name.clone(), ty, kind, storage)?;
                    self.declarations[id.0].defined = true;
                    if !defined.insert(id) {
                        return Err(error(&name, "duplicate function definition"));
                    }
                    if is_main {
                        main_symbol = Some(id);
                    }
                    self.labels.clear();
                    self.gotos.clear();
                    self.current_result = Some(result);
                    self.current_parameters = ids.clone();
                    self.current_variadic = variadic;
                    self.scopes.push(Scope {
                        id: parameters.scope,
                        names,
                    });
                    let body = self.block_existing(parameters.scope)?;
                    self.scopes.pop();
                    self.current_result = None;
                    self.current_parameters.clear();
                    self.current_variadic = false;
                    for target in &self.gotos {
                        if !self.labels.contains(&target.text) {
                            return Err(error(target, "undefined label"));
                        }
                    }
                    definitions.push(Definition {
                        symbol: id,
                        parameters: ids,
                        body,
                    });
                    break;
                }
                let id = if matches!(kind, DeclarationKind::Function | DeclarationKind::Global) {
                    self.linked(name.clone(), ty, kind, storage)?
                } else {
                    self.define(name.clone(), ty, kind)?
                };
                let initial = if self.eat("=") {
                    if kind != DeclarationKind::Global {
                        return Err(error(&name, "initializer requires an object declaration"));
                    }
                    Some(self.object_initial(id, ty)?)
                } else {
                    None
                };
                if kind == DeclarationKind::Global {
                    let defined = initial.is_some();
                    self.record_initial(
                        id,
                        initial,
                        defined,
                        !defined && storage != Storage::Extern,
                    )?;
                }
                if !self.eat(",") {
                    self.expect(";")?;
                    break;
                }
            }
        }
        for declaration in &mut self.declarations {
            if declaration.kind == DeclarationKind::Global
                && declaration.tentative
                && let Type::Array {
                    element,
                    length: None,
                } = self.types.get(declaration.ty)?.clone()
            {
                declaration.ty = self.types.array(element, Some(1))?;
                self.types.pointer(declaration.ty)?;
            }
        }
        Ok(Unit {
            types: self.types,
            declarations: self.declarations,
            definitions,
            main_symbol,
        })
    }
    fn object_initial(&mut self, id: SymbolId, ty: TypeId) -> Result<Expr, String> {
        let ty = if matches!(self.types.get(ty)?, Type::Array { .. }) {
            self.declarations[id.0].ty
        } else {
            ty
        };
        let initial = self.initializer(ty, false)?;
        if matches!(self.types.get(ty)?, Type::Array { .. }) {
            self.declarations[id.0].ty = initial.ty;
        }
        Ok(initial)
    }
    fn initial_budget(&mut self) -> Result<(), String> {
        self.initializer_elements += 1;
        if self.initializer_elements > 65535 {
            return Err(error(self.peek(), "initializer element budget exceeded"));
        }
        Ok(())
    }
    fn zero_initial(&mut self, ty: TypeId, token: &Token) -> Result<Expr, String> {
        self.initial_budget()?;
        let ty = self.types.unqualified(ty)?;
        self.types.pointer(ty)?;
        let kind = match self.types.get(ty)?.clone() {
            Type::Array {
                element,
                length: Some(n),
            } => {
                if n > 65535 {
                    return Err(error(token, "initializer element budget exceeded"));
                }
                let mut values = vec![];
                for _ in 0..n {
                    values.push(self.zero_initial(element, token)?);
                }
                Expression::ArrayInitial(values)
            }
            Type::Integer(_) => Expression::Integer(IntegerValue::new(&self.types, ty, 0)?),
            Type::Pointer(_) => {
                let int = self.types.integer(IntegerKind::Int);
                Expression::Convert(Box::new(Expr {
                    token: token.clone(),
                    ty: int,
                    kind: Expression::Integer(IntegerValue::new(&self.types, int, 0)?),
                }))
            }
            _ => return Err(error(token, "unsupported zero initializer object")),
        };
        Ok(Expr {
            token: token.clone(),
            ty,
            kind,
        })
    }
    fn initializer(&mut self, ty: TypeId, elision: bool) -> Result<Expr, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "initializer nesting limit exceeded"));
        }
        self.depth += 1;
        let result = self.initializer_inner(ty, elision);
        self.depth -= 1;
        result
    }
    fn initializer_inner(&mut self, ty: TypeId, elision: bool) -> Result<Expr, String> {
        self.initial_budget()?;
        let token = self.peek().clone();
        let ty = self.types.unqualified(ty)?;
        self.types.pointer(ty)?;
        if let Type::Array { element, length } = self.types.get(ty)?.clone() {
            if length.is_some_and(|n| n > 65535) {
                return Err(error(&token, "initializer element budget exceeded"));
            }
            let braced = self.eat("{");
            if self.types.integer_kind(element).is_ok_and(|k| {
                matches!(
                    k,
                    IntegerKind::Char | IntegerKind::SignedChar | IntegerKind::UnsignedChar
                )
            }) && let Kind::String(text) = self.peek().kind.clone()
            {
                self.take();
                let n = length.unwrap_or(text.len() as u64 + 1);
                if (text.len() as u64) > n {
                    return Err(error(&token, "string initializer exceeds array bound"));
                }
                let mut bytes = text.into_bytes();
                bytes.resize(n as usize, 0);
                let element_ty = self.types.unqualified(element)?;
                self.types.pointer(element_ty)?;
                let mut values = vec![];
                for byte in bytes {
                    self.initial_budget()?;
                    values.push(Expr {
                        token: token.clone(),
                        ty: element_ty,
                        kind: Expression::Integer(IntegerValue::new(
                            &self.types,
                            element_ty,
                            byte as u64,
                        )?),
                    });
                }
                if braced {
                    self.eat(",");
                    self.expect("}")?;
                }
                let ty = self.types.array(element, Some(n))?;
                self.types.pointer(ty)?;
                self.types.pointer(element)?;
                return Ok(Expr {
                    token,
                    ty,
                    kind: Expression::ArrayInitial(values),
                });
            }
            if !braced && (!elision || length.is_none()) {
                return Err(error(
                    &token,
                    "array initializer requires braces or a character string",
                ));
            }
            let mut values = vec![];
            while length.is_none_or(|n| (values.len() as u64) < n) {
                if self.peek().text == "}" {
                    break;
                }
                if !values.is_empty() {
                    if !self.eat(",") {
                        break;
                    }
                    if self.peek().text == "}" {
                        break;
                    }
                }
                values.push(self.initializer(element, true)?);
            }
            if values.is_empty() {
                return Err(error(&token, "empty initializer is outside C11"));
            }
            if braced {
                self.eat(",");
                self.expect("}")?;
            }
            let n = length.unwrap_or(values.len() as u64);
            while (values.len() as u64) < n {
                values.push(self.zero_initial(element, &token)?);
            }
            let ty = self.types.array(element, Some(n))?;
            self.types.pointer(ty)?;
            self.types.pointer(element)?;
            Ok(Expr {
                token,
                ty,
                kind: Expression::ArrayInitial(values),
            })
        } else {
            let braced = self.eat("{");
            let e = if braced {
                self.initializer(ty, false)?
            } else {
                self.expr(1)?
            };
            if braced {
                self.eat(",");
                self.expect("}")?;
            }
            self.convert(e, ty)
        }
    }
    fn declaration(&mut self) -> Result<Statement, String> {
        let token = self.peek().clone();
        let (base, storage) = self.specifiers()?;
        let typedef = storage == Storage::Typedef;
        let mut declarations = vec![];
        loop {
            let d = self.declarator(true, false)?;
            let (name, ty, _) = d
                .apply(&mut self.types, base)
                .map_err(|e| error(&token, e))?;
            let name = name.ok_or_else(|| error(&token, "missing declaration name"))?;
            let kind = if typedef {
                DeclarationKind::Typedef
            } else if matches!(self.types.get(ty)?, Type::Function { .. }) {
                DeclarationKind::Function
            } else if matches!(storage, Storage::Static | Storage::Extern) {
                DeclarationKind::Global
            } else {
                DeclarationKind::Local
            };
            if kind == DeclarationKind::Function
                && matches!(storage, Storage::Auto | Storage::Register | Storage::Static)
            {
                return Err(error(&name, "invalid function storage class"));
            }
            if kind == DeclarationKind::Local
                && !matches!(self.types.get(ty)?, Type::Array { length: None, .. })
            {
                self.types.layout(ty).map_err(|e| error(&name, e))?;
            }
            let id = if kind == DeclarationKind::Function
                || (kind == DeclarationKind::Global && storage == Storage::Extern)
            {
                self.linked(name.clone(), ty, kind, storage)?
            } else {
                self.define(name.clone(), ty, kind)?
            };
            self.declarations[id.0].storage = storage;
            let initial = if self.eat("=") {
                if !matches!(kind, DeclarationKind::Local | DeclarationKind::Global)
                    || storage == Storage::Extern
                {
                    return Err(error(
                        self.peek(),
                        "only local objects can have initializers here",
                    ));
                }
                Some(self.object_initial(id, ty)?)
            } else {
                None
            };
            if kind == DeclarationKind::Local {
                self.types
                    .layout(self.declarations[id.0].ty)
                    .map_err(|e| error(&name, e))?;
            }
            if kind == DeclarationKind::Global && storage == Storage::Static {
                self.record_initial(id, initial, true, false)?;
            } else if kind == DeclarationKind::Local {
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
        let scope = self.push_scope();
        let result = self.block_existing(scope);
        self.scopes.pop();
        result
    }
    fn block_existing(&mut self, scope: usize) -> Result<Statement, String> {
        let token = self.peek().clone();
        self.expect("{")?;
        let mut items = vec![];
        while !self.eat("}") {
            if self.peek().kind == Kind::End {
                return Err(error(self.peek(), "unterminated block"));
            }
            items.push(self.statement()?);
        }
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
        let e = self.decay(e)?;
        self.scalar(e.ty).map_err(|e2| error(&e.token, e2))?;
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
                    let e = self.expr(0)?;
                    Some(self.decay(e)?)
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
                let e = self.expr(0)?;
                Some(self.decay(e)?)
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
            let result = self
                .current_result
                .ok_or_else(|| error(&token, "return outside a function"))?;
            if self.eat(";") {
                if result != self.types.void() {
                    return Err(error(&token, "nonvoid return requires a value"));
                }
                StatementKind::ReturnVoid
            } else {
                if result == self.types.void() {
                    return Err(error(&token, "void function cannot return an expression"));
                }
                let e = self.expr(0)?;
                let e = self.convert(e, self.types.unqualified(result)?)?;
                self.expect(";")?;
                StatementKind::Return(e)
            }
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
            let e = self.decay(e)?;
            self.expect(";")?;
            StatementKind::Expression(e)
        };
        Ok(Statement { token, kind })
    }
    fn scalar(&self, ty: TypeId) -> Result<(), String> {
        if matches!(
            self.types.get(self.types.unqualified(ty)?)?,
            Type::Integer(_) | Type::Pointer(_)
        ) {
            Ok(())
        } else {
            Err("scalar operand required".into())
        }
    }
    fn decay(&mut self, e: Expr) -> Result<Expr, String> {
        let base = match self.types.get(e.ty)? {
            Type::Array { element, .. } => Some(*element),
            Type::Function { .. } => Some(e.ty),
            _ => None,
        };
        if let Some(base) = base {
            self.types.pointer(e.ty)?;
            let ty = self.types.pointer(base)?;
            Ok(Expr {
                token: e.token.clone(),
                ty,
                kind: Expression::Decay(Box::new(e)),
            })
        } else {
            Ok(e)
        }
    }
    fn convert(&mut self, e: Expr, ty: TypeId) -> Result<Expr, String> {
        self.convert_core(e, ty, false)
    }
    fn cast(&mut self, e: Expr, ty: TypeId) -> Result<Expr, String> {
        self.convert_core(e, ty, true)
    }
    fn convert_core(&mut self, e: Expr, ty: TypeId, explicit: bool) -> Result<Expr, String> {
        let e = self.decay(e)?;
        let ty = self.types.unqualified(ty)?;
        if ty == self.types.void() {
            if !explicit {
                return Err(error(
                    &e.token,
                    "implicit conversion to void is not permitted",
                ));
            }
            return Ok(Expr {
                token: e.token.clone(),
                ty,
                kind: Expression::Discard(Box::new(e)),
            });
        }
        self.scalar(e.ty).map_err(|e2| error(&e.token, e2))?;
        self.scalar(ty).map_err(|e2| error(&e.token, e2))?;
        match (self.types.get(e.ty)?, self.types.get(ty)?) {
            (Type::Integer(_), Type::Integer(_)) => (),
            (Type::Pointer(source), Type::Pointer(target)) => {
                let (source, target) = (*source, *target);
                let s = self.types.unqualified(source)?;
                let t = self.types.unqualified(target)?;
                let sf = matches!(self.types.get(s)?, Type::Function { .. });
                let tf = matches!(self.types.get(t)?, Type::Function { .. });
                if explicit {
                    if sf != tf {
                        return Err(error(
                            &e.token,
                            "function/object pointer casts are outside the delivered subset",
                        ));
                    }
                } else {
                    let sq = self.types.qualifiers(source)?;
                    let tq = self.types.qualifiers(target)?;
                    if (sq.constant && !tq.constant)
                        || (sq.volatile && !tq.volatile)
                        || (sq.restrict && !tq.restrict)
                    {
                        return Err(error(&e.token, "pointer conversion discards qualifiers"));
                    }
                    if !(self.types.compatible(s, t)?
                        || s == self.types.void() && !tf
                        || t == self.types.void() && !sf)
                    {
                        return Err(error(&e.token, "incompatible pointer conversion"));
                    }
                }
            }
            (Type::Integer(_), Type::Pointer(_)) => {
                if !explicit && !constant(&self.types, &e).is_ok_and(|v| v.bits == 0) {
                    return Err(error(
                        &e.token,
                        "only integer constant zero converts to a pointer in this phase",
                    ));
                }
            }
            (Type::Pointer(_), Type::Integer(kind)) if explicit || *kind == IntegerKind::Bool => (),
            _ => return Err(error(&e.token, "incompatible scalar conversion")),
        }
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
    fn updated(&mut self, e: Expr, token: Token, postfix: bool) -> Result<Expr, String> {
        if let Expression::Dereference(address) = &e.kind {
            let Type::Pointer(base) = self.types.get(address.ty)? else {
                unreachable!()
            };
            if self.types.qualifiers(*base)?.constant {
                return Err(error(&token, "update through const pointer"));
            }
            if let Type::Pointer(element) = self.types.get(e.ty)? {
                self.types.layout(*element).map_err(|e| error(&token, e))?;
            } else {
                self.types
                    .integer_kind(e.ty)
                    .map_err(|e| error(&token, e))?;
            }
            Ok(Expr {
                ty: e.ty,
                kind: Expression::IndirectUpdate {
                    address: address.clone(),
                    increment: token.text == "++",
                    postfix,
                },
                token,
            })
        } else {
            let symbol = self.lvalue(&e)?;
            let ty = self.types.unqualified(self.declarations[symbol.0].ty)?;
            Ok(Expr {
                ty,
                kind: Expression::Update {
                    symbol,
                    increment: token.text == "++",
                    postfix,
                },
                token,
            })
        }
    }
    fn modifiable(&self, e: &Expr) -> Result<SymbolId, String> {
        let Expression::Variable(id) = e.kind else {
            return Err(error(&e.token, "modifiable scalar variable required"));
        };
        let d = &self.declarations[id.0];
        if !matches!(
            d.kind,
            DeclarationKind::Local | DeclarationKind::Parameter | DeclarationKind::Global
        ) || self.types.qualifiers(d.ty)?.constant
        {
            return Err(error(&e.token, "modifiable scalar variable required"));
        }
        self.scalar(d.ty).map_err(|e2| error(&e.token, e2))?;
        Ok(id)
    }
    fn lvalue(&self, e: &Expr) -> Result<SymbolId, String> {
        let id = self.modifiable(e)?;
        if let Type::Pointer(base) = self.types.get(e.ty)? {
            self.types.layout(*base).map_err(|e2| error(&e.token, e2))?;
        } else {
            self.types
                .integer_kind(e.ty)
                .map_err(|e2| error(&e.token, e2))?;
        }
        Ok(id)
    }
    fn common(&mut self, a: Expr, b: Expr) -> Result<(Expr, Expr, TypeId), String> {
        let a = self.decay(a)?;
        let b = self.decay(b)?;
        let ap = if let Type::Pointer(p) = self.types.get(a.ty)? {
            Some(*p)
        } else {
            None
        };
        let bp = if let Type::Pointer(p) = self.types.get(b.ty)? {
            Some(*p)
        } else {
            None
        };
        let ty = match (ap, bp) {
            (Some(ap), Some(bp)) => {
                let au = self.types.unqualified(ap)?;
                let bu = self.types.unqualified(bp)?;
                let base = if self.types.compatible(au, bu)?
                    || (au == self.types.void()
                        && !matches!(self.types.get(bu)?, Type::Function { .. }))
                {
                    au
                } else if bu == self.types.void()
                    && !matches!(self.types.get(au)?, Type::Function { .. })
                {
                    bu
                } else {
                    return Err(error(&a.token, "incompatible pointer operands"));
                };
                let aq = self.types.qualifiers(ap)?;
                let bq = self.types.qualifiers(bp)?;
                let base = self.types.qualify(
                    base,
                    Qualifiers {
                        constant: aq.constant || bq.constant,
                        volatile: aq.volatile || bq.volatile,
                        restrict: aq.restrict || bq.restrict,
                    },
                )?;
                self.types.pointer(base)?
            }
            (Some(_), None) => a.ty,
            (None, Some(_)) => b.ty,
            (None, None) if a.ty == self.types.void() && b.ty == self.types.void() => {
                self.types.void()
            }
            _ => self
                .types
                .common_integer(a.ty, b.ty)
                .map_err(|e| error(&a.token, e))?,
        };
        if ty == self.types.void() {
            return Ok((a, b, ty));
        }
        Ok((self.convert(a, ty)?, self.convert(b, ty)?, ty))
    }
    fn binary(&mut self, token: Token, op: Binary, a: Expr, b: Expr) -> Result<Expr, String> {
        let a = self.decay(a)?;
        let b = self.decay(b)?;
        let pointer = matches!(self.types.get(a.ty)?, Type::Pointer(_))
            || matches!(self.types.get(b.ty)?, Type::Pointer(_));
        if !pointer {
            self.types
                .integer_kind(a.ty)
                .map_err(|e| error(&token, e))?;
            self.types
                .integer_kind(b.ty)
                .map_err(|e| error(&token, e))?;
        }
        if pointer && matches!(op, Binary::Add | Binary::Subtract) {
            let (a, b) = if op == Binary::Add && !matches!(self.types.get(a.ty)?, Type::Pointer(_))
            {
                (b, a)
            } else {
                (a, b)
            };
            let Type::Pointer(element) = self.types.get(a.ty)? else {
                return Err(error(&token, "pointer arithmetic requires a pointer base"));
            };
            self.types.layout(*element).map_err(|e| error(&token, e))?;
            if matches!(self.types.get(b.ty)?, Type::Pointer(_)) {
                if op != Binary::Subtract {
                    return Err(error(&token, "cannot add pointers"));
                }
                let (a, b, _) = self.common(a, b)?;
                return Ok(Expr {
                    token,
                    ty: self.types.integer(IntegerKind::LongLong),
                    kind: Expression::PointerDifference(Box::new(a), Box::new(b)),
                });
            }
            self.types
                .integer_kind(b.ty)
                .map_err(|e| error(&token, e))?;
            let index = self.convert(b, self.types.integer(IntegerKind::LongLong))?;
            return Ok(Expr {
                token,
                ty: a.ty,
                kind: Expression::PointerOffset {
                    base: Box::new(a),
                    index: Box::new(index),
                    subtract: op == Binary::Subtract,
                },
            });
        }
        let (a, b) = if pointer {
            if op.comparison() && !matches!(op, Binary::Equal | Binary::NotEqual) {
                let (Type::Pointer(ap), Type::Pointer(bp)) =
                    (self.types.get(a.ty)?, self.types.get(b.ty)?)
                else {
                    return Err(error(
                        &token,
                        "pointer ordering requires two object pointers",
                    ));
                };
                self.types.layout(*ap).map_err(|e| error(&token, e))?;
                self.types.layout(*bp).map_err(|e| error(&token, e))?;
                if !self
                    .types
                    .compatible(self.types.unqualified(*ap)?, self.types.unqualified(*bp)?)?
                {
                    return Err(error(&token, "incompatible pointer ordering"));
                }
            }
            if !op.comparison() {
                return Err(error(
                    &token,
                    "pointer arithmetic/ordering waits for the memory phase",
                ));
            }
            let (a, b, _) = self.common(a, b)?;
            (a, b)
        } else if op.shift() {
            let at = self.types.promoted(a.ty)?;
            let bt = self.types.promoted(b.ty)?;
            (self.convert(a, at)?, self.convert(b, bt)?)
        } else {
            let (a, b, _) = self.common(a, b)?;
            (a, b)
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
    fn va_list_type(&mut self) -> Result<TypeId, String> {
        self.types.pointer(self.types.integer(IntegerKind::Char))
    }
    fn va_object(&mut self, e: &Expr, write: bool) -> Result<SymbolId, String> {
        let Expression::Variable(symbol) = e.kind else {
            return Err(error(&e.token, "va_list must be an object identifier"));
        };
        if write {
            self.modifiable(e)?;
        }
        if e.ty != self.va_list_type()? {
            return Err(error(&e.token, "__builtin_va_list object required"));
        }
        Ok(symbol)
    }
    fn va_expression(&mut self, token: Token) -> Result<Expr, String> {
        self.expect("(")?;
        let first = self.expr(1)?;
        let (ty, kind) = match token.text.as_str() {
            "__builtin_va_start" => {
                let list = self.va_object(&first, true)?;
                self.expect(",")?;
                let last = self.expr(1)?;
                let Expression::Variable(last) = last.kind else {
                    return Err(error(
                        &token,
                        "va_start requires the last named parameter identifier",
                    ));
                };
                if !self.current_variadic || self.current_parameters.last() != Some(&last) {
                    return Err(error(
                        &token,
                        "va_start is outside its variadic function or names the wrong last parameter",
                    ));
                }
                let declaration = &self.declarations[last.0];
                let last_type = self.types.unqualified(declaration.ty)?;
                if declaration.storage == Storage::Register
                    || declaration.adjusted_parameter
                    || (!matches!(self.types.get(last_type)?, Type::Pointer(_))
                        && self.types.promoted(last_type)? != last_type)
                {
                    return Err(error(
                        &token,
                        "va_start last parameter is incompatible with default argument promotion",
                    ));
                }
                (self.types.void(), Expression::VaStart { list, last })
            }
            "__builtin_va_arg" => {
                let list = self.va_object(&first, true)?;
                self.expect(",")?;
                let ty = self.type_name()?;
                let ty = self.types.unqualified(ty)?;
                self.scalar(ty).map_err(|e| error(&token, e))?;
                if !matches!(self.types.get(ty)?, Type::Pointer(_))
                    && self.types.promoted(ty)? != ty
                {
                    return Err(error(
                        &token,
                        "va_arg requires a default-promoted scalar type",
                    ));
                }
                (ty, Expression::VaArg { list })
            }
            "__builtin_va_copy" => {
                let destination = self.va_object(&first, true)?;
                self.expect(",")?;
                let second = self.expr(1)?;
                let source = self.va_object(&second, false)?;
                (
                    self.types.void(),
                    Expression::VaCopy {
                        destination,
                        source,
                    },
                )
            }
            _ => {
                let list = self.va_object(&first, true)?;
                (self.types.void(), Expression::VaEnd { list })
            }
        };
        self.expect(")")?;
        Ok(Expr { token, ty, kind })
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
        let lhs = self.prefix()?;
        let lhs = self.postfix(lhs)?;
        self.infix(lhs, min)
    }
    fn prefix(&mut self) -> Result<Expr, String> {
        let token = self.take();
        let lhs = match &token.kind {
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
                    let converted = self.cast(rhs, ty)?;
                    if ty == self.types.void() {
                        converted
                    } else {
                        Expr {
                            token: token.clone(),
                            ty: self.types.unqualified(ty)?,
                            kind: Expression::Convert(Box::new(converted)),
                        }
                    }
                } else {
                    let inner = self.expr(0)?;
                    self.expect(")")?;
                    inner
                }
            }
            _ if token.text == "*" => {
                let e = self.expr(13)?;
                let e = self.decay(e)?;
                let Type::Pointer(base) = self.types.get(e.ty)? else {
                    return Err(error(&token, "dereference requires a pointer"));
                };
                let base = *base;
                if base == self.types.void() {
                    return Err(error(&token, "cannot dereference void pointer"));
                }
                Expr {
                    token: token.clone(),
                    ty: self.types.unqualified(base)?,
                    kind: Expression::Dereference(Box::new(e)),
                }
            }
            _ if token.text == "&" => {
                let e = self.expr(13)?;
                if let Expression::Dereference(pointer) = e.kind {
                    *pointer
                } else {
                    if matches!(e.kind, Expression::String(_)) {
                        let ty = self.types.pointer(e.ty)?;
                        return Ok(Expr {
                            token,
                            ty,
                            kind: Expression::AddressLiteral(Box::new(e)),
                        });
                    }
                    let Expression::Variable(symbol) = e.kind else {
                        return Err(error(&token, "address requires an lvalue or function"));
                    };
                    let declaration = &self.declarations[symbol.0];
                    if declaration.storage == Storage::Register {
                        return Err(error(&token, "cannot take address of register object"));
                    }
                    if !matches!(
                        declaration.kind,
                        DeclarationKind::Global
                            | DeclarationKind::Local
                            | DeclarationKind::Parameter
                            | DeclarationKind::Function
                    ) {
                        return Err(error(&token, "address requires an object or function"));
                    }
                    let ty = self.types.pointer(declaration.ty)?;
                    Expr {
                        token: token.clone(),
                        ty,
                        kind: Expression::Address(symbol),
                    }
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
                let e = self.decay(e)?;
                let promoted = if op == Unary::Not {
                    self.scalar(e.ty).map_err(|e2| error(&token, e2))?;
                    e.ty
                } else {
                    self.types.promoted(e.ty).map_err(|e2| error(&token, e2))?
                };
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
                self.updated(e, token.clone(), false)?
            }
            _ if matches!(
                token.text.as_str(),
                "__builtin_va_start"
                    | "__builtin_va_arg"
                    | "__builtin_va_copy"
                    | "__builtin_va_end"
            ) =>
            {
                self.va_expression(token.clone())?
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
        Ok(lhs)
    }
    fn postfix(&mut self, mut lhs: Expr) -> Result<Expr, String> {
        loop {
            if self.eat("[") {
                let index = self.expr(0)?;
                let token = lhs.token.clone();
                self.expect("]")?;
                let pointer = self.binary(token.clone(), Binary::Add, lhs, index)?;
                let Type::Pointer(element) = self.types.get(pointer.ty)? else {
                    return Err(error(&token, "subscript requires a pointer and an integer"));
                };
                lhs = Expr {
                    token,
                    ty: self.types.unqualified(*element)?,
                    kind: Expression::Dereference(Box::new(pointer)),
                };
                continue;
            }
            if self.peek().text == "++" || self.peek().text == "--" {
                let op = self.take();
                lhs = self.updated(lhs, op, true)?;
                break;
            }
            if !self.eat("(") {
                break;
            }
            self.expression_operators += 1;
            if self.expression_operators > 128 {
                return Err(error(&lhs.token, "expression size limit exceeded"));
            }
            let direct = if let Expression::Variable(symbol) = lhs.kind {
                (self.declarations[symbol.0].kind == DeclarationKind::Function).then_some(symbol)
            } else {
                None
            };
            let callee = if direct.is_none() {
                Some(self.decay(lhs.clone())?)
            } else {
                None
            };
            let signature = if let Some(symbol) = direct {
                self.declarations[symbol.0].ty
            } else {
                let callee = callee.as_ref().ok_or("callee missing")?;
                let Type::Pointer(base) = self.types.get(callee.ty)? else {
                    return Err(error(
                        &lhs.token,
                        "called expression is not a function pointer",
                    ));
                };
                *base
            };
            let Type::Function {
                result,
                parameters,
                variadic,
                prototype,
            } = self.types.get(signature)?.clone()
            else {
                return Err(error(&lhs.token, "called expression has no function type"));
            };
            if !prototype {
                return Err(error(
                    &lhs.token,
                    "calls require a prototype in the current typed subset",
                ));
            }
            let mut arguments = vec![];
            if !self.eat(")") {
                loop {
                    arguments.push(self.expr(1)?);
                    if self.eat(")") {
                        break;
                    }
                    self.expect(",")?;
                }
            }
            if arguments.len() < parameters.len()
                || (!variadic && arguments.len() != parameters.len())
            {
                return Err(error(&lhs.token, "function argument count mismatch"));
            }
            let mut converted = vec![];
            for (index, argument) in arguments.into_iter().enumerate() {
                let argument = self.decay(argument)?;
                let ty = if let Some(ty) = parameters.get(index) {
                    *ty
                } else {
                    if matches!(self.types.get(argument.ty)?, Type::Pointer(_)) {
                        argument.ty
                    } else {
                        self.types.promoted(argument.ty)?
                    }
                };
                converted.push(self.convert(argument, ty)?);
            }
            lhs = Expr {
                token: lhs.token,
                ty: self.types.unqualified(result)?,
                kind: if let Some(symbol) = direct {
                    Expression::Call {
                        symbol,
                        signature,
                        arguments: converted,
                    }
                } else {
                    Expression::IndirectCall {
                        callee: Box::new(callee.ok_or("callee missing")?),
                        signature,
                        arguments: converted,
                    }
                },
            };
        }
        Ok(lhs)
    }
    fn infix(&mut self, mut lhs: Expr, min: u8) -> Result<Expr, String> {
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
                lhs = self.decay(lhs)?;
                self.scalar(lhs.ty).map_err(|e| error(&t, e))?;
                let a = self.expr(0)?;
                self.expect(":")?;
                let b = self.expr(2)?;
                let (a, b, ty) = self.common(a, b)?;
                lhs = Expr {
                    token: t,
                    ty,
                    kind: Expression::Conditional(Box::new(lhs), Box::new(a), Box::new(b)),
                };
                continue;
            }
            let rhs = self.expr(if assignment {
                precedence
            } else {
                precedence + 1
            })?;
            if assignment {
                if let Expression::Dereference(address) = &lhs.kind {
                    let Type::Pointer(base) = self.types.get(address.ty)? else {
                        unreachable!()
                    };
                    if self.types.qualifiers(*base)?.constant {
                        return Err(error(&t, "assignment through const pointer"));
                    }
                    self.scalar(*base).map_err(|e| error(&t, e))?;
                    let target = self.types.unqualified(*base)?;
                    let (value, operation) = if op == "=" {
                        (self.convert(rhs, target)?, None)
                    } else {
                        let binary = self.binary(
                            t.clone(),
                            binary_operator(&op[..op.len() - 1])
                                .ok_or("unsupported compound operation")?,
                            lhs.clone(),
                            rhs,
                        )?;
                        match binary.kind {
                            Expression::Binary(operator, left, right) => {
                                (*right, Some((operator, left.ty)))
                            }
                            Expression::PointerOffset {
                                base,
                                index,
                                subtract,
                            } => (
                                *index,
                                Some((
                                    if subtract {
                                        Binary::Subtract
                                    } else {
                                        Binary::Add
                                    },
                                    base.ty,
                                )),
                            ),
                            _ => return Err(error(&t, "invalid indirect compound operation")),
                        }
                    };
                    lhs = Expr {
                        token: t,
                        ty: target,
                        kind: Expression::IndirectAssign {
                            address: address.clone(),
                            value: Box::new(value),
                            operation,
                        },
                    };
                    continue;
                }
                let symbol = self.modifiable(&lhs)?;
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
                lhs = self.decay(lhs)?;
                let rhs = self.decay(rhs)?;
                let ty = rhs.ty;
                lhs = Expr {
                    token: t,
                    ty,
                    kind: Expression::Comma(Box::new(lhs), Box::new(rhs)),
                };
            } else if op == "&&" || op == "||" {
                lhs = self.decay(lhs)?;
                let rhs = self.decay(rhs)?;
                self.scalar(lhs.ty).map_err(|e| error(&t, e))?;
                self.scalar(rhs.ty).map_err(|e| error(&t, e))?;
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
fn address_constant(parser: &Parser, e: &Expr) -> Result<(), String> {
    match &e.kind {
        Expression::Address(symbol)
            if matches!(
                parser.declarations[symbol.0].kind,
                DeclarationKind::Global | DeclarationKind::Function
            ) =>
        {
            Ok(())
        }
        Expression::Decay(inner) if matches!(inner.kind, Expression::String(_)) => Ok(()),
        Expression::Decay(inner) if matches!(inner.kind,Expression::Variable(id) if matches!(parser.declarations[id.0].kind,DeclarationKind::Function|DeclarationKind::Global)) => {
            Ok(())
        }
        Expression::PointerOffset { base, index, .. } => {
            address_constant(parser, base)?;
            constant(&parser.types, index).map(|_| ())
        }
        Expression::AddressLiteral(_) => Ok(()),
        Expression::Decay(inner) if matches!(inner.kind, Expression::Dereference(_)) => {
            let Expression::Dereference(pointer) = &inner.kind else {
                unreachable!()
            };
            address_constant(parser, pointer)
        }
        Expression::Convert(inner) => address_constant(parser, inner),
        Expression::Conditional(condition, a, b) => {
            let condition = constant(&parser.types, condition)?;
            address_constant(parser, if condition.bits != 0 { a } else { b })
        }
        _ => {
            if constant(&parser.types, e).is_ok_and(|v| v.bits == 0) {
                Ok(())
            } else {
                Err(error(
                    &e.token,
                    "static pointer initializer requires a null or static object address",
                ))
            }
        }
    }
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
