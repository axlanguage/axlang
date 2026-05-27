use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::AxResult;

pub struct StdNetTcpPack;

impl Pack for StdNetTcpPack {
    fn name(&self) -> &'static str {
        "std.net.tcp"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &["tcp"]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["net.listen", "net.read", "net.write"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Ok(IrNode::RuntimeCall("ax_tcp_ping_server".to_string()))
    }
}
