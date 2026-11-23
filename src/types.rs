//! Canonical Windows x64 LLP64 C types. No Rust-host layout assumptions.
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerKind {
    Bool,
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    UnsignedShort,
    Int,
    UnsignedInt,
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
}
impl IntegerKind {
    pub const ALL: [Self; 12] = [
        Self::Bool,
        Self::Char,
        Self::SignedChar,
        Self::UnsignedChar,
        Self::Short,
        Self::UnsignedShort,
        Self::Int,
        Self::UnsignedInt,
        Self::Long,
        Self::UnsignedLong,
        Self::LongLong,
        Self::UnsignedLongLong,
    ];
    pub fn bits(self) -> u32 {
        match self {
            Self::Bool | Self::Char | Self::SignedChar | Self::UnsignedChar => 8,
            Self::Short | Self::UnsignedShort => 16,
            Self::Int | Self::UnsignedInt | Self::Long | Self::UnsignedLong => 32,
            Self::LongLong | Self::UnsignedLongLong => 64,
        }
    }
    pub fn rank(self) -> u8 {
        match self {
            Self::Bool => 0,
            Self::Char | Self::SignedChar | Self::UnsignedChar => 1,
            Self::Short | Self::UnsignedShort => 2,
            Self::Int | Self::UnsignedInt => 3,
            Self::Long | Self::UnsignedLong => 4,
            Self::LongLong | Self::UnsignedLongLong => 5,
        }
    }
    pub fn signed(self) -> bool {
        matches!(
            self,
            Self::Char | Self::SignedChar | Self::Short | Self::Int | Self::Long | Self::LongLong
        )
    }
    pub fn unsigned(self) -> Self {
        match self {
            Self::Char | Self::SignedChar => Self::UnsignedChar,
            Self::Short => Self::UnsignedShort,
            Self::Int => Self::UnsignedInt,
            Self::Long => Self::UnsignedLong,
            Self::LongLong => Self::UnsignedLongLong,
            other => other,
        }
    }
    pub fn maximum(self) -> u64 {
        if self == Self::Bool {
            1
        } else if self.signed() {
            (1u64 << (self.bits() - 1)) - 1
        } else {
            mask(self.bits())
        }
    }
}
pub fn mask(bits: u32) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Qualifiers {
    pub constant: bool,
    pub volatile: bool,
    pub restrict: bool,
}
impl Qualifiers {
    fn union(self, other: Self) -> Self {
        Self {
            constant: self.constant || other.constant,
            volatile: self.volatile || other.volatile,
            restrict: self.restrict || other.restrict,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Integer(IntegerKind),
    Pointer(TypeId),
    Array {
        element: TypeId,
        length: Option<u64>,
    },
    Function {
        result: TypeId,
        parameters: Vec<TypeId>,
        variadic: bool,
        prototype: bool,
    },
    Qualified(TypeId, Qualifiers),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub align: u32,
}

#[derive(Debug, Clone)]
pub struct TypeContext {
    records: Vec<Type>,
    canonical: HashMap<Type, TypeId>,
    depths: Vec<u16>,
}
impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}
impl TypeContext {
    pub fn new() -> Self {
        let mut context = Self {
            records: vec![],
            canonical: HashMap::new(),
            depths: vec![],
        };
        for ty in std::iter::once(Type::Void).chain(IntegerKind::ALL.map(Type::Integer)) {
            let id = TypeId(context.records.len());
            context.canonical.insert(ty.clone(), id);
            context.records.push(ty);
            context.depths.push(0);
        }
        context
    }
    pub fn void(&self) -> TypeId {
        TypeId(0)
    }
    pub fn integer(&self, kind: IntegerKind) -> TypeId {
        self.canonical[&Type::Integer(kind)]
    }
    pub fn get(&self, id: TypeId) -> Result<&Type, String> {
        self.records
            .get(id.0)
            .ok_or_else(|| format!("invalid type identity {}", id.0))
    }
    pub fn unqualified(&self, id: TypeId) -> Result<TypeId, String> {
        match self.get(id)? {
            Type::Qualified(base, _) => Ok(*base),
            _ => Ok(id),
        }
    }
    pub fn qualifiers(&self, id: TypeId) -> Result<Qualifiers, String> {
        match self.get(id)? {
            Type::Qualified(_, q) => Ok(*q),
            _ => Ok(Qualifiers::default()),
        }
    }
    fn insert(&mut self, ty: Type) -> Result<TypeId, String> {
        if let Some(id) = self.canonical.get(&ty) {
            return Ok(*id);
        }
        if self.records.len() >= 65535 {
            return Err("canonical type budget exceeded".into());
        }
        let children: Vec<TypeId> = match &ty {
            Type::Pointer(base) | Type::Qualified(base, _) => vec![*base],
            Type::Array { element, .. } => vec![*element],
            Type::Function {
                result, parameters, ..
            } => std::iter::once(*result)
                .chain(parameters.iter().copied())
                .collect(),
            _ => vec![],
        };
        let depth = children
            .iter()
            .map(|id| self.depths[id.0])
            .max()
            .unwrap_or(0)
            + 1;
        if depth > 128 {
            return Err("type derivation nesting limit exceeded".into());
        }
        let id = TypeId(self.records.len());
        self.canonical.insert(ty.clone(), id);
        self.records.push(ty);
        self.depths.push(depth);
        Ok(id)
    }
    pub fn pointer(&mut self, base: TypeId) -> Result<TypeId, String> {
        self.get(base)?;
        self.insert(Type::Pointer(base))
    }
    pub fn qualify(&mut self, base: TypeId, q: Qualifiers) -> Result<TypeId, String> {
        let q = q.union(self.qualifiers(base)?);
        let base = self.unqualified(base)?;
        if q == Qualifiers::default() {
            return Ok(base);
        }
        if let Type::Array { element, length } = self.get(base)?.clone() {
            let element = self.qualify(element, q)?;
            return self.array(element, length);
        }
        if matches!(self.get(base)?, Type::Function { .. }) {
            return Err("function type cannot be qualified".into());
        }
        if q.restrict {
            let Type::Pointer(target) = self.get(base)? else {
                return Err("restrict requires a pointer to an object type".into());
            };
            if matches!(self.get(self.unqualified(*target)?)?, Type::Function { .. }) {
                return Err("restrict requires a pointer to an object type".into());
            }
        }
        self.insert(Type::Qualified(base, q))
    }
    pub fn array(&mut self, element: TypeId, length: Option<u64>) -> Result<TypeId, String> {
        self.layout(element)?;
        if length == Some(0) {
            return Err("array bound must be positive".into());
        }
        let ty = Type::Array { element, length };
        if let Some(length) = length {
            self.layout(element)?
                .size
                .checked_mul(length)
                .ok_or("array layout exceeds size_t")?;
        }
        self.insert(ty)
    }
    pub fn function(
        &mut self,
        result: TypeId,
        parameters: Vec<TypeId>,
        variadic: bool,
        prototype: bool,
    ) -> Result<TypeId, String> {
        if matches!(
            self.get(self.unqualified(result)?)?,
            Type::Array { .. } | Type::Function { .. }
        ) {
            return Err("function cannot return an array or function type".into());
        }
        if parameters.len() > 1024 {
            return Err("function parameter type budget exceeded".into());
        }
        if variadic && (parameters.is_empty() || !prototype) {
            return Err("variadic signature requires named parameters and a prototype".into());
        }
        if !prototype && !parameters.is_empty() {
            return Err("nonprototype signature cannot contain typed parameters".into());
        }
        let mut adjusted = vec![];
        for parameter in parameters {
            let parameter = self.unqualified(parameter)?;
            let parameter = match self.get(parameter)?.clone() {
                Type::Void => return Err("void parameter must be the sole (void) spelling".into()),
                Type::Array { element, .. } => self.pointer(element)?,
                Type::Function { .. } => self.pointer(parameter)?,
                _ => parameter,
            };
            adjusted.push(parameter);
        }
        self.insert(Type::Function {
            result,
            parameters: adjusted,
            variadic,
            prototype,
        })
    }
    pub fn integer_kind(&self, ty: TypeId) -> Result<IntegerKind, String> {
        match self.get(self.unqualified(ty)?)? {
            Type::Integer(kind) => Ok(*kind),
            _ => Err("integer type required".into()),
        }
    }
    pub fn layout(&self, ty: TypeId) -> Result<Layout, String> {
        match self.get(ty)? {
            Type::Integer(kind) => Ok(Layout {
                size: u64::from(kind.bits() / 8),
                align: kind.bits() / 8,
            }),
            Type::Pointer(_) => Ok(Layout { size: 8, align: 8 }),
            Type::Qualified(base, _) => self.layout(*base),
            Type::Array {
                element,
                length: Some(n),
            } => {
                let element = self.layout(*element)?;
                Ok(Layout {
                    size: element
                        .size
                        .checked_mul(*n)
                        .ok_or("array layout exceeds size_t")?,
                    align: element.align,
                })
            }
            Type::Array { length: None, .. } => Err("incomplete array has no object size".into()),
            Type::Void | Type::Function { .. } => {
                Err("sizeof/layout requires a complete object type".into())
            }
        }
    }
    pub fn promoted(&self, ty: TypeId) -> Result<TypeId, String> {
        let kind = self.integer_kind(ty)?;
        Ok(if kind.rank() < IntegerKind::Int.rank() {
            self.integer(IntegerKind::Int)
        } else {
            self.integer(kind)
        })
    }
    pub fn common_integer(&self, a: TypeId, b: TypeId) -> Result<TypeId, String> {
        let a = self.integer_kind(self.promoted(a)?)?;
        let b = self.integer_kind(self.promoted(b)?)?;
        let result = if a.signed() == b.signed() {
            if a.rank() >= b.rank() { a } else { b }
        } else {
            let (signed, unsigned) = if a.signed() { (a, b) } else { (b, a) };
            if unsigned.rank() >= signed.rank() {
                unsigned
            } else if signed.maximum() >= unsigned.maximum() {
                signed
            } else {
                signed.unsigned()
            }
        };
        Ok(self.integer(result))
    }
    pub fn compatible(&self, a: TypeId, b: TypeId) -> Result<bool, String> {
        let (a_type, b_type) = (self.get(a)?, self.get(b)?);
        if a == b {
            return Ok(true);
        }
        Ok(match (a_type, b_type) {
            (Type::Qualified(a, qa), Type::Qualified(b, qb)) => {
                qa == qb && self.compatible(*a, *b)?
            }
            (Type::Pointer(a), Type::Pointer(b)) => self.compatible(*a, *b)?,
            (
                Type::Array {
                    element: a,
                    length: al,
                },
                Type::Array {
                    element: b,
                    length: bl,
                },
            ) => (al.is_none() || bl.is_none() || al == bl) && self.compatible(*a, *b)?,
            (
                Type::Function {
                    result: a,
                    parameters: ap,
                    variadic: av,
                    prototype: at,
                },
                Type::Function {
                    result: b,
                    parameters: bp,
                    variadic: bv,
                    prototype: bt,
                },
            ) => {
                self.compatible(*a, *b)?
                    && if *at && *bt {
                        av == bv
                            && ap.len() == bp.len()
                            && ap
                                .iter()
                                .zip(bp)
                                .all(|(a, b)| self.compatible(*a, *b).unwrap_or(false))
                    } else if !*at && !*bt {
                        true
                    } else {
                        let (parameters, variadic) = if *at { (ap, av) } else { (bp, bv) };
                        !*variadic
                            && parameters.iter().all(|ty| {
                                self.promoted(*ty).is_ok_and(|promoted| promoted == *ty)
                                    || matches!(self.get(*ty), Ok(Type::Pointer(_)))
                            })
                    }
            }
            _ => false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegerValue {
    pub ty: TypeId,
    pub bits: u64,
}
impl IntegerValue {
    pub fn new(types: &TypeContext, ty: TypeId, bits: u64) -> Result<Self, String> {
        let kind = types.integer_kind(ty)?;
        let bits = if kind == IntegerKind::Bool {
            u64::from(bits != 0)
        } else {
            bits & mask(kind.bits())
        };
        Ok(Self {
            ty: types.unqualified(ty)?,
            bits,
        })
    }
    pub fn validate(self, types: &TypeContext) -> Result<(), String> {
        let kind = types.integer_kind(self.ty)?;
        if types.unqualified(self.ty)? != self.ty
            || self.bits & !mask(kind.bits()) != 0
            || (kind == IntegerKind::Bool && self.bits > 1)
        {
            return Err("noncanonical integer value".into());
        }
        Ok(())
    }
    pub fn signed_value(self, types: &TypeContext) -> Result<i128, String> {
        self.validate(types)?;
        let kind = types.integer_kind(self.ty)?;
        if !kind.signed() {
            return Ok(i128::from(self.bits));
        }
        let sign = 1u64 << (kind.bits() - 1);
        Ok(if self.bits & sign == 0 {
            i128::from(self.bits)
        } else {
            i128::from(self.bits) - (1i128 << kind.bits())
        })
    }
    pub fn cast(self, types: &TypeContext, target: TypeId) -> Result<Self, String> {
        self.validate(types)?;
        let kind = types.integer_kind(target)?;
        if kind == IntegerKind::Bool {
            return Self::new(types, target, u64::from(self.bits != 0));
        }
        Self::new(types, target, self.signed_value(types)? as u64)
    }
}

/// C11 6.4.4.1 integer constant candidate lists, using frozen target ranks/widths.
pub fn integer_literal(
    types: &TypeContext,
    magnitude: u64,
    radix: u32,
    suffix: &str,
) -> Result<IntegerValue, String> {
    use IntegerKind::*;
    if ![8, 10, 16].contains(&radix) {
        return Err("unsupported integer radix".into());
    }
    if suffix.contains('l') && suffix.contains('L') {
        return Err("mixed-case long-long suffix is invalid".into());
    }
    let suffix = suffix.to_ascii_lowercase();
    let decimal = radix == 10;
    let candidates: &[IntegerKind] = match (suffix.as_str(), decimal) {
        ("", true) => &[Int, Long, LongLong],
        ("", false) => &[
            Int,
            UnsignedInt,
            Long,
            UnsignedLong,
            LongLong,
            UnsignedLongLong,
        ],
        ("u", _) => &[UnsignedInt, UnsignedLong, UnsignedLongLong],
        ("l", true) => &[Long, LongLong],
        ("l", false) => &[Long, UnsignedLong, LongLong, UnsignedLongLong],
        ("ul" | "lu", _) => &[UnsignedLong, UnsignedLongLong],
        ("ll", true) => &[LongLong],
        ("ll", false) => &[LongLong, UnsignedLongLong],
        ("ull" | "llu", _) => &[UnsignedLongLong],
        _ => return Err("invalid integer suffix".into()),
    };
    let kind = candidates
        .iter()
        .find(|kind| magnitude <= kind.maximum())
        .ok_or("integer literal does not fit a supported C type")?;
    IntegerValue::new(types, types.integer(*kind), magnitude)
}
