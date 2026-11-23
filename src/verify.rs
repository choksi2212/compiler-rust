//! Structural/type verification for the legacy teaching IR, not definite initialization.
use crate::ir::{Ir, Value};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    String,
}
fn kind(v: &Value) -> Kind {
    match v {
        Value::Int(_) => Kind::Integer,
        Value::Str(_) => Kind::String,
    }
}
fn operand(slots: &HashMap<usize, Kind>, id: usize, expected: Kind) -> Result<(), String> {
    match slots.get(&id) {
        Some(k) if *k == expected => Ok(()),
        Some(_) => Err(format!("wrong operand type for slot {id}")),
        None => Err(format!("undefined slot {id}")),
    }
}
fn define(slots: &mut HashMap<usize, Kind>, id: usize, k: Kind) -> Result<(), String> {
    if slots.get(&id).is_some_and(|old| *old != k) {
        return Err(format!("slot {id} changes type"));
    }
    slots.insert(id, k);
    Ok(())
}
/// Reject malformed IR before optimization or backend entry. Reset declares an
/// integer slot but does not promise a runtime value; read checks remain in the VM.
pub fn verify(code: &[Ir]) -> Result<(), String> {
    if code.is_empty() {
        return Err("invalid IR: empty program".into());
    }
    if !matches!(code.last(), Some(Ir::Halt | Ir::Jump(_) | Ir::Return(_))) {
        return Err("invalid IR: program can fall off its final instruction".into());
    }
    let mut labels = HashSet::new();
    for (i, op) in code.iter().enumerate() {
        if let Ir::Label(l) = op
            && !labels.insert(*l)
        {
            return Err(format!(
                "invalid IR at instruction {i}: duplicate label {l}"
            ));
        }
    }
    let mut slots = HashMap::new();
    for (i, op) in code.iter().enumerate() {
        let result = (|| {
            match op {
                Ir::Reset(d) => define(&mut slots, *d, Kind::Integer)?,
                Ir::Const(d, v) => define(&mut slots, *d, kind(v))?,
                Ir::Copy(d, a) => {
                    let k = *slots.get(a).ok_or_else(|| format!("undefined slot {a}"))?;
                    define(&mut slots, *d, k)?;
                }
                Ir::Unary(d, op, a) => {
                    let k = match op.as_str() {
                        "strlen" => Kind::String,
                        "+" | "-" | "!" | "bool" => Kind::Integer,
                        _ => return Err(format!("unsupported unary operator {op}")),
                    };
                    operand(&slots, *a, k)?;
                    define(&mut slots, *d, Kind::Integer)?;
                }
                Ir::Binary(d, op, a, b) => {
                    if !["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!="]
                        .contains(&op.as_str())
                    {
                        return Err(format!("unsupported binary operator {op}"));
                    }
                    operand(&slots, *a, Kind::Integer)?;
                    operand(&slots, *b, Kind::Integer)?;
                    define(&mut slots, *d, Kind::Integer)?;
                }
                Ir::Jump(l) => {
                    if !labels.contains(l) {
                        return Err(format!("undefined label {l}"));
                    }
                }
                Ir::Zero(a, l) => {
                    operand(&slots, *a, Kind::Integer)?;
                    if !labels.contains(l) {
                        return Err(format!("undefined label {l}"));
                    }
                }
                Ir::Print(f, args, d) => {
                    let mut chars = f.chars();
                    let mut index = 0;
                    while let Some(c) = chars.next() {
                        if c == '%' {
                            let k = match chars.next() {
                                Some('d') => Kind::Integer,
                                Some('s') => Kind::String,
                                _ => return Err("invalid print conversion".into()),
                            };
                            let a = args.get(index).ok_or("print argument count mismatch")?;
                            operand(&slots, *a, k)?;
                            index += 1;
                        }
                    }
                    if index != args.len() {
                        return Err("print argument count mismatch".into());
                    }
                    define(&mut slots, *d, Kind::Integer)?;
                }
                Ir::Return(a) => operand(&slots, *a, Kind::Integer)?,
                Ir::Label(_) | Ir::Halt => (),
            }
            Ok(())
        })();
        result.map_err(|e: String| format!("invalid IR at instruction {i}: {e}"))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_programs() {
        let bad = vec![
            vec![],
            vec![Ir::Return(999)],
            vec![
                Ir::Const(0, Value::Str("wrong return type".into())),
                Ir::Return(0),
            ],
            vec![Ir::Label(0), Ir::Label(0), Ir::Halt],
            vec![Ir::Jump(12)],
            vec![Ir::Copy(0, 999), Ir::Halt],
            vec![Ir::Reset(0), Ir::Unary(1, "other".into(), 0), Ir::Halt],
            vec![Ir::Reset(0), Ir::Binary(1, "&&".into(), 0, 0), Ir::Halt],
            vec![
                Ir::Const(0, Value::Str("s".into())),
                Ir::Zero(0, 0),
                Ir::Label(0),
                Ir::Halt,
            ],
            vec![Ir::Print("%d".into(), vec![], 0), Ir::Halt],
            vec![
                Ir::Const(0, Value::Str("s".into())),
                Ir::Print("%d".into(), vec![0], 1),
                Ir::Halt,
            ],
            vec![Ir::Print("%x".into(), vec![], 0), Ir::Halt],
            vec![
                Ir::Const(0, Value::Int(1)),
                Ir::Const(0, Value::Str("s".into())),
                Ir::Halt,
            ],
            vec![Ir::Reset(0)],
        ];
        for c in bad {
            assert!(verify(&c).is_err(), "{c:?}");
            assert!(
                crate::bytecode::generate(&c).is_err(),
                "backend accepted {c:?}"
            );
        }
    }
    #[test]
    fn uninitialized_is_a_runtime_error() {
        let c = vec![Ir::Reset(0), Ir::Print("%d".into(), vec![0], 1), Ir::Halt];
        verify(&c).unwrap();
        assert!(
            crate::bytecode::run(&crate::bytecode::generate(&c).unwrap())
                .unwrap_err()
                .contains("uninitialized")
        );
    }
    #[test]
    fn demos_before_and_after_optimization() {
        for name in [
            "addition",
            "loop",
            "factorial",
            "pattern",
            "conditional",
            "strings",
        ] {
            let s = std::fs::read_to_string(format!("examples/{name}.c")).unwrap();
            let ast = crate::parser::parse(crate::lexer::lex(&s).unwrap()).unwrap();
            crate::semantic::check(&ast).unwrap();
            let c = crate::ir::lower(&ast);
            verify(&c).unwrap();
            verify(&crate::optimize::optimize(&c)).unwrap();
        }
    }
}
#[cfg(test)]
mod adversarial {
    use super::*;
    #[test]
    fn every_operand_and_format_is_checked() {
        let prefix = vec![
            Ir::Const(0, Value::Int(4)),
            Ir::Const(1, Value::Str("s".into())),
        ];
        let failures = vec![
            Ir::Unary(2, "strlen".into(), 0),
            Ir::Unary(2, "-".into(), 1),
            Ir::Unary(2, "!".into(), 99),
            Ir::Binary(2, "+".into(), 1, 0),
            Ir::Binary(2, "+".into(), 0, 99),
            Ir::Zero(0, 99),
            Ir::Print("%s".into(), vec![0], 2),
            Ir::Print("".into(), vec![0], 2),
            Ir::Print("%".into(), vec![], 2),
            Ir::Print("%d".into(), vec![99], 2),
        ];
        for op in failures {
            let mut c = prefix.clone();
            c.push(op);
            c.push(Ir::Halt);
            assert!(verify(&c).is_err(), "{c:?}");
        }
    }
    #[test]
    fn malformed_fragments_do_not_panic() {
        for id in [0, 1, usize::MAX] {
            for op in ["+", "invalid", "strlen", "bool"] {
                for instruction in [
                    Ir::Copy(id, 99),
                    Ir::Unary(id, op.into(), 99),
                    Ir::Binary(id, op.into(), 99, 0),
                    Ir::Jump(id),
                    Ir::Zero(99, id),
                    Ir::Print(op.into(), vec![99], id),
                ] {
                    let c = vec![instruction, Ir::Halt];
                    assert!(std::panic::catch_unwind(|| verify(&c)).is_ok());
                }
            }
        }
    }
    #[test]
    fn valid_branch_and_string_flow() {
        let c = vec![
            Ir::Reset(0),
            Ir::Const(0, Value::Int(1)),
            Ir::Zero(0, 0),
            Ir::Const(1, Value::Str("abc".into())),
            Ir::Unary(2, "strlen".into(), 1),
            Ir::Print("%s%d".into(), vec![1, 2], 3),
            Ir::Label(0),
            Ir::Halt,
        ];
        verify(&c).unwrap();
        assert_eq!(
            crate::bytecode::run(&crate::bytecode::generate(&c).unwrap()).unwrap(),
            "abc3"
        );
    }
}
