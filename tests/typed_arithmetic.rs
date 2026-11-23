use compiler::{
    integer::{self, Binary::*, Unary},
    types::{IntegerKind::*, *},
};
fn value(t: &TypeContext, kind: IntegerKind, bits: u64) -> IntegerValue {
    IntegerValue::new(t, t.integer(kind), bits).unwrap()
}
#[test]
fn target_widths_wrapping_division_and_signed_faults() {
    let t = TypeContext::new();
    for kind in [UnsignedInt, UnsignedLong, UnsignedLongLong] {
        let maximum = kind.maximum();
        let max = value(&t, kind, maximum);
        let one = value(&t, kind, 1);
        assert_eq!(integer::binary(&t, Add, max, one).unwrap().bits, 0);
        assert_eq!(
            integer::binary(&t, Subtract, value(&t, kind, 0), one)
                .unwrap()
                .bits,
            maximum
        );
        assert_eq!(integer::binary(&t, Multiply, max, max).unwrap().bits, 1);
        assert_eq!(
            integer::binary(&t, Divide, max, value(&t, kind, 2))
                .unwrap()
                .bits,
            maximum / 2
        );
        assert_eq!(
            integer::unary(&t, Unary::Negate, one).unwrap().bits,
            maximum
        );
    }
    for kind in [Int, Long, LongLong] {
        let max = value(&t, kind, kind.maximum());
        let one = value(&t, kind, 1);
        let min = value(&t, kind, 1u64 << (kind.bits() - 1));
        let minus_one = value(&t, kind, u64::MAX);
        assert!(integer::binary(&t, Add, max, one).is_err());
        assert!(integer::binary(&t, Subtract, min, one).is_err());
        assert!(integer::binary(&t, Divide, min, minus_one).is_err());
        assert!(integer::binary(&t, Remainder, min, minus_one).is_err());
        assert!(integer::unary(&t, Unary::Negate, min).is_err());
        assert!(integer::binary(&t, Divide, one, value(&t, kind, 0)).is_err());
        assert_eq!(
            integer::binary(
                &t,
                Divide,
                value(&t, kind, (-7i64) as u64),
                value(&t, kind, 3)
            )
            .unwrap()
            .signed_value(&t)
            .unwrap(),
            -2
        );
        assert_eq!(
            integer::binary(
                &t,
                Remainder,
                value(&t, kind, (-7i64) as u64),
                value(&t, kind, 3)
            )
            .unwrap()
            .signed_value(&t)
            .unwrap(),
            -1
        );
    }
}
#[test]
fn shifts_bitwise_and_typed_boundaries() {
    let t = TypeContext::new();
    for kind in [
        Int,
        UnsignedInt,
        Long,
        UnsignedLong,
        LongLong,
        UnsignedLongLong,
    ] {
        let a = value(&t, kind, 1);
        let count = value(&t, Int, (kind.bits() - 1) as u64);
        if kind.signed() {
            assert!(integer::binary(&t, ShiftLeft, a, count).is_err());
        } else {
            assert_eq!(
                integer::binary(&t, ShiftLeft, a, count).unwrap().bits,
                1u64 << (kind.bits() - 1)
            );
        }
        assert!(integer::binary(&t, ShiftLeft, a, value(&t, Int, kind.bits() as u64)).is_err());
        assert!(integer::binary(&t, ShiftRight, a, value(&t, Int, u64::MAX)).is_err());
        let minus = value(&t, kind, u64::MAX);
        let result = integer::binary(&t, ShiftRight, minus, value(&t, Int, 1))
            .unwrap()
            .bits;
        assert_eq!(
            result,
            if kind.signed() {
                mask(kind.bits())
            } else {
                mask(kind.bits()) >> 1
            }
        );
        assert_eq!(integer::binary(&t, Xor, minus, minus).unwrap().bits, 0);
        assert_eq!(
            integer::unary(&t, Unary::Complement, a).unwrap().bits,
            mask(kind.bits()) - 1
        );
    }
    assert!(integer::binary(&t, Add, value(&t, Int, 1), value(&t, UnsignedInt, 1)).is_err());
    assert!(integer::binary(&t, Add, value(&t, Short, 1), value(&t, Short, 1)).is_err());
}
#[test]
fn exhaustive_small_signed_comparison_and_arithmetic() {
    let t = TypeContext::new();
    for kind in [Int, LongLong] {
        for a in -32i64..=32 {
            for b in -32i64..=32 {
                let av = value(&t, kind, a as u64);
                let bv = value(&t, kind, b as u64);
                for (op, expected) in [
                    (Add, a + b),
                    (Subtract, a - b),
                    (Multiply, a * b),
                    (Equal, i64::from(a == b)),
                    (Less, i64::from(a < b)),
                    (GreaterEqual, i64::from(a >= b)),
                    (And, a & b),
                    (Or, a | b),
                    (Xor, a ^ b),
                ] {
                    let actual = integer::binary(&t, op, av, bv).unwrap();
                    assert_eq!(
                        actual.signed_value(&t).unwrap(),
                        i128::from(expected),
                        "{kind:?} {a} {op:?} {b}"
                    );
                }
            }
        }
    }
}
