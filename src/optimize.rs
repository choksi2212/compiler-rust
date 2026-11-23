use crate::ir::{Ir, Value};
use std::collections::HashMap;
pub fn binary(op: &str, a: i32, b: i32) -> Result<i32, String> {
    match op {
        "+" => a.checked_add(b),
        "-" => a.checked_sub(b),
        "*" => a.checked_mul(b),
        "/" => a.checked_div(b),
        "%" => a.checked_rem(b),
        "<" => Some((a < b) as i32),
        "<=" => Some((a <= b) as i32),
        ">" => Some((a > b) as i32),
        ">=" => Some((a >= b) as i32),
        "==" => Some((a == b) as i32),
        "!=" => Some((a != b) as i32),
        _ => return Err("invalid binary instruction".into()),
    }
    .ok_or_else(|| {
        if b == 0 && matches!(op, "/" | "%") {
            "division by zero".into()
        } else {
            "integer overflow".into()
        }
    })
}
pub fn unary(op: &str, v: Value) -> Result<Value, String> {
    match (op, v) {
        ("-", Value::Int(n)) => n
            .checked_neg()
            .map(Value::Int)
            .ok_or("integer overflow".into()),
        ("!", Value::Int(n)) => Ok(Value::Int((n == 0) as i32)),
        ("bool", Value::Int(n)) => Ok(Value::Int((n != 0) as i32)),
        ("strlen", Value::Str(s)) => Ok(Value::Int(s.len() as i32)),
        _ => Err("invalid unary instruction".into()),
    }
}
pub fn optimize(code: &[Ir]) -> Vec<Ir> {
    let mut known: HashMap<usize, Value> = HashMap::new();
    let mut out = vec![];
    let mut dead = false;
    for i in code {
        if matches!(i, Ir::Label(_)) {
            known.clear();
            dead = false;
        }
        if dead {
            continue;
        }
        let replacement = match i {
            Ir::Copy(d, a) => known.get(a).cloned().map(|v| Ir::Const(*d, v)),
            Ir::Unary(d, op, a) => known
                .get(a)
                .cloned()
                .and_then(|v| unary(op, v).ok())
                .map(|v| Ir::Const(*d, v)),
            Ir::Binary(d, op, a, b) => match (known.get(a), known.get(b)) {
                (Some(Value::Int(a)), Some(Value::Int(b))) => binary(op, *a, *b)
                    .ok()
                    .map(|n| Ir::Const(*d, Value::Int(n))),
                _ => None,
            },
            _ => None,
        };
        let i = replacement.unwrap_or_else(|| i.clone());
        match &i {
            Ir::Const(d, v) => {
                known.insert(*d, v.clone());
            }
            Ir::Reset(d)
            | Ir::Print(_, _, d)
            | Ir::Copy(d, _)
            | Ir::Unary(d, ..)
            | Ir::Binary(d, ..) => {
                known.remove(d);
            }
            Ir::Jump(_) | Ir::Zero(..) | Ir::Halt => {
                known.clear();
            }
            _ => (),
        };
        if matches!(i, Ir::Jump(_) | Ir::Halt) {
            dead = true;
        }
        out.push(i);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folding() {
        let v = optimize(&[
            Ir::Const(0, Value::Int(2)),
            Ir::Const(1, Value::Int(3)),
            Ir::Binary(2, "+".into(), 0, 1),
            Ir::Halt,
            Ir::Const(3, Value::Int(9)),
        ]);
        assert_eq!(v.len(), 4);
        assert!(matches!(v[2], Ir::Const(2, Value::Int(5))));
    }
    #[test]
    fn preserves_errors_and_boundaries() {
        assert!(binary("/", 1, 0).is_err());
        let v = optimize(&[Ir::Const(0, Value::Int(1)), Ir::Label(0), Ir::Copy(1, 0)]);
        assert!(matches!(v[2], Ir::Copy(..)));
    }
}
