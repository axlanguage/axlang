pub mod registry;
pub mod std_cli;
pub mod std_crypto;
pub mod std_env;
pub mod std_fs;
pub mod std_io;
pub mod std_json;
pub mod std_map;
pub mod std_net_http;
pub mod std_net_http_client;
pub mod std_net_tcp;
pub mod std_path;
pub mod std_process;
pub mod std_str;
pub mod std_time;
pub mod std_url;

use ax_ast::{ServerBlock, TcpBlock};
use ax_diag::{AxResult, Diagnostic};

pub enum PackAstNode<'a> {
    Server(&'a ServerBlock),
    Tcp(&'a TcpBlock),
}

pub struct PackContext;

#[derive(Clone, Debug)]
pub enum IrNode {
    RuntimeCall(String),
}

pub trait Pack {
    fn name(&self) -> &'static str;
    fn provided_syntax(&self) -> &'static [&'static str];
    fn provided_effects(&self) -> &'static [&'static str];
    fn expand(&self, node: PackAstNode<'_>, ctx: &mut PackContext) -> AxResult<IrNode>;
}

pub fn pack_required_diagnostic(
    code: &'static str,
    message: impl Into<String>,
    span: ax_core::Span,
    help: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(code, message, span).help(help)
}
