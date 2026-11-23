use compiler::{
    lexer,
    typed_front::{self, DeclarationKind, Expression, StatementKind},
    types::{IntegerKind::*, *},
};
fn type_of(source: &str, name: &str) -> (TypeContext, TypeId) {
    let unit = typed_front::parse(source).unwrap();
    let ty = unit
        .declarations
        .iter()
        .find(|d| d.token.text == name)
        .unwrap()
        .ty;
    (unit.types, ty)
}
#[test]
fn declaration_specifiers_typedef_qualifiers_and_derived_precedence() {
    let (types, ty) = type_of(
        "typedef unsigned long long U; typedef const U CU; CU f(void);",
        "CU",
    );
    assert!(types.qualifiers(ty).unwrap().constant);
    assert_eq!(types.integer_kind(ty).unwrap(), UnsignedLongLong);
    for (source, expected) in [
        ("typedef int *P[3];", 24),
        ("typedef int (*P)[3];", 8),
        ("typedef int (*P[2])(void);", 16),
        ("typedef int P[3][4];", 48),
        ("typedef int P;typedef P P;", 4),
        ("typedef void *restrict P;", 8),
    ] {
        let (types, ty) = type_of(source, "P");
        assert_eq!(types.layout(ty).unwrap().size, expected, "{source}");
    }
    let (types, ty) = type_of("typedef int *const *P;", "P");
    let Type::Pointer(inner) = types.get(ty).unwrap() else {
        panic!()
    };
    assert!(types.qualifiers(*inner).unwrap().constant);
    let (types, ty) = type_of("int *f(int x[3],const short y,...);", "f");
    let Type::Function {
        result,
        parameters,
        variadic,
        prototype,
    } = types.get(ty).unwrap()
    else {
        panic!()
    };
    assert!(*variadic && *prototype);
    assert!(matches!(types.get(*result).unwrap(), Type::Pointer(_)));
    assert!(matches!(
        types.get(parameters[0]).unwrap(),
        Type::Pointer(_)
    ));
    assert_eq!(types.integer_kind(parameters[1]).unwrap(), Short);
}
#[test]
fn scoped_typedef_lookups_are_not_spelling_based() {
    let source = "typedef long long T;int main(void){T x=1;{int T=2;T++;}T y=3;T:goto end;end:return (int)sizeof(T);}";
    let unit = typed_front::parse(source).unwrap();
    assert_eq!(
        unit.declarations
            .iter()
            .filter(|d| d.token.text == "T")
            .count(),
        2
    );
    assert!(
        unit.declarations
            .iter()
            .any(|d| d.token.text == "T" && d.kind == DeclarationKind::Local)
    );
    assert!(typed_front::parse("typedef int T;int f(int T,T x);").is_err());
    assert!(typed_front::parse("typedef int T;int f(T x,T y);").is_ok());
    assert!(typed_front::parse("int f(int x,int x);").is_err());
    assert!(typed_front::parse("int f(register int x);").is_ok());
    assert!(typed_front::parse("int f(auto int x);").is_err());
}
#[test]
fn casts_promotions_common_types_and_sizeof_are_explicit() {
    let unit=typed_front::parse("int main(void){unsigned int u=1;long s=-1;unsigned long r=u+s;return (int)(sizeof(long)+sizeof(long long)+sizeof(int (*)[3])+_Alignof(short));}").unwrap();
    let body = unit.main().unwrap();
    let StatementKind::Block { items, .. } = &body.kind else {
        panic!()
    };
    let StatementKind::Declarations(declarations) = &items[2].kind else {
        panic!()
    };
    let expr = declarations[0].1.as_ref().unwrap();
    assert_eq!(unit.types.integer_kind(expr.ty).unwrap(), UnsignedLong);
    let StatementKind::Return(expr) = &items[3].kind else {
        panic!()
    };
    assert_eq!(typed_front::constant(&unit.types, expr).unwrap().bits, 22);
    let unit = typed_front::parse("int main(){int x=1;return (int)sizeof(x++);}").unwrap();
    let StatementKind::Block { items, .. } = &unit.main().unwrap().kind else {
        panic!()
    };
    let StatementKind::Return(e) = &items[1].kind else {
        panic!()
    };
    assert_eq!(typed_front::constant(&unit.types, e).unwrap().bits, 4);
    assert!(matches!(e.kind, Expression::Convert(_)));
    for source in [
        "int main(){int x=1;(int)x=3;}",
        "int main(){int x=1;((int)x)++;}",
        "int main(){return sizeof(auto int);}",
        "int main(){return sizeof(*(int*)0);}",
    ] {
        assert!(typed_front::parse(source).is_err(), "{source}");
    }
}
#[test]
fn wide_literals_and_typed_tokens_preserve_real_byte_spans() {
    let text = "/*世界*/\nunsigned long long x=18446744073709551615ULL; x<<=1;";
    let tokens = lexer::lex_typed(text).unwrap();
    for token in &tokens[..tokens.len() - 1] {
        assert_eq!(&text[token.byte_start..token.byte_end], token.text);
    }
    assert!(tokens.iter().any(|t| t.text == "<<="));
    assert!(lexer::lex_typed("#include <stdio.h>\n").is_err());
    assert!(lexer::lex("2147483648").is_err());
    assert!(lexer::lex_typed("2147483648").is_ok());
    let unit=typed_front::parse("int main(){unsigned long long x=18446744073709551615ULL;long long y=2147483648;return (int)sizeof(x);}").unwrap();
    assert_eq!(
        unit.types
            .integer_kind(
                unit.declarations
                    .iter()
                    .find(|d| d.token.text == "x")
                    .unwrap()
                    .ty
            )
            .unwrap(),
        UnsignedLongLong
    );
    assert!(typed_front::parse("int main(){return 18446744073709551615;}").is_err());
}
#[test]
fn invalid_declarations_and_control_contexts_fail_cleanly() {
    for source in [
        "unsigned signed int x;",
        "typedef long short T;",
        "typedef int int T;",
        "typedef void int T;",
        "typedef _Bool long T;",
        "typedef int A[0];",
        "typedef int A[];int main(){return sizeof(A);}",
        "typedef int F(void);int main(){return sizeof(F);}",
        "int f(int);int f(long);",
        "typedef int T;typedef long T;",
        "int main(){int x;int x;}",
        "int main(){const int x=1;x=2;}",
        "int main(){break;}",
        "int main(){continue;}",
        "int main(){goto missing;}",
        "int main(){a:;a:;}",
        "int main(){case 1:;}",
        "int main(){switch(0u){case -1:;case 4294967295u:;}}",
        "int main(){switch(1){case 1:;case 1:;}}",
        "int main(){switch(1){default:;default:;}}",
        "int main(){switch(1){int x=1;case x:;}}",
        "int main(){if(1)int x;}",
        "int main(){return sizeof(void);}",
        "int main(){return (int*)0;}",
        "int main(int x){return x;}",
        "int main(){auto int f(void);}",
    ] {
        assert!(typed_front::parse(source).is_err(), "{source}");
    }
    assert!(typed_front::parse(&format!("int main(){{{}return 0;}}", "a:".repeat(1000))).is_err());
    assert!(typed_front::parse(&format!("typedef int {}T;", "*".repeat(1000))).is_err());
}

