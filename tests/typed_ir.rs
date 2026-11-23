use compiler::{
    integer::Binary,
    typed_ir::*,
    types::{IntegerKind::*, *},
};
fn fixture() -> Module {
    let mut types = TypeContext::new();
    let int = types.integer(Int);
    let signature = types.function(int, vec![], false, true).unwrap();
    Module {
        types,
        globals: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            symbol: SymbolId(0),
            signature,
            parameters: vec![],
            locals: vec![Local {
                symbol: SymbolId(1),
                ty: int,
            }],
            values: vec![int; 3],
            entry: BlockId(0),
            blocks: vec![
                Block {
                    id: BlockId(0),
                    instructions: vec![Instruction::Constant {
                        dst: ValueId(0),
                        bits: 1,
                    }],
                    end: Terminator::Branch {
                        condition: ValueId(0),
                        yes: BlockId(1),
                        no: BlockId(2),
                    },
                },
                Block {
                    id: BlockId(1),
                    instructions: vec![Instruction::Constant {
                        dst: ValueId(1),
                        bits: 7,
                    }],
                    end: Terminator::Jump(BlockId(3)),
                },
                Block {
                    id: BlockId(2),
                    instructions: vec![Instruction::Constant {
                        dst: ValueId(1),
                        bits: 8,
                    }],
                    end: Terminator::Jump(BlockId(3)),
                },
                Block {
                    id: BlockId(3),
                    instructions: vec![Instruction::Binary {
                        dst: ValueId(2),
                        op: Binary::Add,
                        lhs: ValueId(0),
                        rhs: ValueId(1),
                    }],
                    end: Terminator::Return(Some(ValueId(2))),
                },
            ],
        }],
    }
}
#[test]
fn non_ssa_merge_and_loop_must_definitions() {
    let module = fixture();
    verify(&module).unwrap();
    let mut bad = module.clone();
    bad.functions[0].blocks[2].instructions.clear();
    assert!(verify(&bad).unwrap_err().contains("before definition"));
    let mut looped = module.clone();
    looped.functions[0].blocks[3].end = Terminator::Branch {
        condition: ValueId(2),
        yes: BlockId(0),
        no: BlockId(4),
    };
    looped.functions[0].blocks.push(Block {
        id: BlockId(4),
        instructions: vec![],
        end: Terminator::Return(Some(ValueId(2))),
    });
    verify(&looped).unwrap();
    let mut bad = fixture();
    bad.functions[0].blocks[0].instructions = vec![Instruction::InitializeLocal {
        local: LocalId(0),
        src: ValueId(0),
    }];
    assert!(verify(&bad).unwrap_err().contains("before definition"));
}
#[test]
fn malformed_identities_types_and_terminators_are_rejected() {
    let original = fixture();
    for mutate in [
        |m: &mut Module| m.functions[0].entry = BlockId(999),
        |m: &mut Module| m.functions[0].blocks[1].id = BlockId(0),
        |m: &mut Module| m.functions[0].blocks[0].end = Terminator::Jump(BlockId(999)),
        |m: &mut Module| m.functions[0].blocks[3].end = Terminator::Return(None),
        |m: &mut Module| m.functions[0].blocks[3].end = Terminator::Return(Some(ValueId(999))),
        |m: &mut Module| m.functions[0].values[0] = TypeId(999),
        |m: &mut Module| {
            m.functions[0].blocks[0].instructions[0] = Instruction::Constant {
                dst: ValueId(0),
                bits: u64::MAX,
            }
        },
        |m: &mut Module| m.functions[0].locals[0].symbol = SymbolId(0),
        |m: &mut Module| {
            m.functions[0].blocks[3].instructions[0] = Instruction::Convert {
                dst: ValueId(999),
                src: ValueId(1),
            }
        },
        |m: &mut Module| {
            m.functions[0].blocks[3].instructions[0] = Instruction::ReadLocal {
                dst: ValueId(2),
                local: LocalId(999),
            }
        },
    ] {
        let mut bad = original.clone();
        mutate(&mut bad);
        assert!(verify(&bad).is_err());
    }
    let mut bad = original.clone();
    bad.functions.push(bad.functions[0].clone());
    assert!(verify(&bad).is_err());
    let mut bad = original.clone();
    bad.functions[0].values[1] = bad.types.integer(UnsignedInt);
    assert!(verify(&bad).is_err());
    let mut bad = original;
    bad.functions[0].values[0] = bad.types.integer(Short);
    assert!(verify(&bad).is_err());
}
#[test]
fn memory_call_and_volatile_effect_contracts_are_verified() {
    let mut module = fixture();
    let int = module.types.integer(Int);
    let volatile = module
        .types
        .qualify(
            int,
            Qualifiers {
                volatile: true,
                ..Default::default()
            },
        )
        .unwrap();
    let pointer = module.types.pointer(volatile).unwrap();
    let function = &mut module.functions[0];
    function.locals[0].ty = volatile;
    function.values = vec![pointer, int];
    function.blocks = vec![Block {
        id: BlockId(0),
        instructions: vec![
            Instruction::AddressLocal {
                dst: ValueId(0),
                local: LocalId(0),
            },
            Instruction::Load {
                dst: ValueId(1),
                address: ValueId(0),
                align: 4,
                volatile: true,
            },
            Instruction::Store {
                address: ValueId(0),
                src: ValueId(1),
                align: 4,
                volatile: true,
            },
        ],
        end: Terminator::Return(Some(ValueId(1))),
    }];
    verify(&module).unwrap();
    let effects = module.functions[0].blocks[0].instructions[1]
        .effects(&module.functions[0], &module.types)
        .unwrap();
    assert!(effects.read && effects.observable && effects.may_trap);
    for (align, volatile) in [(3, true), (4, false)] {
        let mut bad = module.clone();
        bad.functions[0].blocks[0].instructions[1] = Instruction::Load {
            dst: ValueId(1),
            address: ValueId(0),
            align,
            volatile,
        };
        assert!(verify(&bad).is_err());
    }
    let mut module = fixture();
    let int = module.types.integer(Int);
    let ci = module
        .types
        .qualify(
            int,
            Qualifiers {
                constant: true,
                ..Default::default()
            },
        )
        .unwrap();
    module.functions[0].locals[0].ty = ci;
    module.functions[0].blocks[1]
        .instructions
        .push(Instruction::InitializeLocal {
            local: LocalId(0),
            src: ValueId(1),
        });
    verify(&module).unwrap();
    module.functions[0].blocks[1]
        .instructions
        .push(Instruction::WriteLocal {
            local: LocalId(0),
            src: ValueId(1),
        });
    assert!(verify(&module).is_err());
    let mut module = fixture();
    let int = module.types.integer(Int);
    let signature = module.types.function(int, vec![int], true, true).unwrap();
    let fp = module.types.pointer(signature).unwrap();
    let f = &mut module.functions[0];
    f.locals[0].ty = fp;
    f.values = vec![fp, int, int];
    f.blocks = vec![Block {
        id: BlockId(0),
        instructions: vec![
            Instruction::ReadLocal {
                dst: ValueId(0),
                local: LocalId(0),
            },
            Instruction::Constant {
                dst: ValueId(1),
                bits: 5,
            },
            Instruction::Call {
                dst: Some(ValueId(2)),
                callee: ValueId(0),
                signature,
                arguments: vec![ValueId(1), ValueId(1)],
            },
        ],
        end: Terminator::Return(Some(ValueId(2))),
    }];
    verify(&module).unwrap();
    assert!(
        module.functions[0].blocks[0].instructions[2]
            .effects(&module.functions[0], &module.types)
            .unwrap()
            .observable
    );
    let mut bad = module.clone();
    bad.functions[0].blocks[0].instructions[2] = Instruction::Call {
        dst: Some(ValueId(2)),
        callee: ValueId(0),
        signature,
        arguments: vec![],
    };
    assert!(verify(&bad).is_err());
    let mut bad = module;
    bad.functions[0].values[1] = bad.types.integer(UnsignedInt);
    assert!(verify(&bad).is_err());
}
#[test]
fn switch_cases_and_cross_word_dataflow_are_checked() {
    let mut m = fixture();
    let int = m.types.integer(Int);
    let f = &mut m.functions[0];
    f.values = vec![int; 65];
    f.blocks = vec![
        Block {
            id: BlockId(0),
            instructions: vec![Instruction::Constant {
                dst: ValueId(64),
                bits: 1,
            }],
            end: Terminator::Switch {
                value: ValueId(64),
                cases: vec![(0, BlockId(1))],
                default: BlockId(2),
            },
        },
        Block {
            id: BlockId(1),
            instructions: vec![],
            end: Terminator::Return(Some(ValueId(64))),
        },
        Block {
            id: BlockId(2),
            instructions: vec![],
            end: Terminator::Return(Some(ValueId(64))),
        },
    ];
    verify(&m).unwrap();
    let mut bad = m.clone();
    if let Terminator::Switch { cases, .. } = &mut bad.functions[0].blocks[0].end {
        cases.push((0, BlockId(2)));
    }
    assert!(verify(&bad).is_err());
    let mut bad = m;
    if let Terminator::Switch { cases, .. } = &mut bad.functions[0].blocks[0].end {
        cases[0].0 = u64::MAX;
    }
    assert!(verify(&bad).is_err());
}
