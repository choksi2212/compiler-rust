//! C initializer syntax binding with real ordered subobject writes.
use super::*;
use crate::aggregate::RecordKind;
use crate::typed_ir::InitializerStep as Step;
#[derive(Debug, Clone)]
pub(super) enum RawInitializer {
    Value(Expr),
    List(Token, Vec<RawEntry>),
}
#[derive(Debug, Clone)]
pub(super) struct RawEntry {
    designators: Vec<Designator>,
    value: RawInitializer,
}
#[derive(Debug, Clone)]
enum Designator {
    Element(u64),
    Member(String),
}
impl RawInitializer {
    fn token(&self) -> &Token {
        match self {
            Self::Value(e) => &e.token,
            Self::List(t, _) => t,
        }
    }
}
impl Parser {
    pub(super) fn raw_initializer(&mut self) -> Result<RawInitializer, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "initializer nesting limit exceeded"));
        }
        self.depth += 1;
        let result = self.raw_initializer_inner();
        self.depth -= 1;
        result
    }
    fn raw_initializer_inner(&mut self) -> Result<RawInitializer, String> {
        self.initial_budget()?;
        let token = self.peek().clone();
        if !self.eat("{") {
            return self.expr(1).map(RawInitializer::Value);
        }
        let mut entries = vec![];
        if self.eat("}") {
            return Err(error(&token, "empty initializer is outside C11"));
        }
        loop {
            let mut designators = vec![];
            loop {
                if self.eat("[") {
                    let index = self.expr(1)?;
                    let value = constant(&self.types, &index)?.signed_value(&self.types)?;
                    let value = u64::try_from(value).map_err(|_| {
                        error(&index.token, "initializer index must be nonnegative")
                    })?;
                    if value >= 65535 {
                        return Err(error(&index.token, "initializer index budget exceeded"));
                    }
                    self.expect("]")?;
                    designators.push(Designator::Element(value));
                } else if self.eat(".") {
                    let name = self.take();
                    if name.kind != Kind::Word || keyword(&name.text) {
                        return Err(error(&name, "expected initializer member name"));
                    }
                    designators.push(Designator::Member(name.text));
                } else {
                    break;
                }
                if designators.len() > 128 {
                    return Err(error(
                        &token,
                        "initializer designator nesting limit exceeded",
                    ));
                }
            }
            if !designators.is_empty() {
                self.expect("=")?;
            }
            let value = self.raw_initializer()?;
            entries.push(RawEntry { designators, value });
            if self.eat("}") {
                break;
            }
            self.expect(",")?;
            if self.eat("}") {
                break;
            }
        }
        Ok(RawInitializer::List(token, entries))
    }
    fn prepare_initial_type(
        &mut self,
        ty: TypeId,
        seen: &mut HashSet<TypeId>,
    ) -> Result<(), String> {
        if !seen.insert(ty) {
            return Ok(());
        }
        self.types.pointer(ty)?;
        let plain = self.types.unqualified(ty)?;
        self.types.pointer(plain)?;
        match self.types.get(plain)?.clone() {
            Type::Array { element, length } => {
                if length.is_some_and(|n| n > 65535) {
                    return Err(error(self.peek(), "initializer element budget exceeded"));
                }
                self.prepare_initial_type(element, seen)?;
            }
            Type::Record(_) => {
                let fields = self
                    .types
                    .record_type(plain)?
                    .fields
                    .as_ref()
                    .ok_or("initializer of incomplete record")?
                    .clone();
                for field in fields {
                    if !matches!(
                        self.types.get(self.types.unqualified(field.ty)?)?,
                        Type::Array { length: None, .. }
                    ) {
                        self.prepare_initial_type(field.ty, seen)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn char_array(&self, ty: TypeId) -> Result<bool, String> {
        Ok(
            if let Type::Array { element, .. } = self.types.get(self.types.unqualified(ty)?)? {
                self.types.integer_kind(*element).is_ok_and(|k| {
                    matches!(
                        k,
                        IntegerKind::Char | IntegerKind::SignedChar | IntegerKind::UnsignedChar
                    )
                })
            } else {
                false
            },
        )
    }
    fn bind_string_initial(&mut self, ty: TypeId, e: &Expr) -> Result<Expr, String> {
        let Type::Array { element, length } = self.types.get(self.types.unqualified(ty)?)?.clone()
        else {
            return Err("character array required".into());
        };
        let Expression::String(text) = &e.kind else {
            return Err("string object required".into());
        };
        let n = length.unwrap_or(text.len() as u64 + 1);
        if n > 65535 {
            return Err(error(&e.token, "initializer element budget exceeded"));
        }
        if text.len() as u64 > n {
            return Err(error(&e.token, "string initializer exceeds array bound"));
        }
        let ty = self.types.array(element, Some(n))?;
        self.prepare_initial_type(ty, &mut HashSet::new())?;
        let value_ty = self.types.unqualified(element)?;
        let mut writes = vec![];
        // The object is zeroed before these actual source bytes are stored; an exact
        // character bound intentionally omits the terminator as C permits.
        for (i, byte) in text.bytes().enumerate() {
            self.initial_budget()?;
            writes.push(InitialWrite {
                path: vec![Step::Element(i as u64)],
                value: Expr {
                    token: e.token.clone(),
                    ty: value_ty,
                    kind: Expression::Integer(IntegerValue::new(
                        &self.types,
                        value_ty,
                        u64::from(byte),
                    )?),
                },
            });
        }
        Ok(Expr {
            token: e.token.clone(),
            ty,
            kind: Expression::ObjectInitial(writes),
        })
    }
    fn child_steps(&self, ty: TypeId) -> Result<Vec<Step>, String> {
        match self.types.get(self.types.unqualified(ty)?)? {
            Type::Array { length, .. } => Ok(if length == &Some(0) {
                vec![]
            } else {
                vec![Step::Element(0)]
            }),
            Type::Record(_) => {
                let record = self.types.record_type(ty)?;
                let fields = record
                    .fields
                    .as_ref()
                    .ok_or("initializer of incomplete record")?;
                let mut children = vec![];
                for (i, field) in fields.iter().enumerate() {
                    if (field.name.is_some() || field.anonymous)
                        && field.bits.is_none_or(|b| b.width != 0)
                        && !matches!(
                            self.types.get(self.types.unqualified(field.ty)?)?,
                            Type::Array { length: None, .. }
                        )
                    {
                        children.push(Step::Member(i));
                    }
                }
                if record.kind == RecordKind::Union {
                    children.truncate(1);
                }
                Ok(children)
            }
            _ => Ok(vec![]),
        }
    }
    fn path_type(&self, root: TypeId, path: &[Step]) -> Result<TypeId, String> {
        let mut ty = root;
        for (position, step) in path.iter().enumerate() {
            match step {
                Step::Element(index) => {
                    let Type::Array { element, length } =
                        self.types.get(self.types.unqualified(ty)?)?
                    else {
                        return Err("array designator applied to non-array object".into());
                    };
                    if length.is_some_and(|n| *index >= n) {
                        return Err("array initializer designator exceeds bound".into());
                    }
                    ty = *element;
                }
                Step::Member(index) => {
                    let field = self
                        .types
                        .record_type(ty)?
                        .fields
                        .as_ref()
                        .ok_or("initializer of incomplete record")?
                        .get(*index)
                        .ok_or("invalid initializer member identity")?;
                    if field.bits.is_some() && position + 1 != path.len() {
                        return Err("cannot designate within a bitfield".into());
                    }
                    if matches!(
                        self.types.get(self.types.unqualified(field.ty)?)?,
                        Type::Array { length: None, .. }
                    ) {
                        return Err("flexible array member cannot be initialized".into());
                    }
                    ty = field.ty;
                }
            }
        }
        Ok(ty)
    }
    fn designation(&self, root: TypeId, designators: &[Designator]) -> Result<Vec<Step>, String> {
        let mut path = vec![];
        let mut ty = root;
        for designator in designators {
            match designator {
                Designator::Element(index) => {
                    path.push(Step::Element(*index));
                    ty = self.path_type(root, &path)?;
                }
                Designator::Member(name) => {
                    let member = self.types.member(ty, name)?;
                    path.extend(member.path.into_iter().map(Step::Member));
                    ty = self.path_type(root, &path)?;
                }
            }
        }
        Ok(path)
    }
    fn following_initial_path(
        &self,
        root: TypeId,
        path: &[Step],
    ) -> Result<Option<Vec<Step>>, String> {
        let mut path = path.to_vec();
        while let Some(last) = path.pop() {
            let parent = self.path_type(root, &path)?;
            let next = match last {
                Step::Element(index) => {
                    let Type::Array { length, .. } =
                        self.types.get(self.types.unqualified(parent)?)?
                    else {
                        return Err("initializer array cursor mismatch".into());
                    };
                    let next = index.checked_add(1).ok_or("initializer index overflow")?;
                    if length.is_none_or(|n| next < n) {
                        Some(Step::Element(next))
                    } else {
                        None
                    }
                }
                Step::Member(index) => {
                    let record = self.types.record_type(parent)?;
                    if record.kind == RecordKind::Union {
                        None
                    } else {
                        self.child_steps(parent)?
                            .into_iter()
                            .find(|step| matches!(step,Step::Member(n) if *n>index))
                    }
                }
            };
            if let Some(next) = next {
                path.push(next);
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
    fn aggregate_initial_type(&self, ty: TypeId) -> Result<bool, String> {
        Ok(matches!(
            self.types.get(self.types.unqualified(ty)?)?,
            Type::Array { .. } | Type::Record(_)
        ))
    }
    pub(super) fn bind_initializer(
        &mut self,
        ty: TypeId,
        raw: &RawInitializer,
    ) -> Result<Expr, String> {
        let ty = self.types.unqualified(ty)?;
        self.prepare_initial_type(ty, &mut HashSet::new())?;
        let token = raw.token().clone();
        match raw {
            RawInitializer::Value(value) => {
                if self.char_array(ty)? && matches!(value.kind, Expression::String(_)) {
                    return self.bind_string_initial(ty, value);
                }
                if matches!(self.types.get(ty)?, Type::Array { .. }) {
                    return Err(error(
                        &token,
                        "array initializer requires braces or a character string",
                    ));
                }
                self.convert(value.clone(), ty)
            }
            RawInitializer::List(_, entries) => {
                if !self.aggregate_initial_type(ty)? {
                    if entries.len() != 1 || !entries[0].designators.is_empty() {
                        return Err(error(
                            &token,
                            "scalar initializer has excess entries or a designator",
                        ));
                    }
                    return self.bind_initializer(ty, &entries[0].value);
                }
                if self.char_array(ty)?
                    && entries.len() == 1
                    && entries[0].designators.is_empty()
                    && matches!(
                        &entries[0].value,
                        RawInitializer::Value(Expr {
                            kind: Expression::String(_),
                            ..
                        })
                    )
                {
                    let RawInitializer::Value(value) = &entries[0].value else {
                        unreachable!()
                    };
                    return self.bind_string_initial(ty, value);
                }
                let mut cursor = self.child_steps(ty)?.first().copied().map(|s| vec![s]);
                let mut writes = vec![];
                let mut extent = 0u64;
                let mut union_members: HashMap<Vec<Step>, usize> = HashMap::new();
                for entry in entries {
                    let mut path = if entry.designators.is_empty() {
                        cursor.clone().ok_or_else(|| {
                            error(entry.value.token(), "excess aggregate initializer entry")
                        })?
                    } else {
                        self.designation(ty, &entry.designators)
                            .map_err(|e| error(entry.value.token(), e))?
                    };
                    let mut target = self.path_type(ty, &path)?;
                    if let RawInitializer::Value(value) = &entry.value {
                        while self.aggregate_initial_type(target)? {
                            if self.char_array(target)?
                                && matches!(value.kind, Expression::String(_))
                            {
                                break;
                            }
                            if self.types.aggregate_value(target)?
                                && self.types.compatible(
                                    self.types.unqualified(value.ty)?,
                                    self.types.unqualified(target)?,
                                )?
                            {
                                break;
                            }
                            let step = self
                                .child_steps(target)?
                                .first()
                                .copied()
                                .ok_or("aggregate has no initializable subobject")?;
                            path.push(step);
                            target = self.path_type(ty, &path)?;
                        }
                    }
                    // Selecting a different union member discards the previous representation,
                    // including relocations. Other containing-struct fields remain untouched.
                    for i in 0..path.len() {
                        if let Step::Member(member) = path[i] {
                            let ancestor = self.path_type(ty, &path[..i])?;
                            if self.types.record_type(ancestor)?.kind == RecordKind::Union
                                && union_members
                                    .insert(path[..i].to_vec(), member)
                                    .is_some_and(|previous| previous != member)
                            {
                                writes.push(InitialWrite {
                                    path: path[..i].to_vec(),
                                    value: Expr {
                                        token: token.clone(),
                                        ty: self.types.unqualified(ancestor)?,
                                        kind: Expression::ObjectInitial(vec![]),
                                    },
                                });
                            }
                        }
                    }
                    let value = self.bind_initializer(target, &entry.value)?;
                    if let Some(Step::Element(index)) = path.first() {
                        extent = extent.max(index + 1);
                    }
                    cursor = self.following_initial_path(ty, &path)?;
                    writes.push(InitialWrite { path, value });
                }
                let ty = if let Type::Array {
                    element,
                    length: None,
                } = self.types.get(ty)?.clone()
                {
                    if extent == 0 {
                        return Err(error(&token, "cannot infer empty array bound"));
                    }
                    self.types.array(element, Some(extent))?
                } else {
                    ty
                };
                self.prepare_initial_type(ty, &mut HashSet::new())?;
                Ok(Expr {
                    token,
                    ty,
                    kind: Expression::ObjectInitial(writes),
                })
            }
        }
    }
}

impl Parser {
    pub(super) fn compound_literal(&mut self, ty: TypeId, token: Token) -> Result<Expr, String> {
        let raw = self.raw_initializer()?;
        let initial = self.bind_initializer(ty, &raw)?;
        let ty = if matches!(
            self.types.get(self.types.unqualified(ty)?)?,
            Type::Array { length: None, .. }
        ) {
            initial.ty
        } else {
            ty
        };
        self.types.layout(ty).map_err(|e| error(&token, e))?;
        if self.declarations.len() >= 65535 {
            return Err(error(&token, "declaration budget exceeded"));
        }
        let static_duration = self.current_result.is_none();
        if static_duration {
            self.static_initial(&initial)?;
        }
        let symbol = SymbolId(self.declarations.len());
        let mut name = token.clone();
        name.kind = Kind::Word;
        name.text = format!("__compiler_compound_{}", symbol.0);
        self.declarations.push(Declaration {
            token: name,
            ty,
            kind: if static_duration {
                DeclarationKind::Global
            } else {
                DeclarationKind::CompoundLocal
            },
            scope: self.scopes.last().unwrap().id,
            storage: if static_duration {
                Storage::Static
            } else {
                Storage::Auto
            },
            linkage: None,
            initial: if static_duration {
                Some(initial.clone())
            } else {
                None
            },
            defined: true,
            tentative: false,
            adjusted_parameter: false,
            alignment: None,
        });
        let pointer = self.types.pointer(ty)?;
        let address = Expr {
            token: token.clone(),
            ty: pointer,
            kind: Expression::CompoundAddress {
                symbol,
                initial: if static_duration {
                    None
                } else {
                    Some(Box::new(initial))
                },
            },
        };
        Ok(Expr {
            token,
            ty: self.types.unqualified(ty)?,
            kind: Expression::Dereference(Box::new(address)),
        })
    }
}
