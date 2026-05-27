use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdTimePack;

impl Pack for StdTimePack {
    fn name(&self) -> &'static str {
        "std.time"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["time.now", "time.sleep"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("std.time".to_string()))
    }
}
