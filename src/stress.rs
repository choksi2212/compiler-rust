use crate::{bytecode, ir, lexer, optimize, parser, semantic};
fn execute(source: &str, opt: bool) -> Result<String, String> {
    let ast = parser::parse(lexer::lex(source)?)?;
    semantic::check(&ast)?;
    let code = ir::lower(&ast);
    bytecode::run(&bytecode::generate(&if opt {
        optimize::optimize(&code)
    } else {
        code
    })?)
}
fn body(s: &str, opt: bool) -> Result<String, String> {
    execute(&format!("int main(){{{s}return 0;}}"), opt)
}
#[test]
fn reject_invalid_c() {
    for s in [
        "int float=1;",
        "int switch=1;",
        "if(1) int x=2;",
        "for(int i=0;i<1;int x=1){}",
        "int printf=2;printf(\"hello\");",
        "int strlen=2;printf(\"%d\",strlen(\"hi\"));",
        "int main=2;",
    ] {
        assert!(
            body(s, true).is_err(),
            "accepted invalid or unsupported C: {s}"
        );
    }
}
#[test]
fn shadow_initializer_is_not_outer_value() {
    for opt in [false, true] {
        let r = body("int x=7;{int x=x;printf(\"%d\",x);}", opt);
        assert!(
            r.is_err(),
            "self initialization must not read outer x: {r:?}"
        );
    }
}
#[test]
fn loop_declarations_are_fresh() {
    for opt in [false, true] {
        let r = body(
            "for(int i=0;i<2;i=i+1){int x;if(i==0){x=9;}else{printf(\"%d\",x);}}",
            opt,
        );
        assert!(r.is_err(), "stale loop local: {r:?}");
    }
}
#[test]
fn decimal_literals_do_not_reinterpret_c_octal() {
    assert!(body("printf(\"%d\",010);", true).is_err());
}
#[test]
fn arithmetic_matrix() {
    for a in -12i32..=12 {
        for b in -12i32..=12 {
            for op in [
                "+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||",
            ] {
                if b == 0 && matches!(op, "/" | "%") {
                    continue;
                }
                let expected = match op {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    "%" => a % b,
                    "<" => (a < b) as i32,
                    "<=" => (a <= b) as i32,
                    ">" => (a > b) as i32,
                    ">=" => (a >= b) as i32,
                    "==" => (a == b) as i32,
                    "!=" => (a != b) as i32,
                    "&&" => (a != 0 && b != 0) as i32,
                    _ => (a != 0 || b != 0) as i32,
                };
                let s = format!("int a={a};int b={b};printf(\"%d\",a{op}b);");
                for opt in [false, true] {
                    assert_eq!(body(&s, opt).unwrap(), expected.to_string(), "{s}");
                }
            }
        }
    }
}
#[test]
fn mutations_never_panic() {
    let seed = "int main(){int x=2;for(int i=0;i<3;i=i+1){x=x+i;}printf(\"%d\",x);return 0;}";
    for at in 0..seed.len() {
        for replacement in ["", ";", "}", "\"", "@", "return"] {
            let mut s = seed.to_string();
            s.replace_range(at..at + 1, replacement);
            assert!(
                std::panic::catch_unwind(|| execute(&s, true)).is_ok(),
                "panic: {s}"
            );
        }
    }
}
#[test]
fn control_flow_and_errors() {
    for opt in [false, true] {
        for (s, expected) in [
            ("if(1) if(0) printf(\"bad\");else printf(\"ok\");", "ok"),
            ("int x=1;if(0){x=9;}else{x=4;}printf(\"%d\",x);", "4"),
            ("printf(\"%d\",0&&(1/0));", "0"),
            ("printf(\"%d\",1||(1/0));", "1"),
            ("int n=printf(\"abc\");printf(\"%d\",n);", "abc3"),
        ] {
            assert_eq!(body(s, opt).unwrap(), expected);
        }
        for s in [
            "printf(\"%d\",2147483647+1);",
            "printf(\"%d\",1/0);",
            "printf(\"%d\",1%0);",
            "int x;printf(\"%d\",x);",
            "printf(\"%d\",-(-2147483647-1));",
        ] {
            assert!(body(s, opt).is_err(), "{s}");
        }
    }
}
#[test]
fn huge_inputs_fail_cleanly() {
    for expression in [
        format!("{}1{}", "(".repeat(10000), ")".repeat(10000)),
        format!("{}1", "!".repeat(10000)),
        vec!["1"; 10000].join("+"),
    ] {
        assert!(body(&format!("int x={expression};"), true).is_err());
    }
    let blocks = format!("{}{}", "{".repeat(10000), "}".repeat(10000));
    assert!(body(&blocks, true).is_err());
}
#[test]
fn unsupported_compound_tokens() {
    for s in [
        "int a=3;int b=2;printf(\"%d\",a--b);",
        "int a=3;int b=2;printf(\"%d\",a++b);",
        "int a=1;a+=2;",
    ] {
        assert!(body(s, true).is_err(), "{s}");
    }
}
#[test]
fn deterministic_random_input() {
    let alphabet = b"intmain{}();=+-*/%0123456789 abcdef\n\"!&|<>";
    let mut seed = 2212u64;
    for _ in 0..10000 {
        let mut s = String::new();
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            s.push(alphabet[(seed >> 32) as usize % alphabet.len()] as char);
        }
        assert!(
            std::panic::catch_unwind(|| execute(&s, true)).is_ok(),
            "{s}"
        );
    }
}
