use ax_ast::Program;
use ax_semantic::SemanticInfo;

#[derive(Clone, Debug)]
pub struct IrProgram {
    pub program: Program,
    pub semantic: SemanticInfo,
}

impl IrProgram {
    pub fn lower(program: &Program, semantic: &SemanticInfo) -> Self {
        Self {
            program: program.clone(),
            semantic: semantic.clone(),
        }
    }
}
