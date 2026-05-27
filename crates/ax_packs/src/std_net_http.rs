use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdNetHttpPack;

impl Pack for StdNetHttpPack {
    fn name(&self) -> &'static str {
        "std.net.http"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &["server"]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["net.listen", "net.read", "net.write"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("ax_http_server_start".to_string()))
    }
}
