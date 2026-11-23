//! Target arithmetic shared by typed constant evaluation and verified folding.
use crate::types::{IntegerKind, IntegerValue, TypeContext, TypeId, mask};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Plus,
    Negate,
    Complement,
    Not,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
impl Binary {
    pub fn comparison(self) -> bool {
        matches!(
            self,
            Self::Equal
                | Self::NotEqual
                | Self::Less
                | Self::LessEqual
                | Self::Greater
                | Self::GreaterEqual
        )
    }
    pub fn shift(self) -> bool {
        matches!(self, Self::ShiftLeft | Self::ShiftRight)
    }
}
fn promoted(types: &TypeContext, ty: TypeId) -> Result<(), String> {
    if types.promoted(ty)? != ty {
        return Err("typed arithmetic requires explicit integer promotion".into());
    }
    Ok(())
}
fn signed_result(types: &TypeContext, ty: TypeId, value: i128) -> Result<IntegerValue, String> {
    let kind = types.integer_kind(ty)?;
    let minimum = -(1i128 << (kind.bits() - 1));
    let maximum = (1i128 << (kind.bits() - 1)) - 1;
    if value < minimum || value > maximum {
        return Err("signed integer overflow".into());
    }
    IntegerValue::new(types, ty, value as u64)
}
pub fn unary(types: &TypeContext, op: Unary, value: IntegerValue) -> Result<IntegerValue, String> {
    value.validate(types)?;
    if op == Unary::Not {
        return IntegerValue::new(
            types,
            types.integer(IntegerKind::Int),
            u64::from(value.bits == 0),
        );
    }
    promoted(types, value.ty)?;
    match op {
        Unary::Plus => Ok(value),
        Unary::Complement => IntegerValue::new(types, value.ty, !value.bits),
        Unary::Negate if types.integer_kind(value.ty)?.signed() => {
            signed_result(types, value.ty, -value.signed_value(types)?)
        }
        Unary::Negate => IntegerValue::new(types, value.ty, 0u64.wrapping_sub(value.bits)),
        Unary::Not => unreachable!(),
    }
}
pub fn binary(
    types: &TypeContext,
    op: Binary,
    a: IntegerValue,
    b: IntegerValue,
) -> Result<IntegerValue, String> {
    a.validate(types)?;
    b.validate(types)?;
    promoted(types, a.ty)?;
    promoted(types, b.ty)?;
    if !op.shift() && a.ty != b.ty {
        return Err("typed binary operands require an explicit common conversion".into());
    }
    let kind = types.integer_kind(a.ty)?;
    let (signed_a, signed_b) = (a.signed_value(types)?, b.signed_value(types)?);
    if op.comparison() {
        let ordering = if kind.signed() {
            signed_a.cmp(&signed_b)
        } else {
            a.bits.cmp(&b.bits)
        };
        let answer = match op {
            Binary::Equal => ordering.is_eq(),
            Binary::NotEqual => !ordering.is_eq(),
            Binary::Less => ordering.is_lt(),
            Binary::LessEqual => !ordering.is_gt(),
            Binary::Greater => ordering.is_gt(),
            Binary::GreaterEqual => !ordering.is_lt(),
            _ => unreachable!(),
        };
        return IntegerValue::new(types, types.integer(IntegerKind::Int), u64::from(answer));
    }
    if op.shift() {
        if signed_b < 0 || signed_b >= i128::from(kind.bits()) {
            return Err("shift count outside promoted operand width".into());
        }
        let count = signed_b as u32;
        return if op == Binary::ShiftLeft {
            if kind.signed() {
                if signed_a < 0 {
                    return Err("left shift of a negative signed value".into());
                }
                signed_result(types, a.ty, signed_a << count)
            } else {
                IntegerValue::new(types, a.ty, (a.bits << count) & mask(kind.bits()))
            }
        } else if kind.signed() {
            IntegerValue::new(types, a.ty, (signed_a >> count) as u64)
        } else {
            IntegerValue::new(types, a.ty, a.bits >> count)
        };
    }
    if matches!(op, Binary::And | Binary::Or | Binary::Xor) {
        let result = match op {
            Binary::And => a.bits & b.bits,
            Binary::Or => a.bits | b.bits,
            _ => a.bits ^ b.bits,
        };
        return IntegerValue::new(types, a.ty, result);
    }
    if matches!(op, Binary::Divide | Binary::Remainder) && b.bits == 0 {
        return Err("division by zero".into());
    }
    if kind.signed() {
        let value = match op {
            Binary::Add => signed_a + signed_b,
            Binary::Subtract => signed_a - signed_b,
            Binary::Multiply => signed_a * signed_b,
            Binary::Divide | Binary::Remainder => {
                if signed_a == -(1i128 << (kind.bits() - 1)) && signed_b == -1 {
                    return Err("signed integer overflow".into());
                }
                if op == Binary::Divide {
                    signed_a / signed_b
                } else {
                    signed_a % signed_b
                }
            }
            _ => unreachable!(),
        };
        signed_result(types, a.ty, value)
    } else {
        let value = match op {
            Binary::Add => a.bits.wrapping_add(b.bits),
            Binary::Subtract => a.bits.wrapping_sub(b.bits),
            Binary::Multiply => a.bits.wrapping_mul(b.bits),
            Binary::Divide => a.bits / b.bits,
            Binary::Remainder => a.bits % b.bits,
            _ => unreachable!(),
        };
        IntegerValue::new(types, a.ty, value)
    }
}
