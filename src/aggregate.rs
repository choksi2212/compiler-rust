//! Nominal record identities and the frozen Windows-target object layout ledger.
use crate::types::{IntegerKind, Layout, Type, TypeContext, TypeId};
use std::collections::HashSet;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Struct,
    Union,
}
#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub name: Option<String>,
    pub ty: TypeId,
    pub bits: Option<u32>,
    pub alignment: Option<u32>,
    pub anonymous: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitField {
    pub width: u32,
    pub shift: u32,
    pub storage: TypeId,
}
#[derive(Debug, Clone)]
pub struct Field {
    pub name: Option<String>,
    pub ty: TypeId,
    pub offset: u64,
    pub bits: Option<BitField>,
    pub anonymous: bool,
}
#[derive(Debug, Clone)]
pub struct Record {
    pub kind: RecordKind,
    pub tag: Option<String>,
    pub fields: Option<Vec<Field>>,
    pub layout: Option<Layout>,
    pub flexible: bool,
    pub depth: u16,
}
#[derive(Debug, Clone)]
pub struct Member {
    pub path: Vec<usize>,
    pub offset: u64,
    pub ty: TypeId,
    pub bits: Option<BitField>,
}
fn aligned(value: u64, align: u32) -> Result<u64, String> {
    value
        .checked_add(u64::from(align) - 1)
        .map(|n| n & !(u64::from(align) - 1))
        .ok_or_else(|| "record alignment overflow".into())
}
impl TypeContext {
    pub fn new_record(&mut self, kind: RecordKind, tag: Option<String>) -> Result<TypeId, String> {
        if self.aggregates.len() >= 65535 {
            return Err("record identity budget exceeded".into());
        }
        let id = RecordId(self.aggregates.len());
        let ty = self.insert(Type::Record(id))?;
        self.aggregates.push(Record {
            kind,
            tag,
            fields: None,
            layout: None,
            flexible: false,
            depth: 1,
        });
        Ok(ty)
    }
    pub fn record(&self, id: RecordId) -> Result<&Record, String> {
        self.aggregates
            .get(id.0)
            .ok_or_else(|| "invalid record identity".into())
    }
    pub fn record_type(&self, ty: TypeId) -> Result<&Record, String> {
        let Type::Record(id) = self.get(self.unqualified(ty)?)? else {
            return Err("record type required".into());
        };
        self.record(*id)
    }
    pub fn contains_flexible(&self, ty: TypeId) -> Result<bool, String> {
        Ok(match self.get(self.unqualified(ty)?)? {
            Type::Record(id) => self.record(*id)?.flexible,
            Type::Array { element, .. } => self.contains_flexible(*element)?,
            _ => false,
        })
    }
    fn object_depth(&self, ty: TypeId) -> Result<u16, String> {
        Ok(match self.get(self.unqualified(ty)?)? {
            Type::Record(id) => self.record(*id)?.depth,
            Type::Array { element, .. } => self
                .object_depth(*element)?
                .checked_add(1)
                .ok_or("aggregate depth overflow")?,
            _ => 0,
        })
    }
    pub fn complete_record(&mut self, ty: TypeId, specs: Vec<FieldSpec>) -> Result<(), String> {
        let Type::Record(id) = *self.get(self.unqualified(ty)?)? else {
            return Err("record type required".into());
        };
        let record = self.record(id)?;
        if record.fields.is_some() {
            return Err("record is already complete".into());
        }
        let kind = record.kind;
        if specs.is_empty() || specs.len() > 4096 {
            return Err("record requires1..4096 member declarations".into());
        }
        let mut fields = vec![];
        let mut size = 0u64;
        let mut align = 1u32;
        let mut unit: Option<(u64, u64, u32)> = None;
        let mut flexible = false;
        let mut names = HashSet::new();
        let mut depth = 1;
        for (index, spec) in specs.iter().enumerate() {
            if let Some(name) = &spec.name
                && !names.insert(name.clone())
            {
                return Err("duplicate record member name".into());
            }
            if let Some(width) = spec.bits {
                if spec.anonymous || spec.alignment.is_some() {
                    return Err(
                        "bitfield cannot be anonymous aggregate or explicitly aligned".into(),
                    );
                }
                let integer = self.integer_kind(spec.ty)?;
                if width > integer.bits()
                    || (integer == IntegerKind::Bool && width > 1)
                    || (width == 0 && spec.name.is_some())
                {
                    return Err("invalid bitfield width".into());
                }
                let layout = self.layout(spec.ty)?;
                if width == 0 {
                    if unit.take().is_some() {
                        align = align.max(layout.align);
                        if kind == RecordKind::Struct {
                            size = aligned(size, layout.align)?;
                        }
                    }
                    fields.push(Field {
                        name: None,
                        ty: spec.ty,
                        offset: size,
                        bits: Some(BitField {
                            width: 0,
                            shift: 0,
                            storage: spec.ty,
                        }),
                        anonymous: false,
                    });
                    continue;
                }
                align = align.max(layout.align);
                let (offset, shift) = if kind == RecordKind::Union {
                    size = size.max(layout.size);
                    (0, 0)
                } else {
                    let prior = unit.filter(|(_, bytes, used)| {
                        *bytes == layout.size && *used + width <= integer.bits()
                    });
                    if let Some((offset, bytes, used)) = prior {
                        unit = Some((offset, bytes, used + width));
                        (offset, used)
                    } else {
                        let offset = aligned(size, layout.align)?;
                        size = offset
                            .checked_add(layout.size)
                            .ok_or("record size overflow")?;
                        unit = Some((offset, layout.size, width));
                        (offset, 0)
                    }
                };
                fields.push(Field {
                    name: spec.name.clone(),
                    ty: spec.ty,
                    offset,
                    bits: Some(BitField {
                        width,
                        shift,
                        storage: spec.ty,
                    }),
                    anonymous: false,
                });
                continue;
            }
            unit = None;
            let (layout, is_tail) = if let Type::Array {
                element,
                length: None,
            } = self.get(self.unqualified(spec.ty)?)?
            {
                if kind != RecordKind::Struct
                    || index + 1 != specs.len()
                    || spec.name.is_none()
                    || names.len() < 2
                {
                    return Err(
                        "flexible array must be final named struct member after named storage"
                            .into(),
                    );
                }
                let element = self.layout(*element)?;
                (
                    Layout {
                        size: 0,
                        align: element.align,
                    },
                    true,
                )
            } else {
                (self.layout(spec.ty)?, false)
            };
            if kind == RecordKind::Struct && self.contains_flexible(spec.ty)? {
                return Err("struct member contains flexible array tail".into());
            }
            flexible |= is_tail || self.contains_flexible(spec.ty)?;
            depth = depth.max(
                self.object_depth(spec.ty)?
                    .checked_add(1)
                    .ok_or("aggregate depth overflow")?,
            );
            if depth > 128 {
                return Err("aggregate containment nesting limit exceeded".into());
            }
            let member_align = spec.alignment.unwrap_or(layout.align);
            if !member_align.is_power_of_two() || member_align > 16 || member_align < layout.align {
                return Err("unsupported or weakening member alignment".into());
            }
            align = align.max(member_align);
            let offset = if kind == RecordKind::Struct {
                aligned(size, member_align)?
            } else {
                0
            };
            if kind == RecordKind::Struct {
                size = offset
                    .checked_add(layout.size)
                    .ok_or("record size overflow")?;
            } else {
                size = size.max(layout.size);
            }
            if spec.anonymous {
                if spec.name.is_some() {
                    return Err("anonymous member cannot have an explicit name".into());
                }
                let nested = self.record_type(spec.ty)?;
                let members = nested
                    .fields
                    .as_ref()
                    .ok_or("incomplete anonymous member")?;
                self.collect_member_names(members, &mut names)?;
            } else if spec.name.is_none() {
                return Err("unnamed object member requires an anonymous record".into());
            }
            fields.push(Field {
                name: spec.name.clone(),
                ty: spec.ty,
                offset,
                bits: None,
                anonymous: spec.anonymous,
            });
        }
        if names.is_empty() {
            return Err("record requires named member storage".into());
        }
        let layout = Layout {
            size: aligned(size, align)?,
            align,
        };
        let record = &mut self.aggregates[id.0];
        record.fields = Some(fields);
        record.layout = Some(layout);
        record.flexible = flexible;
        record.depth = depth;
        Ok(())
    }
    fn collect_member_names(
        &self,
        fields: &[Field],
        names: &mut HashSet<String>,
    ) -> Result<(), String> {
        for field in fields {
            if let Some(name) = &field.name {
                if !names.insert(name.clone()) {
                    return Err("ambiguous anonymous member name".into());
                }
            } else if field.anonymous {
                self.collect_member_names(
                    self.record_type(field.ty)?
                        .fields
                        .as_ref()
                        .ok_or("incomplete anonymous member")?,
                    names,
                )?;
            }
        }
        Ok(())
    }
    pub fn member(&self, ty: TypeId, name: &str) -> Result<Member, String> {
        let record = self.record_type(ty)?;
        let fields = record
            .fields
            .as_ref()
            .ok_or("member of incomplete record")?;
        for (index, field) in fields.iter().enumerate() {
            if field.name.as_deref() == Some(name) {
                return Ok(Member {
                    path: vec![index],
                    offset: field.offset,
                    ty: field.ty,
                    bits: field.bits,
                });
            }
            if field.anonymous
                && let Ok(mut found) = self.member(field.ty, name)
            {
                found.offset = found
                    .offset
                    .checked_add(field.offset)
                    .ok_or("member offset overflow")?;
                found.path.insert(0, index);
                return Ok(found);
            }
        }
        Err(format!("record has no member {name}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub usize);
#[derive(Debug, Clone)]
pub struct Enumeration {
    pub tag: Option<String>,
    pub values: Option<Vec<(String, i32)>>,
}
impl TypeContext {
    pub fn new_enum(&mut self, tag: Option<String>) -> Result<TypeId, String> {
        let id = EnumId(self.enumerations.len());
        let ty = self.insert(Type::Enum(id))?;
        self.enumerations.push(Enumeration { tag, values: None });
        Ok(ty)
    }
    pub fn enumeration(&self, id: EnumId) -> Result<&Enumeration, String> {
        self.enumerations
            .get(id.0)
            .ok_or_else(|| "invalid enum identity".into())
    }
    pub fn complete_enum(&mut self, ty: TypeId, values: Vec<(String, i32)>) -> Result<(), String> {
        let Type::Enum(id) = *self.get(self.unqualified(ty)?)? else {
            return Err("enum type required".into());
        };
        if self.enumeration(id)?.values.is_some() {
            return Err("enum already complete".into());
        }
        if values.is_empty() || values.len() > 4096 {
            return Err("enum requires1..4096 enumerators".into());
        }
        let mut names = HashSet::new();
        if values.iter().any(|(name, _)| !names.insert(name)) {
            return Err("duplicate enumerator".into());
        }
        self.enumerations[id.0].values = Some(values);
        Ok(())
    }
}

impl TypeContext {
    pub fn member_path(&self, object: TypeId, path: &[usize]) -> Result<Member, String> {
        if path.is_empty() || path.len() > 128 {
            return Err("invalid member path length".into());
        }
        let mut ty = object;
        let mut offset = 0u64;
        let mut bits = None;
        for index in path {
            let field = self
                .record_type(ty)?
                .fields
                .as_ref()
                .ok_or("member of incomplete record")?
                .get(*index)
                .ok_or("invalid member identity")?;
            offset = offset
                .checked_add(field.offset)
                .ok_or("member offset overflow")?;
            ty = field.ty;
            bits = field.bits;
        }
        Ok(Member {
            path: path.to_vec(),
            offset,
            ty,
            bits,
        })
    }
    pub fn member_type(&mut self, object: TypeId, path: &[usize]) -> Result<TypeId, String> {
        self.member_path(object, path)?;
        let mut ty = object;
        let mut q = self.qualifiers(object)?;
        for index in path {
            let field = self
                .record_type(ty)?
                .fields
                .as_ref()
                .ok_or("incomplete record")?[*index]
                .clone();
            let fq = self.qualifiers(field.ty)?;
            q.constant |= fq.constant;
            q.volatile |= fq.volatile;
            ty = field.ty;
        }
        self.qualify(ty, q)
    }
}

impl TypeContext {
    pub fn contains_const(&self, ty: TypeId) -> Result<bool, String> {
        if self.qualifiers(ty)?.constant {
            return Ok(true);
        }
        match self.get(self.unqualified(ty)?)? {
            Type::Record(id) => {
                for field in self
                    .record(*id)?
                    .fields
                    .as_ref()
                    .ok_or("incomplete record")?
                {
                    if self.contains_const(field.ty)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Type::Array { element, .. } => self.contains_const(*element),
            _ => Ok(false),
        }
    }
    pub fn aggregate_value(&self, ty: TypeId) -> Result<bool, String> {
        Ok(matches!(self.get(self.unqualified(ty)?)?, Type::Record(_)))
    }
    pub fn indirect_aggregate(&self, ty: TypeId) -> Result<bool, String> {
        Ok(self.aggregate_value(ty)? && !matches!(self.layout(ty)?.size, 1 | 2 | 4 | 8))
    }
}
