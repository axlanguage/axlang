use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdJsonPack;

impl Pack for StdJsonPack {
    fn name(&self) -> &'static str {
        "std.json"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &[]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("std.json".to_string()))
    }
}
