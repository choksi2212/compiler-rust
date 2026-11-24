use compiler::{
    integer::Binary,
    typed_ir::*,
    typed_opt,
    types::{IntegerKind::*, *},
};
fn program(kind: IntegerKind, op: Binary, a: u64, b: u64) -> Module {
    let mut types = TypeContext::new();
    let ty = types.integer(kind);
    let signature = types.function(ty, vec![], false, true).unwrap();
    Module {
        externs: vec![],
        data: vec![],
        entry: None,
        types,
        globals: vec![],
        functions: vec![Function {
            name: "fixture".into(),
            linkage: compiler::types::Linkage::Internal,
            id: FunctionId(0),
            symbol: SymbolId(0),
            signature,
            parameters: vec![],
            locals: vec![],
            values: vec![ty; 3],
            entry: BlockId(0),
            blocks: vec![Block {
                id: BlockId(0),
                instructions: vec![
                    Instruction::Constant {
                        dst: ValueId(0),
                        bits: a,
                    },
                    Instruction::Constant {
                        dst: ValueId(1),
                        bits: b,
                    },
                    Instruction::Binary {
                        dst: ValueId(2),
                        op,
                        lhs: ValueId(0),
                        rhs: ValueId(1),
                    },
                ],
                end: Terminator::Return(Some(ValueId(2))),
            }],
        }],
    }
}
#[test]
fn target_constants_wrap_unsigned_and_preserve_signed_faults() {
    let m = program(UnsignedLongLong, Binary::Add, u64::MAX, 1);
    let o = typed_opt::optimize(&m).unwrap();
    assert_eq!(
        o.functions[0].blocks[0].instructions[2],
        Instruction::Constant {
            dst: ValueId(2),
            bits: 0
        }
    );
    let m = program(LongLong, Binary::Add, i64::MAX as u64, 1);
    let o = typed_opt::optimize(&m).unwrap();
    assert_eq!(
        o.functions[0].blocks[0].instructions[2],
        m.functions[0].blocks[0].instructions[2]
    );
    let m = program(UnsignedLongLong, Binary::Divide, 7, 0);
    let o = typed_opt::optimize(&m).unwrap();
    assert!(matches!(
        o.functions[0].blocks[0].instructions[2],
        Instruction::Binary { .. }
    ));
}
#[test]
fn branch_simplification_removes_only_unreachable_blocks() {
    let mut m = program(Int, Binary::Add, 1, 2);
    let f = &mut m.functions[0];
    f.blocks[0].end = Terminator::Branch {
        condition: ValueId(2),
        yes: BlockId(1),
        no: BlockId(2),
    };
    for i in [1, 2] {
        f.blocks.push(Block {
            id: BlockId(i),
            instructions: vec![],
            end: Terminator::Return(Some(ValueId(2))),
        });
    }
    let o = typed_opt::optimize(&m).unwrap();
    assert_eq!(o.functions[0].blocks.len(), 2);
    assert_eq!(o.functions[0].blocks[0].end, Terminator::Jump(BlockId(1)));
    assert_eq!(m.functions[0].blocks.len(), 3);
}
#[test]
fn writes_initialization_and_volatile_reads_survive_folding() {
    let mut m = program(Int, Binary::Add, 1, 2);
    let int = m.types.integer(Int);
    let volatile = m
        .types
        .qualify(
            int,
            Qualifiers {
                volatile: true,
                ..Default::default()
            },
        )
        .unwrap();
    let f = &mut m.functions[0];
    f.locals.push(Local {
        alignment: None,
        symbol: SymbolId(1),
        ty: volatile,
    });
    f.blocks[0].instructions.extend([
        Instruction::ResetLocal(LocalId(0)),
        Instruction::InitializeLocal {
            local: LocalId(0),
            src: ValueId(2),
        },
        Instruction::ReadLocal {
            dst: ValueId(2),
            local: LocalId(0),
        },
        Instruction::WriteLocal {
            local: LocalId(0),
            src: ValueId(2),
        },
    ]);
    let o = typed_opt::optimize(&m).unwrap();
    assert_eq!(
        &o.functions[0].blocks[0].instructions[3..],
        &m.functions[0].blocks[0].instructions[3..]
    );
}
