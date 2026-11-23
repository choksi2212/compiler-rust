//! Compilation job metadata and structured diagnostics for shared compiler stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    WindowsX64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    Teaching,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Input,
    Lex,
    Parse,
    Semantic,
    Ir,
    Optimize,
    Bytecode,
    Runtime,
    Native,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: SourceId,
    pub stage: Stage,
    pub message: String,
}
#[derive(Debug)]
pub struct Source {
    pub name: String,
    pub text: String,
}
#[derive(Debug)]
pub struct CompilationContext {
    pub target: Target,
    pub semantics: Semantics,
    sources: Vec<Source>,
}
impl Default for CompilationContext {
    fn default() -> Self {
        Self {
            target: Target::WindowsX64,
            semantics: Semantics::Teaching,
            sources: Vec::new(),
        }
    }
}
impl CompilationContext {
    pub fn add_source(&mut self, name: impl Into<String>, text: impl Into<String>) -> SourceId {
        let id = SourceId(self.sources.len());
        self.sources.push(Source {
            name: name.into(),
            text: text.into(),
        });
        id
    }
    pub fn source(&self, id: SourceId) -> Result<&Source, Diagnostic> {
        self.sources.get(id.0).ok_or_else(|| Diagnostic {
            source: id,
            stage: Stage::Input,
            message: "invalid source identity".into(),
        })
    }
    pub fn run_stage<T>(
        &self,
        id: SourceId,
        stage: Stage,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, Diagnostic> {
        self.source(id)?;
        operation().map_err(|message| Diagnostic {
            source: id,
            stage,
            message,
        })
    }
    pub fn render(&self, error: &Diagnostic) -> String {
        match self.source(error.source) {
            Ok(source) => format!("{}:{}", source.name, error.message),
            Err(_) => format!("<unknown source>:{}", error.message),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_and_errors() {
        let mut c = CompilationContext::default();
        let a = c.add_source("a.c", "a");
        let b = c.add_source("b.c", "b");
        assert_ne!(a, b);
        assert_eq!(c.source(b).unwrap().text, "b");
        let e = c
            .run_stage(a, Stage::Parse, || Err::<(), _>("1:2: expected ;".into()))
            .unwrap_err();
        assert_eq!(e.stage, Stage::Parse);
        assert_eq!(c.render(&e), "a.c:1:2: expected ;");
        assert!(c.run_stage(SourceId(99), Stage::Parse, || Ok(())).is_err());
    }
}
