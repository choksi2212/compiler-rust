//! Conservative typed passes. Each pass is followed by the IR verifier.
use crate::{
    integer,
    typed_ir::{self, Instruction, Module, Terminator, ValueId},
    types::IntegerValue,
};
use std::collections::{HashMap, HashSet};

pub fn optimize(input: &Module) -> Result<Module, String> {
    typed_ir::verify(input)?;
    let mut output = input.clone();
    fold_blocks(&mut output)?;
    typed_ir::verify(&output)?;
    remove_unreachable(&mut output);
    typed_ir::verify(&output)?;
    Ok(output)
}
fn fold_blocks(module: &mut Module) -> Result<(), String> {
    let types = &module.types;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut known: HashMap<ValueId, IntegerValue> = HashMap::new();
            for instruction in &mut block.instructions {
                let folded = match instruction {
                    Instruction::Constant { dst, bits }
                        if types.integer_kind(function.values[dst.0]).is_ok() =>
                    {
                        Some(IntegerValue {
                            ty: function.values[dst.0],
                            bits: *bits,
                        })
                    }
                    Instruction::Convert { dst, src } => known
                        .get(src)
                        .and_then(|value| value.cast(types, function.values[dst.0]).ok()),
                    Instruction::Unary { op, src, .. } => known
                        .get(src)
                        .and_then(|value| integer::unary(types, *op, *value).ok()),
                    Instruction::Binary { op, lhs, rhs, .. } => {
                        match (known.get(lhs), known.get(rhs)) {
                            (Some(a), Some(b)) => integer::binary(types, *op, *a, *b).ok(),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(dst) = instruction.destination() {
                    known.remove(&dst);
                    if let Some(value) = folded {
                        value.validate(types)?;
                        *instruction = Instruction::Constant {
                            dst,
                            bits: value.bits,
                        };
                        known.insert(dst, value);
                    }
                }
            }
            let replacement = match &block.end {
                Terminator::Branch { condition, yes, no } => known
                    .get(condition)
                    .map(|value| Terminator::Jump(if value.bits != 0 { *yes } else { *no })),
                Terminator::Switch {
                    value,
                    cases,
                    default,
                } => known.get(value).map(|value| {
                    Terminator::Jump(
                        cases
                            .iter()
                            .find(|c| c.0 == value.bits)
                            .map_or(*default, |c| c.1),
                    )
                }),
                _ => None,
            };
            if let Some(replacement) = replacement {
                block.end = replacement;
            }
        }
    }
    Ok(())
}
fn remove_unreachable(module: &mut Module) {
    for function in &mut module.functions {
        let targets: HashMap<_, _> = function
            .blocks
            .iter()
            .map(|block| (block.id, block.end.targets()))
            .collect();
        let mut reachable = HashSet::new();
        let mut pending = vec![function.entry];
        while let Some(block) = pending.pop() {
            if reachable.insert(block) {
                pending.extend(targets[&block].iter().copied());
            }
        }
        function
            .blocks
            .retain(|block| reachable.contains(&block.id));
    }
}
