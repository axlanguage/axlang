use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdFsPack;

impl Pack for StdFsPack {
    fn name(&self) -> &'static str {
        "std.fs"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["fs.read", "fs.write"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("std.fs".to_string()))
    }
}
