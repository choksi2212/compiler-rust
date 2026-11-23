//! Typed HIR to explicit, verified control-flow IR.
use crate::{
    integer::{Binary, Unary},
    typed_front::{DeclarationKind, Expr, Expression, Statement, StatementKind, Unit},
    typed_ir::{
        self, Block, BlockId, Function, FunctionId, Instruction as I, Local, LocalId, Module,
        SymbolId, Terminator, ValueId,
    },
    types::TypeId,
};
use std::collections::HashMap;
fn key(s: &Statement) -> usize {
    s as *const Statement as usize
}
#[derive(Default)]
struct LiteralPool {
    data: Vec<typed_ir::Data>,
    cache: HashMap<Vec<u8>, typed_ir::DataId>,
}
impl LiteralPool {
    fn add(&mut self, ty: TypeId, text: &str) -> typed_ir::DataId {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        if let Some(id) = self.cache.get(&bytes) {
            return *id;
        }
        let id = typed_ir::DataId(self.data.len());
        self.cache.insert(bytes.clone(), id);
        self.data.push(typed_ir::Data { id, ty, bytes });
        id
    }
}
fn scalar(unit: &Unit, ty: TypeId) -> Result<(), String> {
    if matches!(
        unit.types.get(unit.types.unqualified(ty)?)?,
        crate::types::Type::Integer(_) | crate::types::Type::Pointer(_)
    ) {
        Ok(())
    } else {
        Err("non-scalar execution waits for the memory phase".into())
    }
}
struct Pending {
    instructions: Vec<I>,
    end: Option<Terminator>,
}
struct Lower<'a> {
    unit: &'a Unit,
    literals: &'a mut LiteralPool,
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
        scalar(self.unit, ty)?;
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
    fn variable(&self, symbol: SymbolId) -> Result<typed_ir::Variable, String> {
        if self.unit.declarations[symbol.0].kind == DeclarationKind::Global {
            Ok(typed_ir::Variable::Global(symbol))
        } else {
            self.local(symbol).map(typed_ir::Variable::Local)
        }
    }
    fn read_variable(&mut self, symbol: SymbolId, ty: TypeId) -> Result<ValueId, String> {
        let dst = self.value(ty)?;
        if self.unit.declarations[symbol.0].kind == DeclarationKind::Global {
            self.emit(I::ReadGlobal { dst, symbol })?;
        } else {
            let local = self.local(symbol)?;
            self.emit(I::ReadLocal { dst, local })?;
        }
        Ok(dst)
    }
    fn write_variable(&mut self, symbol: SymbolId, src: ValueId) -> Result<(), String> {
        if self.unit.declarations[symbol.0].kind == DeclarationKind::Global {
            self.emit(I::WriteGlobal { symbol, src })
        } else {
            let local = self.local(symbol)?;
            self.emit(I::WriteLocal { local, src })
        }
    }
    fn call(
        &mut self,
        symbol: SymbolId,
        signature: TypeId,
        arguments: &[Expr],
        result: TypeId,
    ) -> Result<Option<ValueId>, String> {
        let declaration = &self.unit.declarations[symbol.0];
        if declaration.token.text == "printf"
            && declaration.linkage == Some(crate::types::Linkage::External)
            && !self.unit.definitions.iter().any(|d| d.symbol == symbol)
        {
            crate::typed_runtime::service(
                &self.unit.types,
                "printf",
                signature,
                crate::types::Linkage::External,
            )?;
            fn literal(e: &Expr) -> Option<&str> {
                match &e.kind {
                    Expression::String(s) => Some(s),
                    Expression::Convert(e) | Expression::Decay(e) => literal(e),
                    _ => None,
                }
            }
            let format = arguments
                .first()
                .and_then(literal)
                .ok_or("owned printf currently requires a literal format")?;
            let bytes = format.as_bytes();
            let mut cursor = 0;
            let mut argument = 1;
            while cursor < bytes.len() && bytes[cursor] != 0 {
                if bytes[cursor] == b'%' {
                    cursor += 1;
                    let spec = *bytes.get(cursor).ok_or("unterminated printf conversion")?;
                    let value = arguments
                        .get(argument)
                        .ok_or("missing printf conversion argument")?;
                    let ty = self.unit.types.unqualified(value.ty)?;
                    let valid = match spec {
                        b'd' => ty == self.unit.types.integer(crate::types::IntegerKind::Int),
                        b's' => {
                            if let crate::types::Type::Pointer(base) = self.unit.types.get(ty)? {
                                self.unit.types.unqualified(*base)?
                                    == self.unit.types.integer(crate::types::IntegerKind::Char)
                            } else {
                                false
                            }
                        }
                        _ => return Err("owned printf currently supports only %d and %s".into()),
                    };
                    if !valid {
                        return Err("printf conversion argument type mismatch".into());
                    }
                    argument += 1;
                }
                cursor += 1;
            }
        }
        let mut values = vec![];
        for argument in arguments {
            values.push(self.value_expr(argument)?);
        }
        let dst = if result == self.unit.types.void() {
            None
        } else {
            Some(self.value(result)?)
        };
        self.emit(I::DirectCall {
            dst,
            symbol,
            signature,
            arguments: values,
        })?;
        Ok(dst)
    }
    fn expr(&mut self, e: &Expr) -> Result<Option<ValueId>, String> {
        match &e.kind {
            Expression::VaStart { list, last } => {
                let list = self.variable(*list)?;
                let last = self.local(*last)?;
                self.emit(I::VaStart { list, last })?;
                Ok(None)
            }
            Expression::VaCopy {
                destination,
                source,
            } => {
                let destination = self.variable(*destination)?;
                let source = self.variable(*source)?;
                self.emit(I::VaCopy {
                    destination,
                    source,
                })?;
                Ok(None)
            }
            Expression::VaEnd { list } => {
                let list = self.variable(*list)?;
                self.emit(I::VaEnd { list })?;
                Ok(None)
            }
            Expression::Discard(a) => {
                self.expr(a)?;
                Ok(None)
            }
            Expression::Call {
                symbol,
                signature,
                arguments,
            } if e.ty == self.unit.types.void() => self.call(*symbol, *signature, arguments, e.ty),
            Expression::Conditional(condition, a, b) if e.ty == self.unit.types.void() => {
                let condition = self.value_expr(condition)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                for (block, arm) in [(yes, a), (no, b)] {
                    self.current = Some(block);
                    self.expr(arm)?;
                    self.finish(Terminator::Jump(merge))?;
                }
                self.current = Some(merge);
                Ok(None)
            }
            Expression::Comma(a, b) => {
                self.expr(a)?;
                self.expr(b)
            }
            _ => self.value_expr(e).map(Some),
        }
    }
    fn value_expr(&mut self, e: &Expr) -> Result<ValueId, String> {
        match &e.kind {
            Expression::Call {
                symbol,
                signature,
                arguments,
            } => self
                .call(*symbol, *signature, arguments, e.ty)?
                .ok_or_else(|| "void expression has no value".into()),
            Expression::Discard(_) => Err("void expression has no value".into()),
            Expression::VaArg { list } => {
                let list = self.variable(*list)?;
                let dst = self.value(e.ty)?;
                self.emit(I::VaArg { dst, list })?;
                Ok(dst)
            }
            Expression::VaStart { .. } | Expression::VaCopy { .. } | Expression::VaEnd { .. } => {
                Err("void variadic operation has no value".into())
            }
            Expression::Address(symbol) => {
                let dst = self.value(e.ty)?;
                self.emit(I::GlobalAddress {
                    dst,
                    symbol: *symbol,
                })?;
                Ok(dst)
            }
            Expression::Decay(inner) => {
                let Expression::String(text) = &inner.kind else {
                    return Err(
                        "array/function address execution waits for the memory phase".into(),
                    );
                };
                let data = self.literals.add(inner.ty, text);
                let dst = self.value(e.ty)?;
                self.emit(I::DataAddress { dst, data })?;
                Ok(dst)
            }
            Expression::Integer(value) => self.constant(e.ty, value.bits),
            Expression::Variable(symbol) => self.read_variable(*symbol, e.ty),
            Expression::String(_) => {
                Err("string object execution is not yet supported in typed mode".into())
            }
            Expression::Convert(a) => {
                let src = self.value_expr(a)?;
                self.convert(src, e.ty)
            }
            Expression::Unary(op, a) => {
                let src = self.value_expr(a)?;
                let dst = self.value(e.ty)?;
                self.emit(I::Unary { dst, op: *op, src })?;
                Ok(dst)
            }
            Expression::Binary(op, a, b) => {
                let lhs = self.value_expr(a)?;
                let rhs = self.value_expr(b)?;
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
                let src = self.value_expr(rhs)?;
                self.write_variable(*symbol, src)?;
                Ok(src)
            }
            Expression::Update {
                symbol,
                increment,
                postfix,
            } => {
                let old = self.read_variable(*symbol, e.ty)?;
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
                self.write_variable(*symbol, result)?;
                Ok(if *postfix { old } else { result })
            }
            Expression::Comma(a, b) => {
                self.expr(a)?;
                self.value_expr(b)
            }
            Expression::Logical(and, a, b) => {
                let condition = self.value_expr(a)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                let dst = self.value(e.ty)?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                let (evaluate, short) = if *and { (yes, no) } else { (no, yes) };
                self.current = Some(evaluate);
                let value = self.value_expr(b)?;
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
                let condition = self.value_expr(condition)?;
                let yes = self.block()?;
                let no = self.block()?;
                let merge = self.block()?;
                let dst = self.value(e.ty)?;
                self.finish(Terminator::Branch { condition, yes, no })?;
                for (block, expression) in [(yes, a), (no, b)] {
                    self.current = Some(block);
                    let src = self.value_expr(expression)?;
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
                        let src = self.value_expr(initial)?;
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
                let condition = self.value_expr(condition)?;
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
                let condition = self.value_expr(condition)?;
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
                let condition = self.value_expr(condition)?;
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
                    let condition = self.value_expr(condition)?;
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
            StatementKind::ReturnVoid => self.finish(Terminator::Return(None))?,
            StatementKind::Return(e) => {
                let value = self.value_expr(e)?;
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
                let value = self.value_expr(condition)?;
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
        .main_symbol
        .ok_or("native compilation requires a main definition")?;
    let mut literals = LiteralPool::default();
    let mut functions = vec![];
    for definition in &unit.definitions {
        functions.push(lower_function(
            unit,
            definition,
            &mut literals,
            FunctionId(functions.len()),
            definition.symbol == main,
        )?);
    }
    let defined: std::collections::HashSet<_> = unit.definitions.iter().map(|f| f.symbol).collect();
    let externs = unit
        .declarations
        .iter()
        .enumerate()
        .filter(|(i, d)| d.kind == DeclarationKind::Function && !defined.contains(&SymbolId(*i)))
        .map(|(i, d)| typed_ir::External {
            linkage: d.linkage.unwrap_or(crate::types::Linkage::External),
            symbol: SymbolId(i),
            name: d.token.text.clone(),
            signature: d.ty,
        })
        .collect();
    let entry = functions.iter().find(|f| f.symbol == main).map(|f| f.id);
    let globals = unit
        .declarations
        .iter()
        .enumerate()
        .filter(|(_, d)| d.kind == DeclarationKind::Global)
        .map(|(i, d)| {
            let initial = if let Some(e) = &d.initial {
                global_initializer(unit, &mut literals, e, d.ty)?
            } else {
                typed_ir::GlobalInitializer::Zero
            };
            Ok(typed_ir::Global {
                symbol: SymbolId(i),
                name: d.token.text.clone(),
                ty: d.ty,
                linkage: d.linkage.unwrap_or(crate::types::Linkage::Internal),
                defined: d.defined || d.tentative,
                initial,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let module = Module {
        types: unit.types.clone(),
        globals,
        data: literals.data,
        externs,
        entry,
        functions,
    };
    typed_ir::verify(&module)?;
    Ok(module)
}
fn lower_function(
    unit: &Unit,
    definition: &crate::typed_front::Definition,
    literals: &mut LiteralPool,
    id: FunctionId,
    is_main: bool,
) -> Result<Function, String> {
    let mut l = Lower {
        unit,
        literals,
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
    let mut symbols = definition.parameters.clone();
    collect_locals(&definition.body, &mut symbols);
    for symbol in symbols {
        let declaration = &unit.declarations[symbol.0];
        scalar(unit, declaration.ty).map_err(|e| {
            format!(
                "{}:{}: {e}",
                declaration.token.line, declaration.token.column
            )
        })?;
        let local = LocalId(l.locals.len());
        l.identities.insert(symbol, local);
        l.scope_locals
            .entry(declaration.scope)
            .or_default()
            .push(local);
        l.locals.push(Local {
            symbol,
            ty: declaration.ty,
        });
    }
    let parameters = definition
        .parameters
        .iter()
        .map(|id| l.identities[id])
        .collect();
    let entry = l.block()?;
    l.current = Some(entry);
    l.collect(&definition.body, &mut vec![])?;
    l.stmt(&definition.body)?;
    let signature = unit.declarations[definition.symbol.0].ty;
    let crate::types::Type::Function { result, .. } = unit.types.get(signature)? else {
        return Err("function definition lacks a signature".into());
    };
    if l.current.is_some() {
        if *result == unit.types.void() {
            l.finish(Terminator::Return(None))?;
        } else if is_main {
            let zero = l.constant(*result, 0)?;
            l.finish(Terminator::Return(Some(zero)))?;
        } else {
            // C11 permits falling off a nonvoid function when its value is not used.
            // No result is invented; using that call's value has undefined behavior.
            l.finish(Terminator::Fallthrough)?;
        }
    }
    let blocks = l
        .blocks
        .into_iter()
        .enumerate()
        .map(|(i, b)| {
            Ok(Block {
                id: BlockId(i),
                instructions: b.instructions,
                end: b
                    .end
                    .ok_or("compiler invariant: unterminated function block")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Function {
        name: unit.declarations[definition.symbol.0].token.text.clone(),
        linkage: unit.declarations[definition.symbol.0]
            .linkage
            .unwrap_or(crate::types::Linkage::External),
        id,
        symbol: definition.symbol,
        signature,
        parameters,
        locals: l.locals,
        values: l.values,
        entry,
        blocks,
    })
}
fn collect_locals(statement: &Statement, symbols: &mut Vec<SymbolId>) {
    if let StatementKind::Declarations(declarations) = &statement.kind {
        symbols.extend(declarations.iter().map(|d| d.0));
    }
    for child in statement.children() {
        collect_locals(child, symbols);
    }
}

fn global_initializer(
    unit: &Unit,
    literals: &mut LiteralPool,
    e: &Expr,
    storage: TypeId,
) -> Result<typed_ir::GlobalInitializer, String> {
    use typed_ir::GlobalInitializer as G;
    if unit.types.integer_kind(storage).is_ok() {
        return Ok(G::Integer(
            crate::typed_front::constant(&unit.types, e)?
                .cast(&unit.types, unit.types.unqualified(storage)?)?
                .bits,
        ));
    }
    match &e.kind {
        Expression::Address(symbol) => Ok(G::Address {
            symbol: *symbol,
            addend: 0,
        }),
        Expression::Decay(inner) => {
            if let Expression::String(text) = &inner.kind {
                Ok(G::DataAddress {
                    data: literals.add(inner.ty, text),
                    addend: 0,
                })
            } else {
                Err("static function/array addresses wait for the memory phase".into())
            }
        }
        Expression::Convert(inner) => global_initializer(unit, literals, inner, storage),
        Expression::Conditional(condition, a, b) => global_initializer(
            unit,
            literals,
            if crate::typed_front::constant(&unit.types, condition)?.bits != 0 {
                a
            } else {
                b
            },
            storage,
        ),
        _ => {
            if crate::typed_front::constant(&unit.types, e).is_ok_and(|v| v.bits == 0) {
                Ok(G::Zero)
            } else {
                Err("unsupported static pointer initializer".into())
            }
        }
    }
}
