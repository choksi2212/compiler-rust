use compiler::{
    aggregate::{FieldSpec, RecordKind},
    types::{IntegerKind::*, Layout, TypeContext},
};
fn field(name: &str, ty: compiler::types::TypeId) -> FieldSpec {
    FieldSpec {
        name: Some(name.into()),
        ty,
        bits: None,
        alignment: None,
        anonymous: false,
    }
}
#[test]
fn record_padding_union_overlap_nested_and_explicit_alignment() {
    let mut t = TypeContext::new();
    let char_ = t.integer(Char);
    let int = t.integer(Int);
    let short = t.integer(Short);
    let wide = t.integer(LongLong);
    let record = t.new_record(RecordKind::Struct, Some("S".into())).unwrap();
    t.complete_record(
        record,
        vec![field("a", char_), field("b", int), field("c", short)],
    )
    .unwrap();
    assert_eq!(t.layout(record).unwrap(), Layout { size: 12, align: 4 });
    assert_eq!(t.member(record, "b").unwrap().offset, 4);
    assert_eq!(t.member(record, "c").unwrap().offset, 8);
    let union = t.new_record(RecordKind::Union, None).unwrap();
    let bytes = t.array(char_, Some(3)).unwrap();
    t.complete_record(union, vec![field("wide", wide), field("bytes", bytes)])
        .unwrap();
    assert_eq!(t.layout(union).unwrap(), Layout { size: 8, align: 8 });
    assert_eq!(t.member(union, "bytes").unwrap().offset, 0);
    let nested = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(
        nested,
        vec![field("prefix", char_), field("object", record)],
    )
    .unwrap();
    assert_eq!(t.layout(nested).unwrap(), Layout { size: 16, align: 4 });
    assert_eq!(t.member(nested, "object").unwrap().offset, 4);
    let aligned = t.new_record(RecordKind::Struct, None).unwrap();
    let mut special = field("value", int);
    special.alignment = Some(16);
    t.complete_record(
        aligned,
        vec![field("prefix", char_), special, field("last", char_)],
    )
    .unwrap();
    assert_eq!(
        t.layout(aligned).unwrap(),
        Layout {
            size: 32,
            align: 16
        }
    );
    assert_eq!(t.member(aligned, "value").unwrap().offset, 16);
}
#[test]
fn bitfield_units_widths_separators_and_signedness_ledger() {
    let mut t = TypeContext::new();
    let uint = t.integer(UnsignedInt);
    let char_ = t.integer(UnsignedChar);
    let int = t.integer(Int);
    let mut specs = vec![];
    for (name, width) in [("a", 3), ("b", 5), ("c", 30), ("d", 4)] {
        let mut f = field(name, uint);
        f.bits = Some(width);
        specs.push(f);
    }
    let r = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(r, specs).unwrap();
    assert_eq!(t.layout(r).unwrap(), Layout { size: 12, align: 4 });
    for (name, offset, shift, width) in [
        ("a", 0, 0, 3),
        ("b", 0, 3, 5),
        ("c", 4, 0, 30),
        ("d", 8, 0, 4),
    ] {
        let f = t.member(r, name).unwrap();
        assert_eq!(f.offset, offset);
        let bits = f.bits.unwrap();
        assert_eq!((bits.shift, bits.width), (shift, width));
    }
    let r = t.new_record(RecordKind::Struct, None).unwrap();
    let mut a = field("a", char_);
    a.bits = Some(4);
    let separator = FieldSpec {
        name: None,
        ty: int,
        bits: Some(0),
        alignment: None,
        anonymous: false,
    };
    t.complete_record(r, vec![a, separator, field("last", char_)])
        .unwrap();
    assert_eq!(t.member(r, "last").unwrap().offset, 4);
    assert_eq!(t.layout(r).unwrap(), Layout { size: 8, align: 4 });
    let r = t.new_record(RecordKind::Struct, None).unwrap();
    let mut bad = field("x", uint);
    bad.bits = Some(33);
    assert!(t.complete_record(r, vec![bad]).is_err());
    assert!(t.layout(r).is_err());
    t.complete_record(r, vec![field("valid", uint)]).unwrap();
}
#[test]
fn nominal_incomplete_recursive_anonymous_and_flexible_constraints() {
    let mut t = TypeContext::new();
    let int = t.integer(Int);
    let node = t
        .new_record(RecordKind::Struct, Some("Node".into()))
        .unwrap();
    assert!(t.layout(node).is_err());
    assert!(t.complete_record(node, vec![field("self", node)]).is_err());
    let pointer = t.pointer(node).unwrap();
    t.complete_record(node, vec![field("value", int), field("next", pointer)])
        .unwrap();
    assert_eq!(t.layout(node).unwrap(), Layout { size: 16, align: 8 });
    let other = t
        .new_record(RecordKind::Struct, Some("Node".into()))
        .unwrap();
    t.complete_record(other, vec![field("value", int), field("next", pointer)])
        .unwrap();
    assert!(!t.compatible(node, other).unwrap());
    assert!(t.complete_record(node, vec![field("again", int)]).is_err());
    let inner = t.new_record(RecordKind::Union, None).unwrap();
    t.complete_record(inner, vec![field("x", int), field("y", int)])
        .unwrap();
    let outer = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(
        outer,
        vec![
            field("prefix", int),
            FieldSpec {
                name: None,
                ty: inner,
                bits: None,
                alignment: None,
                anonymous: true,
            },
        ],
    )
    .unwrap();
    assert_eq!(t.member(outer, "y").unwrap().offset, 4);
    assert_eq!(t.member(outer, "y").unwrap().path, vec![1, 1]);
    let tail = t.array(int, None).unwrap();
    let flex = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(flex, vec![field("count", int), field("data", tail)])
        .unwrap();
    assert_eq!(t.layout(flex).unwrap(), Layout { size: 4, align: 4 });
    assert!(t.array(flex, Some(2)).is_err());
    let bad = t.new_record(RecordKind::Struct, None).unwrap();
    assert!(t.complete_record(bad, vec![field("nested", flex)]).is_err());
    assert!(t.complete_record(bad, vec![field("data", tail)]).is_err());
    let ambiguous = t.new_record(RecordKind::Struct, None).unwrap();
    assert!(
        t.complete_record(
            ambiguous,
            vec![
                field("x", int),
                FieldSpec {
                    name: None,
                    ty: inner,
                    bits: None,
                    alignment: None,
                    anonymous: true
                }
            ]
        )
        .is_err()
    );
}

