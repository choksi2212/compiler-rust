use compiler::{
    typed_front,
    typed_ir::{self, Instruction, ValueId},
    typed_lower,
    types::IntegerKind,
};
#[test]
fn aggregate_member_bitfield_and_temporary_identity_contracts_are_verified() {
    let source = "struct S{int x;};struct B{volatile unsigned int n:3;};struct B g;int main(){struct S a;struct S b;a.x=42;g.n=3;int n=g.n;return (b=a).x+n;}";
    let unit = typed_front::parse(source).unwrap();
    let module = typed_lower::lower(&unit).unwrap();
    typed_ir::verify(&module).unwrap();
    let mut count = 0;
    for (fi, f) in module.functions.iter().enumerate() {
        for (bi, b) in f.blocks.iter().enumerate() {
            for (ii, i) in b.instructions.iter().enumerate() {
                let replacement = match i {
                    Instruction::BitfieldRead {
                        dst, base, path, ..
                    } => Some(Instruction::BitfieldRead {
                        dst: *dst,
                        base: *base,
                        path: path.clone(),
                        volatile: false,
                    }),
                    Instruction::BitfieldWrite {
                        dst,
                        base,
                        src,
                        volatile,
                        ..
                    } => Some(Instruction::BitfieldWrite {
                        dst: *dst,
                        base: *base,
                        path: vec![99999],
                        src: *src,
                        volatile: *volatile,
                    }),
                    Instruction::MemberAddress { dst, base, .. } => {
                        Some(Instruction::MemberAddress {
                            dst: *dst,
                            base: *base,
                            path: vec![99999],
                        })
                    }
                    Instruction::AddressValue { dst, .. } => {
                        let scalar = f
                            .values
                            .iter()
                            .position(|ty| *ty == module.types.integer(IntegerKind::Int))
                            .unwrap();
                        Some(Instruction::AddressValue {
                            dst: *dst,
                            src: ValueId(scalar),
                        })
                    }
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    let mut bad = module.clone();
                    bad.functions[fi].blocks[bi].instructions[ii] = replacement;
                    assert!(typed_ir::verify(&bad).is_err(), "{i:?}");
                    count += 1;
                }
            }
        }
    }
    assert!(count >= 5);
}

#[test]
fn ordinary_whole_record_writes_reject_recursive_const_members() {
    let unit = typed_front::parse(
        "struct C{const int x;};struct C global;int main(){struct C a;struct C b=a;return 0;}",
    )
    .unwrap();
    let module = typed_lower::lower(&unit).unwrap();
    typed_ir::verify(&module).unwrap();
    let global = module
        .globals
        .iter()
        .find(|g| g.name == "global")
        .unwrap()
        .symbol;
    let mut checked = 0;
    for (fi, f) in module.functions.iter().enumerate() {
        for (bi, b) in f.blocks.iter().enumerate() {
            for (ii, instruction) in b.instructions.iter().enumerate() {
                if let Instruction::InitializeLocal { local, src } = instruction {
                    if !module.types.aggregate_value(f.values[src.0]).unwrap() {
                        continue;
                    }
                    let mut bad = module.clone();
                    bad.functions[fi].blocks[bi].instructions[ii] = Instruction::WriteLocal {
                        local: *local,
                        src: *src,
                    };
                    assert!(
                        typed_ir::verify(&bad).is_err(),
                        "const member local assignment accepted"
                    );
                    let mut bad = module.clone();
                    bad.functions[fi].blocks[bi].instructions[ii] = Instruction::WriteGlobal {
                        symbol: global,
                        src: *src,
                    };
                    assert!(
                        typed_ir::verify(&bad).is_err(),
                        "const member global assignment accepted"
                    );
                    let mut bad = module.clone();
                    let pointer = bad.types.pointer(f.locals[local.0].ty).unwrap();
                    let dst = ValueId(bad.functions[fi].values.len());
                    bad.functions[fi].values.push(pointer);
                    let align = bad.types.layout(f.locals[local.0].ty).unwrap().align;
                    bad.functions[fi].blocks[bi].instructions[ii] =
                        Instruction::AddressLocal { dst, local: *local };
                    bad.functions[fi].blocks[bi].instructions.insert(
                        ii + 1,
                        Instruction::Store {
                            address: dst,
                            src: *src,
                            align,
                            volatile: false,
                        },
                    );
                    assert!(
                        typed_ir::verify(&bad).is_err(),
                        "const member indirect assignment accepted"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 0);
}