#[test]
fn typed_frontend_mutations_and_random_tokens_do_not_panic() {
    use compiler::{native_runtime::RuntimeOptions, typed_image, typed_ir, typed_lower, typed_opt};
    let seeds = [
        "int g=2;int f(int n,...){__builtin_va_list p;__builtin_va_start(p,n);int x=__builtin_va_arg(p,int);__builtin_va_end(p);return x+g;}int main(){return f(1,3);}",
        "int printf(const char*,...);int main(){return printf(\"%d\",3);}",
        "typedef unsigned long long U;int main(void){U x=0xffffffffffffffffULL;for(int i=0;i<5;i++){x>>=1;}return (int)x;}",
        "typedef int A[3];int main(){int i=0;do{switch(i++){case 1:continue;default:break;}if(i>4)goto out;}while(i<8);out:return sizeof(A);}",
    ];
    let pipeline = |source: &str| -> Result<(), String> {
        let unit = typed_front::parse(source)?;
        if unit.main().is_none() {
            return Ok(());
        }
        let ir = typed_lower::lower(&unit)?;
        typed_ir::verify(&ir)?;
        let optimized = typed_opt::optimize(&ir)?;
        typed_image::build(&optimized, &RuntimeOptions::default()).map(|_| ())
    };
    for seed in seeds {
        for at in 0..seed.len() {
            for replacement in ["", ":", ";", "*", "++", "typedef", "}"] {
                let mut source = seed.to_string();
                source.replace_range(at..at + 1, replacement);
                assert!(
                    std::panic::catch_unwind(|| pipeline(&source)).is_ok(),
                    "{source}"
                );
            }
        }
    }
    let alphabet = b"int main unsigned typedef (){}[];:=?~^&|+-*/%0123456789 abc\n\"'";
    let mut state = 2212u64;
    for _ in 0..10000 {
        let mut source = String::new();
        for _ in 0..80 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push(alphabet[(state >> 32) as usize % alphabet.len()] as char);
        }
        assert!(
            std::panic::catch_unwind(|| pipeline(&source)).is_ok(),
            "{source}"
        );
    }
    assert!(typed_front::parse_tokens(vec![]).is_err());
    let mut tokens = lexer::lex_typed("int main(){}").unwrap();
    tokens.pop();
    assert!(typed_front::parse_tokens(tokens).is_err());
}

#[test]
fn combined_derivations_and_declaration_budgets_are_enforced() {
    let mut declarator = "T".to_string();
    for _ in 0..16 {
        declarator = format!("({}{declarator})", "*".repeat(32));
    }
    assert!(typed_front::parse(&format!("typedef int {declarator};")).is_err());
    let mut source = String::from("int main(){");
    for i in 0..65536 {
        source.push_str(&format!("int x{i};"));
    }
    source.push_str("return 0;}");
    assert!(
        typed_front::parse(&source)
            .unwrap_err()
            .contains("declaration budget")
    );
}
