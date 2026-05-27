use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdProcessPack;

impl Pack for StdProcessPack {
    fn name(&self) -> &'static str {
        "std.process"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["process.exec"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("std.process".to_string()))
    }
}
