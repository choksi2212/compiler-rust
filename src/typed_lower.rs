//! Typed HIR to explicit, verified control-flow IR.
use crate::{
    integer::{Binary, Unary},
    typed_front::{DeclarationKind, Expr, Expression, Statement, StatementKind, Unit},
    typed_ir::{
        self, Block, BlockId, Function, FunctionId, Instruction as I, Local, LocalId, Module,
        SymbolId, Terminator, ValueId,
    },
    types::{IntegerKind, TypeId},
};
use std::collections::HashMap;
fn key(s: &Statement) -> usize {
    s as *const Statement as usize
}
struct Pending {
    instructions: Vec<I>,
    end: Option<Terminator>,
}
struct Lower<'a> {
    unit: &'a Unit,
    locals: Vec<Local>,
    identities: HashMap<SymbolId, LocalId>,
    scope_locals: HashMap<usize, Vec<LocalId>>,
    values: Vec<TypeId>,
    blocks: Vec<Pending>,
    current: Option<BlockId>,
    targets: HashMap<usize, BlockId>,
    names: HashMap<String, usize>,
    paths: HashMap<usize, Vec<usize>>,
    path: Vec<usize>,
    controls: Vec<(BlockId, Option<BlockId>)>,
}
impl Lower<'_> {
    fn block(&mut self) -> Result<BlockId, String> {
        if self.blocks.len() >= 4096 {
            return Err("typed CFG block budget exceeded".into());
        }
        let id = BlockId(self.blocks.len());
        self.blocks.push(Pending {
            instructions: vec![],
            end: None,
        });
        Ok(id)
    }
    fn active(&mut self) -> Result<BlockId, String> {
        if let Some(b) = self.current {
            Ok(b)
        } else {
            let b = self.block()?;
            self.current = Some(b);
            Ok(b)
        }
    }
    fn emit(&mut self, i: I) -> Result<(), String> {
        let b = self.active()?;
        self.blocks[b.0].instructions.push(i);
        Ok(())
    }
    fn finish(&mut self, end: Terminator) -> Result<(), String> {
        let b = self.active()?;
        if self.blocks[b.0].end.replace(end).is_some() {
            return Err("compiler invariant: block terminated twice".into());
        }
        self.current = None;
        Ok(())
    }
    fn fallthrough(&mut self, target: BlockId) -> Result<(), String> {
        if self.current.is_some() {
            self.finish(Terminator::Jump(target))?;
        }
        Ok(())
    }
    fn activate(&mut self, target: BlockId) -> Result<(), String> {
        self.fallthrough(target)?;
        self.current = Some(target);
        Ok(())
    }
    fn value(&mut self, ty: TypeId) -> Result<ValueId, String> {
        self.unit.types.integer_kind(ty)?;
        if self.values.len() >= 65535 {
            return Err("typed value budget exceeded".into());
        }
        let id = ValueId(self.values.len());
        self.values.push(self.unit.types.unqualified(ty)?);
        Ok(id)
    }
    fn constant(&mut self, ty: TypeId, bits: u64) -> Result<ValueId, String> {
        let dst = self.value(ty)?;
        self.emit(I::Constant { dst, bits })?;
        Ok(dst)
    }
    fn convert(&mut self, src: ValueId, ty: TypeId) -> Result<ValueId, String> {
        let dst = self.value(ty)?;
        self.emit(I::Convert { dst, src })?;
        Ok(dst)
    }
    fn local(&self, symbol: SymbolId) -> Result<LocalId, String> {
        self.identities
            .get(&symbol)
            .copied()
            .ok_or_else(|| "function/typedef value execution is not yet supported".into())
    }
    fn expr(&mut self, e: &Expr) -> Result<ValueId, String> {
        match &e.kind {
            Expression::Integer(value) => self.constant(e.ty, value.bits),
            Expression::Variable(symbol) => {
                let local = self.local(*symbol)?;
                let dst = self.value(e.ty)?;
                self.emit(I::ReadLocal { dst, local })?;
                Ok(dst)
            }
            Expression::String(_) => {
                Err("string object execution is not yet supported in typed mode".into())
            }
            Expression::Convert(a) => {
                let src = self.expr(a)?;
                self.convert(src, e.ty)
            }
            Expression::Unary(op, a) => {
                let src = self.expr(a)?;
                let dst = self.value(e.ty)?;
                self.emit(I::Unary { dst, op: *op, src })?;
                Ok(dst)
            }
            Expression::Binary(op, a, b) => {
                let lhs = self.expr(a)?;
                let rhs = self.expr(b)?;
                let dst = self.value(e.ty)?;
                self.emit(I::Binary {
                    dst,
                    op: *op,
                    lhs,
                    rhs,
                })?;
                Ok(dst)
            }
            Expression::Assign(symbol, rhs) => {
                let src = self.expr(rhs)?;
                let local = self.local(*symbol)?;
                self.emit(I::WriteLocal { local, src })?;
                Ok(src)
            }
            Expression::Update {
                symbol,
                increment,
                postfix,
            } => {
                let local = self.local(*symbol)?;
                let old = self.value(e.ty)?;
                self.emit(I::ReadLocal { dst: old, local })?;
                let promoted = self.unit.types.promoted(e.ty)?;
                let lhs = self.convert(old, promoted)?;
                let rhs = self.constant(promoted, 1)?;
                let result = self.value(promoted)?;
                self.emit(I::Binary {
                    dst: result,
                    op: if *increment {
                        Binary::Add
                    } else {
                        Binary::Subtract
                    },
                    lhs,
                    rhs,
                })?;
                let result = self.convert(result, e.ty)?;
                self.emit(I::WriteLocal { local, src: result })?;
                Ok(if *postfix { old } else { result })
            }
            Expression::Comma(a, b) => {
                self.expr(a)?;
                self.expr(b)
            }
            Expression::Logical(and, a, b) => {
                let condition = self.expr(a)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                let dst = self.value(e.ty)?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                let (evaluate, short) = if *and { (yes, no) } else { (no, yes) };
                self.current = Some(evaluate);
                let value = self.expr(b)?;
                let not = self.value(e.ty)?;
                self.emit(I::Unary {
                    dst: not,
                    op: Unary::Not,
                    src: value,
                })?;
                self.emit(I::Unary {
                    dst,
                    op: Unary::Not,
                    src: not,
                })?;
                self.finish(Terminator::Jump(merge))?;
                self.current = Some(short);
                self.emit(I::Constant {
                    dst,
                    bits: u64::from(!*and),
                })?;
                self.finish(Terminator::Jump(merge))?;
                self.current = Some(merge);
                Ok(dst)
            }
            Expression::Conditional(condition, a, b) => {
                let condition = self.expr(condition)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                let dst = self.value(e.ty)?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                for (block, expression) in [(yes, a), (no, b)] {
                    self.current = Some(block);
                    let src = self.expr(expression)?;
                    self.emit(I::Convert { dst, src })?;
                    self.finish(Terminator::Jump(merge))?;
                }
                self.current = Some(merge);
                Ok(dst)
            }
        }
    }
    fn collect(&mut self, s: &Statement, path: &mut Vec<usize>) -> Result<(), String> {
        let scope = match &s.kind {
            StatementKind::Block { scope, .. } | StatementKind::For { scope, .. } => Some(*scope),
            _ => None,
        };
        if let Some(scope) = scope {
            path.push(scope);
        }
        self.paths.insert(key(s), path.clone());
        if let StatementKind::Label(name, _) = &s.kind {
            self.names.insert(name.clone(), key(s));
        }
        if matches!(s.kind, StatementKind::Case(..) | StatementKind::Label(..)) {
            let target = self.block()?;
            self.targets.insert(key(s), target);
        }
        for child in s.children() {
            self.collect(child, path)?;
        }
        if scope.is_some() {
            path.pop();
        }
        Ok(())
    }
    fn entered_resets(&self, target: usize) -> Vec<I> {
        let path = &self.paths[&target];
        let common = self
            .path
            .iter()
            .zip(path)
            .take_while(|(a, b)| a == b)
            .count();
        let mut resets = vec![];
        for scope in &path[common..] {
            if let Some(locals) = self.scope_locals.get(scope) {
                resets.extend(locals.iter().map(|l| I::ResetLocal(*l)));
            }
        }
        resets
    }
    fn bridge(&mut self, target: usize) -> Result<BlockId, String> {
        let bridge = self.block()?;
        self.blocks[bridge.0].instructions = self.entered_resets(target);
        self.blocks[bridge.0].end = Some(Terminator::Jump(self.targets[&target]));
        Ok(bridge)
    }
    fn stmt(&mut self, s: &Statement) -> Result<(), String> {
        match &s.kind {
            StatementKind::Empty => (),
            StatementKind::Expression(e) => {
                self.expr(e)?;
            }
            StatementKind::Declarations(declarations) => {
                for (symbol, initial) in declarations {
                    let local = self.local(*symbol)?;
                    self.emit(I::ResetLocal(local))?;
                    if let Some(initial) = initial {
                        let src = self.expr(initial)?;
                        self.emit(I::InitializeLocal { local, src })?;
                    }
                }
            }
            StatementKind::Block { scope, items } => {
                self.path.push(*scope);
                for item in items {
                    self.stmt(item)?;
                }
                self.path.pop();
            }
            StatementKind::If(condition, a, b) => {
                let condition = self.expr(condition)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                self.current = Some(yes);
                self.stmt(a)?;
                self.fallthrough(merge)?;
                self.current = Some(no);
                if let Some(b) = b {
                    self.stmt(b)?;
                }
                self.fallthrough(merge)?;
                self.current = Some(merge);
            }
            StatementKind::While(condition, body) => {
                let start = self.block()?;
                let yes = self.block()?;
                let end = self.block()?;
                self.activate(start)?;
                let condition = self.expr(condition)?;
                self.finish(Terminator::Branch {
                    condition,
                    yes,
                    no: end,
                })?;
                self.current = Some(yes);
                self.controls.push((end, Some(start)));
                self.stmt(body)?;
                self.controls.pop();
                self.fallthrough(start)?;
                self.current = Some(end);
            }
            StatementKind::Do(body, condition) => {
                let start = self.block()?;
                let check = self.block()?;
                let end = self.block()?;
                self.activate(start)?;
                self.controls.push((end, Some(check)));
                self.stmt(body)?;
                self.controls.pop();
                self.fallthrough(check)?;
                self.current = Some(check);
                let condition = self.expr(condition)?;
                self.finish(Terminator::Branch {
                    condition,
                    yes: start,
                    no: end,
                })?;
                self.current = Some(end);
            }
            StatementKind::For {
                scope,
                initial,
                condition,
                update,
                body,
            } => {
                self.path.push(*scope);
                self.stmt(initial)?;
                let start = self.block()?;
                let yes = self.block()?;
                let advance = self.block()?;
                let end = self.block()?;
                self.activate(start)?;
                if let Some(condition) = condition {
                    let condition = self.expr(condition)?;
                    self.finish(Terminator::Branch {
                        condition,
                        yes,
                        no: end,
                    })?;
                } else {
                    self.finish(Terminator::Jump(yes))?;
                }
                self.current = Some(yes);
                self.controls.push((end, Some(advance)));
                self.stmt(body)?;
                self.controls.pop();
                self.fallthrough(advance)?;
                self.current = Some(advance);
                if let Some(update) = update {
                    self.expr(update)?;
                }
                self.finish(Terminator::Jump(start))?;
                self.current = Some(end);
                self.path.pop();
            }
            StatementKind::Break => {
                let end = self.controls.last().ok_or("invalid typed break context")?.0;
                self.finish(Terminator::Jump(end))?;
            }
            StatementKind::Continue => {
                let target = self
                    .controls
                    .iter()
                    .rev()
                    .find_map(|c| c.1)
                    .ok_or("invalid typed continue context")?;
                self.finish(Terminator::Jump(target))?;
            }
            StatementKind::Return(e) => {
                let value = self.expr(e)?;
                self.finish(Terminator::Return(Some(value)))?;
            }
            StatementKind::Label(_, child) | StatementKind::Case(_, child) => {
                self.activate(self.targets[&key(s)])?;
                self.stmt(child)?;
            }
            StatementKind::Goto(name) => {
                let target = self.names[name];
                for reset in self.entered_resets(target) {
                    self.emit(reset)?;
                }
                self.finish(Terminator::Jump(self.targets[&target]))?;
            }
            StatementKind::Switch(condition, body) => {
                let value = self.expr(condition)?;
                let end = self.block()?;
                let mut collected = vec![];
                collect_cases(body, &mut collected);
                let mut cases = vec![];
                let mut default = end;
                for case in collected {
                    let StatementKind::Case(value, _) = &case.kind else {
                        unreachable!()
                    };
                    let bridge = self.bridge(key(case))?;
                    if let Some(value) = value {
                        cases.push((value.bits, bridge));
                    } else {
                        default = bridge;
                    }
                }
                self.finish(Terminator::Switch {
                    value,
                    cases,
                    default,
                })?;
                self.controls.push((end, None));
                self.stmt(body)?;
                self.controls.pop();
                self.fallthrough(end)?;
                self.current = Some(end);
            }
        }
        Ok(())
    }
}
fn collect_cases<'a>(s: &'a Statement, cases: &mut Vec<&'a Statement>) {
    if matches!(s.kind, StatementKind::Switch(..)) {
        return;
    }
    if matches!(s.kind, StatementKind::Case(..)) {
        cases.push(s);
    }
    for child in s.children() {
        collect_cases(child, cases);
    }
}
pub fn lower(unit: &Unit) -> Result<Module, String> {
    let main = unit
        .main
        .as_ref()
        .ok_or("native compilation requires a main definition")?;
    let symbol = unit
        .main_symbol
        .ok_or("main declaration identity is missing")?;
    let mut l = Lower {
        unit,
        locals: vec![],
        identities: HashMap::new(),
        scope_locals: HashMap::new(),
        values: vec![],
        blocks: vec![],
        current: None,
        targets: HashMap::new(),
        names: HashMap::new(),
        paths: HashMap::new(),
        path: vec![],
        controls: vec![],
    };
    for (i, declaration) in unit.declarations.iter().enumerate() {
        if declaration.kind == DeclarationKind::Local {
            unit.types.integer_kind(declaration.ty).map_err(|_| {
                format!(
                    "{}:{}: noninteger local execution is not yet supported in typed mode",
                    declaration.token.line, declaration.token.column
                )
            })?;
            let id = LocalId(l.locals.len());
            l.identities.insert(SymbolId(i), id);
            l.scope_locals
                .entry(declaration.scope)
                .or_default()
                .push(id);
            l.locals.push(Local {
                symbol: SymbolId(i),
                ty: declaration.ty,
            });
        }
    }
    let entry = l.block()?;
    l.current = Some(entry);
    l.collect(main, &mut vec![])?;
    l.stmt(main)?;
    if l.current.is_some() {
        let value = l.constant(unit.types.integer(IntegerKind::Int), 0)?;
        l.finish(Terminator::Return(Some(value)))?;
    }
    let mut blocks = vec![];
    for (i, pending) in l.blocks.into_iter().enumerate() {
        blocks.push(Block {
            id: BlockId(i),
            instructions: pending.instructions,
            end: pending
                .end
                .ok_or_else(|| format!("compiler invariant: unterminated typed block {i}"))?,
        });
    }
    let module = Module {
        types: unit.types.clone(),
        globals: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            symbol,
            signature: unit.declarations[symbol.0].ty,
            parameters: vec![],
            locals: l.locals,
            values: l.values,
            entry,
            blocks,
        }],
    };
    typed_ir::verify(&module)?;
    Ok(module)
}