#[test]
fn frontend_records_tag_scopes_bitfields_and_anonymous_members() {
    let unit=compiler::typed_front::parse("struct Node;struct Node{int value;struct Node*next;};typedef struct Node Node;union Value{int i;long long wide;};struct Outer{char prefix;union{int x;int y;};unsigned int a:3,b:5;};int main(){struct Node local;{struct Node{char byte;};struct Node inner;}return sizeof(Node);}").unwrap();
    let declaration = unit
        .declarations
        .iter()
        .find(|d| d.token.text == "local")
        .unwrap();
    assert_eq!(
        unit.types.layout(declaration.ty).unwrap(),
        Layout { size: 16, align: 8 }
    );
    let inner = unit
        .declarations
        .iter()
        .find(|d| d.token.text == "inner")
        .unwrap();
    assert_eq!(
        unit.types.layout(inner.ty).unwrap(),
        Layout { size: 1, align: 1 }
    );
    assert!(
        unit.types
            .compatible(declaration.ty, inner.ty)
            .is_ok_and(|same| !same)
    );
}
#[test]
fn invalid_record_declarations_fail_without_completing_bad_types() {
    for source in [
        "struct R{struct R recursive;};",
        "struct R{int a;int a;};",
        "struct R{int a:33;};",
        "struct R{int x:0;};",
        "struct R{int a[];};",
        "union R{int n;int a[];};",
        "struct R{int n;int a[];int after;};",
        "struct R{static int x;};",
        "struct R{int f(void);};",
        "struct R{int a;};union R*p;",
        "struct R{int a;};struct R{char b;};",
        "struct R{int x;union{int x;char y;};};",
    ] {
        assert!(compiler::typed_front::parse(source).is_err(), "{source}");
    }
}

#[test]
fn aggregate_layout_failures_are_transactional_and_depth_is_bounded() {
    let mut t = TypeContext::new();
    let int = t.integer(Int);
    let byte = t.integer(Char);
    let bad = t.new_record(RecordKind::Struct, None).unwrap();
    let mut weak = field("x", int);
    weak.alignment = Some(2);
    assert!(t.complete_record(bad, vec![weak]).is_err());
    assert!(t.layout(bad).is_err());
    let mut huge = field("x", int);
    huge.alignment = Some(32);
    assert!(t.complete_record(bad, vec![huge]).is_err());
    t.complete_record(bad, vec![field("valid", int)]).unwrap();
    assert_eq!(t.layout(bad).unwrap(), Layout { size: 4, align: 4 });
    let mut nested = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(nested, vec![field("value", byte)])
        .unwrap();
    for _ in 1..128 {
        let next = t.new_record(RecordKind::Struct, None).unwrap();
        t.complete_record(next, vec![field("child", nested)])
            .unwrap();
        nested = next;
    }
    let too_deep = t.new_record(RecordKind::Struct, None).unwrap();
    assert!(
        t.complete_record(too_deep, vec![field("child", nested)])
            .unwrap_err()
            .contains("nesting")
    );
    assert!(t.layout(too_deep).is_err());
}
#[test]
fn anonymous_member_qualification_and_identity_paths_are_preserved() {
    let mut t = TypeContext::new();
    let int = t.integer(Int);
    let q = compiler::types::Qualifiers {
        constant: true,
        volatile: true,
        ..Default::default()
    };
    let inner = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(inner, vec![field("value", int)]).unwrap();
    let qualified = t.qualify(inner, q).unwrap();
    let outer = t.new_record(RecordKind::Struct, None).unwrap();
    t.complete_record(
        outer,
        vec![FieldSpec {
            name: None,
            ty: qualified,
            bits: None,
            alignment: None,
            anonymous: true,
        }],
    )
    .unwrap();
    let member = t.member(outer, "value").unwrap();
    let qualified_member = t.member_type(outer, &member.path).unwrap();
    assert_eq!(t.qualifiers(qualified_member).unwrap(), q);
    assert!(t.member_path(outer, &[]).is_err());
    assert!(t.member_path(outer, &[99]).is_err());
    assert!(t.contains_const(outer).unwrap());
}
