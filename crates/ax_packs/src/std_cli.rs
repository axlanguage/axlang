use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdCliPack;

impl Pack for StdCliPack {
    fn name(&self) -> &'static str {
        "std.cli"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["cli.read"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("std.cli".to_string()))
    }
}
