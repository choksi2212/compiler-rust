//! Source-to-native orchestration; no file publication or process execution.
use crate::context::{CompilationContext, Diagnostic, SourceId, Stage};
#[derive(Debug, Clone)]
pub struct NativeCompileOptions {
    pub optimize: bool,
    pub runtime: crate::native_runtime::RuntimeOptions,
}
impl Default for NativeCompileOptions {
    fn default() -> Self {
        Self {
            optimize: true,
            runtime: crate::native_runtime::RuntimeOptions::default(),
        }
    }
}
#[derive(Debug)]
pub struct NativeCompilation {
    pub source: SourceId,
    pub tokens: Vec<crate::lexer::Token>,
    pub ast: crate::parser::Stmt,
    pub ir: Vec<crate::ir::Ir>,
    pub optimized_ir: Vec<crate::ir::Ir>,
    pub image: crate::pe::Image,
}
impl CompilationContext {
    pub fn compile_native(
        &self,
        source: SourceId,
        options: &NativeCompileOptions,
    ) -> Result<NativeCompilation, Diagnostic> {
        let input = self.source(source)?;
        let tokens = self.run_stage(source, Stage::Lex, || crate::lexer::lex(&input.text))?;
        let ast = self.run_stage(source, Stage::Parse, || {
            crate::parser::parse(tokens.clone())
        })?;
        self.run_stage(source, Stage::Semantic, || crate::semantic::check(&ast))?;
        let ir = crate::ir::lower(&ast);
        self.run_stage(source, Stage::Ir, || crate::verify::verify(&ir))?;
        let optimized_ir = crate::optimize::optimize(&ir);
        self.run_stage(source, Stage::Optimize, || {
            crate::verify::verify(&optimized_ir)
        })?;
        let image = self.run_stage(source, Stage::Native, || {
            crate::native_runtime::image(
                if options.optimize { &optimized_ir } else { &ir },
                &options.runtime,
            )
        })?;
        Ok(NativeCompilation {
            source,
            tokens,
            ast,
            ir,
            optimized_ir,
            image,
        })
    }
}
