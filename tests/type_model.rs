use compiler::types::{IntegerKind::*, *};

#[test]
fn canonical_llp64_layout_and_qualifiers_are_target_owned() {
    let mut t = TypeContext::new();
    for (kind, size, signed, rank) in [
        (Bool, 1, false, 0),
        (Char, 1, true, 1),
        (SignedChar, 1, true, 1),
        (UnsignedChar, 1, false, 1),
        (Short, 2, true, 2),
        (UnsignedShort, 2, false, 2),
        (Int, 4, true, 3),
        (UnsignedInt, 4, false, 3),
        (Long, 4, true, 4),
        (UnsignedLong, 4, false, 4),
        (LongLong, 8, true, 5),
        (UnsignedLongLong, 8, false, 5),
    ] {
        let id = t.integer(kind);
        assert_eq!(
            t.layout(id).unwrap(),
            Layout {
                size,
                align: size as u32
            }
        );
        assert_eq!(kind.signed(), signed);
        assert_eq!(kind.rank(), rank);
        assert_eq!(t.integer_kind(id).unwrap(), kind);
    }
    assert_ne!(t.integer(Int), t.integer(Long));
    assert_ne!(t.integer(Char), t.integer(SignedChar));
    let int = t.integer(Int);
    let q = Qualifiers {
        constant: true,
        ..Default::default()
    };
    let ci = t.qualify(int, q).unwrap();
    assert_eq!(ci, t.qualify(ci, q).unwrap());
    let p = t.pointer(ci).unwrap();
    assert_eq!(p, t.pointer(ci).unwrap());
    assert_eq!(t.layout(p).unwrap(), Layout { size: 8, align: 8 });
    assert!(!t.compatible(int, ci).unwrap());
    let array = t.array(int, Some(3)).unwrap();
    let ca = t.qualify(array, q).unwrap();
    assert!(matches!(t.get(ca).unwrap(),Type::Array { element, length:Some(3) } if *element==ci));
    let restricted = Qualifiers {
        restrict: true,
        ..Default::default()
    };
    assert!(t.qualify(int, restricted).is_err());
    assert!(t.qualify(p, restricted).is_ok());
    let void_pointer = t.pointer(t.void()).unwrap();
    assert!(t.qualify(void_pointer, restricted).is_ok());
}

