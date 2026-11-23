use crate::ir::{Ir, Value};
use crate::optimize::{binary, unary};
use std::collections::HashMap;
#[derive(Debug, Clone)]
pub enum Op {
    Reset(usize),
    Push(Value),
    Load(usize),
    Store(usize),
    Unary(String),
    Binary(String),
    Jump(usize),
    Zero(usize),
    Print(String, usize),
    Halt,
    Return,
}
pub fn generate(ir: &[Ir]) -> Result<Vec<Op>, String> {
    crate::verify::verify(ir)?;
    let mut labels = HashMap::new();
    let mut out = vec![];
    let mut fixes = vec![];
    for i in ir {
        match i {
            Ir::Reset(d) => out.push(Op::Reset(*d)),
            Ir::Const(d, v) => out.extend([Op::Push(v.clone()), Op::Store(*d)]),
            Ir::Copy(d, a) => out.extend([Op::Load(*a), Op::Store(*d)]),
            Ir::Unary(d, op, a) => out.extend([Op::Load(*a), Op::Unary(op.clone()), Op::Store(*d)]),
            Ir::Binary(d, op, a, b) => out.extend([
                Op::Load(*a),
                Op::Load(*b),
                Op::Binary(op.clone()),
                Op::Store(*d),
            ]),
            Ir::Label(l) => {
                labels.insert(*l, out.len());
            }
            Ir::Jump(l) => {
                fixes.push((out.len(), *l));
                out.push(Op::Jump(0));
            }
            Ir::Zero(a, l) => {
                out.push(Op::Load(*a));
                fixes.push((out.len(), *l));
                out.push(Op::Zero(0));
            }
            Ir::Print(f, args, d) => {
                for a in args {
                    out.push(Op::Load(*a));
                }
                out.push(Op::Print(f.clone(), args.len()));
                out.push(Op::Store(*d));
            }
            Ir::Return(a) => out.extend([Op::Load(*a), Op::Return]),
            Ir::Halt => out.push(Op::Halt),
        }
    }
    for (at, label) in fixes {
        let target = *labels.get(&label).ok_or("undefined IR label")?;
        match &mut out[at] {
            Op::Jump(t) | Op::Zero(t) => *t = target,
            _ => unreachable!(),
        }
    }
    Ok(out)
}
fn pop(s: &mut Vec<Value>) -> Result<Value, String> {
    s.pop().ok_or("bytecode stack underflow".into())
}
fn int(v: Value) -> Result<i32, String> {
    if let Value::Int(n) = v {
        Ok(n)
    } else {
        Err("bytecode expected integer".into())
    }
}
pub fn run(code: &[Op]) -> Result<String, String> {
    Ok(execute(code)?.output)
}
#[derive(Debug, PartialEq, Eq)]
pub struct Execution {
    pub output: String,
    pub exit_code: i32,
}
pub fn execute(code: &[Op]) -> Result<Execution, String> {
    let mut pc = 0;
    let mut stack = vec![];
    let mut vars = HashMap::new();
    let mut output = String::new();
    let mut steps = 0;
    loop {
        steps += 1;
        if steps > 10_000_000 {
            return Err("execution instruction limit exceeded".into());
        }
        let op = code
            .get(pc)
            .ok_or("bytecode instruction pointer out of bounds")?;
        pc += 1;
        match op {
            Op::Reset(d) => {
                vars.remove(d);
            }
            Op::Push(v) => stack.push(v.clone()),
            Op::Load(a) => stack.push(
                vars.get(a)
                    .cloned()
                    .ok_or("read of uninitialized variable")?,
            ),
            Op::Store(d) => {
                vars.insert(*d, pop(&mut stack)?);
            }
            Op::Unary(op) => {
                let a = pop(&mut stack)?;
                stack.push(unary(op, a)?);
            }
            Op::Binary(op) => {
                let b = int(pop(&mut stack)?)?;
                let a = int(pop(&mut stack)?)?;
                stack.push(Value::Int(binary(op, a, b)?));
            }
            Op::Jump(t) => pc = *t,
            Op::Zero(t) => {
                if int(pop(&mut stack)?)? == 0 {
                    pc = *t;
                }
            }
            Op::Print(f, n) => {
                let output_start = output.len();
                if stack.len() < *n {
                    return Err("bytecode print stack underflow".into());
                }
                let args = stack.split_off(stack.len() - n);
                let mut args = args.into_iter();
                let mut chars = f.split('\0').next().unwrap().chars();
                while let Some(c) = chars.next() {
                    if c != '%' {
                        output.push(c);
                        continue;
                    }
                    match (chars.next(), args.next()) {
                        (Some('d'), Some(Value::Int(n))) => output.push_str(&n.to_string()),
                        (Some('s'), Some(Value::Str(s))) => {
                            output.push_str(s.split('\0').next().unwrap())
                        }
                        _ => return Err("bytecode format mismatch".into()),
                    }
                }
                stack.push(Value::Int((output.len() - output_start) as i32));
                if args.next().is_some() {
                    return Err("unused bytecode print arguments".into());
                }
            }
            Op::Return => {
                return Ok(Execution {
                    output,
                    exit_code: int(pop(&mut stack)?)?,
                });
            }
            Op::Halt => {
                return Ok(Execution {
                    output,
                    exit_code: 0,
                });
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn execute(body: &str, opt: bool) -> Result<String, String> {
        let a = crate::parser::parse(crate::lexer::lex(&format!(
            "int main(){{{body}return 0;}}"
        ))?)?;
        crate::semantic::check(&a)?;
        let ir = crate::ir::lower(&a);
        run(&generate(&if opt {
            crate::optimize::optimize(&ir)
        } else {
            ir
        })?)
    }
    #[test]
    fn loops_scopes_and_optimization() {
        let s = "int sum=0;for(int i=0;i<5;i=i+1){sum=sum+i;}{int sum=99;printf(\"%d \",sum);}printf(\"%d\",sum);";
        for o in [false, true] {
            assert_eq!(execute(s, o).unwrap(), "99 10");
        }
    }
    #[test]
    fn short_circuit() {
        for o in [false, true] {
            assert_eq!(
                execute("printf(\"%d %d\",0&&(1/0),1||(1/0));", o).unwrap(),
                "0 1"
            );
        }
    }
    #[test]
    fn runtime_errors() {
        for o in [false, true] {
            assert!(execute("int x;printf(\"%d\",x);", o).is_err());
            assert!(execute("printf(\"%d\",1/0);", o).is_err());
        }
        assert!(run(&[Op::Store(0)]).is_err());
        assert!(run(&[Op::Jump(999)]).is_err());
    }
    #[test]
    fn strings_and_while() {
        assert_eq!(execute("int x=3;while(x>0){printf(\"*\");x=x-1;}printf(\"%s %d\",\"ok\",strlen(\"hello\"));",true).unwrap(),"***ok 5");
    }
}
