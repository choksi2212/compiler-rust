//! P14 declaration constraints and unevaluated generic selection.
//! This is a child of typed_front so parser state stays in the existing frontend.
use super::*;

impl Parser {
    /// Consume one alignment specifier. Zero has no effect; all delivered
    /// nonzero alignments are natural powers of two through sixteen bytes.
    pub(super) fn alignment_specifier(&mut self) -> Result<Option<u32>, String> {
        if self.depth >= 64 {
            return Err(error(self.peek(), "alignment nesting limit exceeded"));
        }
        self.depth += 1;
        let result = self.alignment_specifier_inner();
        self.depth -= 1;
        result
    }

    fn alignment_specifier_inner(&mut self) -> Result<Option<u32>, String> {
        let token = self.take();
        self.expect("(")?;
        let value = if self.declaration_start() {
            let ty = self.type_name()?;
            self.types.layout(ty).map_err(|e| error(&token, e))?.align
        } else {
            let expression = self.expr(1)?;
            let value = constant(&self.types, &expression)?.signed_value(&self.types)?;
            u32::try_from(value).map_err(|_| {
                error(
                    &expression.token,
                    "alignment must be nonnegative and representable",
                )
            })?
        };
        self.expect(")")?;
        if value == 0 {
            return Ok(Some(0));
        }
        if !value.is_power_of_two() || value > 16 {
            return Err(error(
                &token,
                "supported alignment is a power of two through16 bytes",
            ));
        }
        Ok(Some(value))
    }

    fn natural_object_alignment(&self, ty: TypeId) -> Result<u32, String> {
        match self.types.get(self.types.unqualified(ty)?)? {
            Type::Array { element, .. } => self.natural_object_alignment(*element),
            _ => Ok(self.types.layout(ty)?.align),
        }
    }

    /// Called only after a declarator resolves, so a pointer's alignment is
    /// checked against the pointer object rather than its base specifier.
    pub(super) fn apply_declaration_alignment(
        &mut self,
        id: SymbolId,
        alignment: Option<u32>,
    ) -> Result<(), String> {
        let Some(alignment) = alignment else {
            return Ok(());
        };
        let declaration = &self.declarations[id.0];
        if !matches!(
            declaration.kind,
            DeclarationKind::Local | DeclarationKind::Global
        ) || declaration.storage == Storage::Register
        {
            return Err(error(
                &declaration.token,
                "alignment specifier requires a non-register object declaration",
            ));
        }
        if alignment == 0 {
            return Ok(());
        }
        let natural = self
            .natural_object_alignment(declaration.ty)
            .map_err(|e| error(&declaration.token, e))?;
        if alignment < natural {
            return Err(error(
                &declaration.token,
                "alignment specifier weakens natural object alignment",
            ));
        }
        if declaration.alignment.is_some_and(|old| old != alignment) {
            return Err(error(
                &declaration.token,
                "conflicting object alignment redeclaration",
            ));
        }
        self.declarations[id.0].alignment = Some(alignment);
        Ok(())
    }

    pub(super) fn static_assert_declaration(&mut self) -> Result<(), String> {
        let token = self.take();
        self.expect("(")?;
        let expression = self.expr(1)?;
        let value = constant(&self.types, &expression)?;
        self.expect(",")?;
        let message = self.take();
        let Kind::String(message) = message.kind else {
            return Err(error(&token, "static assertion requires a string message"));
        };
        self.expect(")")?;
        self.expect(";")?;
        if value.bits == 0 {
            return Err(error(&token, format!("static assertion failed: {message}")));
        }
        Ok(())
    }

    pub(super) fn generic_expression(&mut self, token: Token) -> Result<Expr, String> {
        self.expect("(")?;
        let controlling = self.expr(1)?;
        // Integer promotions are not part of generic controlling-type
        // conversion. In particular, retain a bitfield's declared type.
        let controlling = if Self::bitfield_place(&controlling).is_some() {
            controlling
        } else {
            self.decay(controlling)?
        };
        let controlling_ty = self.types.unqualified(controlling.ty)?;
        self.expect(",")?;
        let mut associations = Vec::new();
        let mut selected = None;
        let mut default = None;
        let mut association_count = 0usize;
        loop {
            association_count += 1;
            if association_count > 1024 {
                return Err(error(self.peek(), "generic association budget exceeded"));
            }
            let association_token = self.peek().clone();
            let ty = if self.eat("default") {
                if default.is_some() {
                    return Err(error(
                        &association_token,
                        "duplicate generic default association",
                    ));
                }
                None
            } else {
                let ty = self.type_name()?;
                self.types
                    .layout(ty)
                    .map_err(|e| error(&association_token, e))?;
                for previous in &associations {
                    if self.types.compatible(*previous, ty)? {
                        return Err(error(
                            &association_token,
                            "duplicate compatible generic association type",
                        ));
                    }
                }
                associations.push(ty);
                Some(ty)
            };
            self.expect(":")?;
            // Every association is parsed and analyzed, but only the selected
            // expression is retained for lowering; the controlling expression
            // itself never becomes executable IR.
            let expression = self.expr(1)?;
            if let Some(ty) = ty {
                if self.types.compatible(controlling_ty, ty)? {
                    if selected.is_some() {
                        return Err(error(
                            &association_token,
                            "multiple matching generic associations",
                        ));
                    }
                    selected = Some(expression);
                }
            } else {
                default = Some(expression);
            }
            if self.eat(")") {
                break;
            }
            self.expect(",")?;
        }
        selected
            .or(default)
            .ok_or_else(|| error(&token, "generic selection has no compatible association"))
    }
}
