use crate::{IrNode, Pack, PackAstNode, PackContext};
use ax_diag::{AxResult, Diagnostic};

pub struct StdIoPack;

impl Pack for StdIoPack {
    fn name(&self) -> &'static str {
        "std.io"
    }

    fn provided_syntax(&self) -> &'static [&'static str] {
        &[]
    }

    fn provided_effects(&self) -> &'static [&'static str] {
        &["io.stdout", "io.stderr", "io.stdin"]
    }

    fn expand(&self, _node: PackAstNode<'_>, _ctx: &mut PackContext) -> AxResult<IrNode> {
        Err(Diagnostic::error(
            "AX_PACK_REQUIRED",
            "std.io does not provide top-level syntax",
            ax_core::Span::default(),
        ))
    }
}