#[test]
fn incomplete_arrays_functions_and_compatibility_are_checked() {
    let mut t = TypeContext::new();
    let int = t.integer(Int);
    let array = t.array(int, Some(7)).unwrap();
    let incomplete = t.array(int, None).unwrap();
    assert!(t.compatible(array, incomplete).unwrap());
    let other_array = t.array(int, Some(8)).unwrap();
    assert!(!t.compatible(array, other_array).unwrap());
    assert!(t.layout(incomplete).is_err());
    assert!(t.layout(t.void()).is_err());
    assert!(t.array(int, Some(0)).is_err());
    assert!(t.array(t.void(), Some(2)).is_err());
    assert!(t.array(incomplete, Some(2)).is_err());
    assert!(t.array(t.integer(LongLong), Some(u64::MAX)).is_err());
    let f = t.function(int, vec![array], false, true).unwrap();
    let pointer = t.pointer(int).unwrap();
    let same = t.function(int, vec![pointer], false, true).unwrap();
    assert_eq!(f, same);
    assert!(t.layout(f).is_err());
    assert!(t.array(f, Some(1)).is_err());
    assert!(t.function(array, vec![], false, true).is_err());
    assert!(t.function(f, vec![], false, true).is_err());
    assert!(t.function(int, vec![t.void()], false, true).is_err());
    assert!(t.function(int, vec![], true, true).is_err());
    let no_proto = t.function(int, vec![], false, false).unwrap();
    let proto = t.function(int, vec![int], false, true).unwrap();
    assert!(t.compatible(no_proto, proto).unwrap());
    let narrow = t
        .function(int, vec![t.integer(Short)], false, true)
        .unwrap();
    assert!(!t.compatible(no_proto, narrow).unwrap());
    let variadic = t.function(int, vec![int], true, true).unwrap();
    assert!(!t.compatible(no_proto, variadic).unwrap());
    let callback = t.pointer(f).unwrap();
    assert!(
        t.qualify(
            callback,
            Qualifiers {
                restrict: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(t.get(TypeId(usize::MAX)).is_err());
    assert!(t.pointer(TypeId(usize::MAX)).is_err());
    assert!(t.compatible(int, TypeId(usize::MAX)).is_err());
    let mut nested = int;
    for _ in 0..128 {
        nested = t.pointer(nested).unwrap();
    }
    assert!(t.pointer(nested).is_err());
}

#[test]
fn promotion_and_common_arithmetic_rank_matrix() {
    let t = TypeContext::new();
    for kind in [Bool, Char, SignedChar, UnsignedChar, Short, UnsignedShort] {
        assert_eq!(t.promoted(t.integer(kind)).unwrap(), t.integer(Int));
    }
    for (a, b, expected) in [
        (Int, UnsignedInt, UnsignedInt),
        (Long, UnsignedInt, UnsignedLong),
        (LongLong, UnsignedLong, LongLong),
        (UnsignedLongLong, LongLong, UnsignedLongLong),
        (UnsignedShort, Short, Int),
        (Char, Bool, Int),
        (Long, Int, Long),
        (Long, LongLong, LongLong),
        (UnsignedLong, Int, UnsignedLong),
    ] {
        for (a, b) in [(a, b), (b, a)] {
            assert_eq!(
                t.common_integer(t.integer(a), t.integer(b)).unwrap(),
                t.integer(expected)
            );
        }
    }
    for a in IntegerKind::ALL {
        for b in IntegerKind::ALL {
            let ab = t.common_integer(t.integer(a), t.integer(b)).unwrap();
            let ba = t.common_integer(t.integer(b), t.integer(a)).unwrap();
            assert_eq!(ab, ba);
        }
    }
}

#[test]
fn literal_candidate_lists_distinguish_radix_suffix_and_rank() {
    let t = TypeContext::new();
    for (n, base, suffix, kind) in [
        (2147483647, 10, "", Int),
        (2147483648, 10, "", LongLong),
        (2147483648, 16, "", UnsignedInt),
        (u32::MAX as u64, 8, "", UnsignedInt),
        (u32::MAX as u64 + 1, 16, "", LongLong),
        (u64::MAX, 16, "", UnsignedLongLong),
        (1, 10, "l", Long),
        (1, 10, "ll", LongLong),
        (1, 10, "U", UnsignedInt),
        (1, 10, "UL", UnsignedLong),
        (u32::MAX as u64 + 1, 10, "UL", UnsignedLongLong),
        (u32::MAX as u64, 16, "L", UnsignedLong),
        (1, 8, "LLU", UnsignedLongLong),
    ] {
        let value = integer_literal(&t, n, base, suffix).unwrap();
        assert_eq!(value.ty, t.integer(kind), "{n} base{base} {suffix}");
        assert_eq!(value.bits, n);
    }
    assert!(integer_literal(&t, u64::MAX, 10, "").is_err());
    assert!(integer_literal(&t, u64::MAX, 10, "ll").is_err());
    assert!(integer_literal(&t, 1, 2, "").is_err());
    for suffix in ["uu", "lL", "lul", "xyz"] {
        assert!(integer_literal(&t, 1, 10, suffix).is_err());
    }
}

#[test]
fn exhaustive_byte_conversions_and_wide_boundary_patterns() {
    let t = TypeContext::new();
    for bits in 0..=255u64 {
        let source = IntegerValue::new(&t, t.integer(SignedChar), bits).unwrap();
        let signed = i128::from(bits as u8 as i8);
        assert_eq!(source.signed_value(&t).unwrap(), signed);
        for destination in IntegerKind::ALL {
            let converted = source.cast(&t, t.integer(destination)).unwrap();
            let expected = if destination == Bool {
                u64::from(bits != 0)
            } else {
                (signed as u64) & mask(destination.bits())
            };
            assert_eq!(converted.bits, expected, "{bits} to {destination:?}");
        }
    }
    for kind in IntegerKind::ALL {
        for bits in [
            0,
            1,
            127,
            128,
            255,
            256,
            32767,
            32768,
            u32::MAX as u64,
            1u64 << 32,
            1u64 << 63,
            u64::MAX,
        ] {
            let value = IntegerValue::new(&t, t.integer(kind), bits).unwrap();
            value.validate(&t).unwrap();
            for target in IntegerKind::ALL {
                value
                    .cast(&t, t.integer(target))
                    .unwrap()
                    .validate(&t)
                    .unwrap();
            }
        }
    }
    assert!(
        IntegerValue {
            ty: t.integer(Bool),
            bits: 2
        }
        .validate(&t)
        .is_err()
    );
    assert!(
        IntegerValue {
            ty: t.integer(Int),
            bits: u64::MAX
        }
        .cast(&t, t.integer(LongLong))
        .is_err()
    );
}
