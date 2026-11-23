//! Explicit partial-C11 pipeline; the legacy checked frontend remains separate.
use crate::context::{CompilationContext, Diagnostic, SourceId, Stage};
#[derive(Debug)]
pub struct Frontend {
    pub tokens: Vec<crate::lexer::Token>,
    pub unit: crate::typed_front::Unit,
}
#[derive(Debug)]
pub struct Compilation {
    pub frontend: Frontend,
    pub ir: crate::typed_ir::Module,
    pub optimized_ir: crate::typed_ir::Module,
    pub image: crate::pe::Image,
    pub machine_dump: String,
}
impl CompilationContext {
    pub fn check_typed(&self, source: SourceId) -> Result<Frontend, Diagnostic> {
        let input = self.source(source)?;
        if self.semantics != crate::context::Semantics::C11 {
            return Err(Diagnostic {
                source,
                stage: Stage::Input,
                message: "typed pipeline requires C11 semantics".into(),
            });
        }
        let tokens = self.run_stage(source, Stage::Lex, || crate::lexer::lex_typed(&input.text))?;
        let unit = self.run_stage(source, Stage::Parse, || {
            crate::typed_front::parse_tokens(tokens.clone())
        })?;
        Ok(Frontend { tokens, unit })
    }
    pub fn compile_typed(
        &self,
        source: SourceId,
        options: &crate::pipeline::NativeCompileOptions,
    ) -> Result<Compilation, Diagnostic> {
        let frontend = self.check_typed(source)?;
        let ir = self.run_stage(source, Stage::Ir, || {
            crate::typed_lower::lower(&frontend.unit)
        })?;
        let optimized_ir =
            self.run_stage(source, Stage::Optimize, || crate::typed_opt::optimize(&ir))?;
        let selected = if options.optimize { &optimized_ir } else { &ir };
        let built = self.run_stage(source, Stage::Native, || {
            crate::native_runtime::build_program(
                &options.runtime,
                &Default::default(),
                &Default::default(),
                |labels, _| crate::typed_native::lower(selected, labels, true),
            )
        })?;
        Ok(Compilation {
            frontend,
            ir,
            optimized_ir,
            image: built.image,
            machine_dump: built.machine_dump,
        })
    }
}
