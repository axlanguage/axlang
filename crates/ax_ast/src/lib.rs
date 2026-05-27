use ax_core::Span;

pub fn minified_std_call_name(callee: &str) -> Option<String> {
    if let Some(alias) = hot_minified_std_call_name(callee) {
        return Some(alias.to_string());
    }
    let (root, operation) = callee.split_once('.')?;
    let (alias_root, operations) = std_call_pack(root)?;
    let index = operations.iter().position(|item| *item == operation)?;
    Some(format!("{}{}", alias_root, encode_alias_index(index)))
}

pub fn expanded_std_call_alias(alias: &str) -> Option<String> {
    if let Some(callee) = hot_expanded_std_call_alias(alias) {
        return Some(callee.to_string());
    }
    let (alias_root, code) = if let Some((alias_root, code)) = alias.split_once('.') {
        (alias_root, code)
    } else {
        let mut chars = alias.char_indices();
        let (_, first) = chars.next()?;
        let split = first.len_utf8();
        (&alias[..split], &alias[split..])
    };
    let (root, operations) = std_call_pack_by_alias(alias_root)?;
    let index = decode_alias_index(code)?;
    operations
        .get(index)
        .map(|operation| format!("{}.{}", root, operation))
}

pub fn minified_std_call_aliases() -> Vec<String> {
    hot_std_call_aliases()
        .iter()
        .map(|(alias, _)| (*alias).to_string())
        .chain(
            std_call_packs()
                .into_iter()
                .flat_map(|(_, alias_root, operations)| {
                    operations
                        .iter()
                        .enumerate()
                        .map(move |(idx, _)| format!("{}{}", alias_root, encode_alias_index(idx)))
                }),
        )
        .collect()
}

fn hot_minified_std_call_name(callee: &str) -> Option<&'static str> {
    hot_std_call_aliases()
        .iter()
        .find(|(_, hot_callee)| *hot_callee == callee)
        .map(|(alias, _)| *alias)
}

fn hot_expanded_std_call_alias(alias: &str) -> Option<&'static str> {
    hot_std_call_aliases()
        .iter()
        .find(|(hot_alias, _)| *hot_alias == alias)
        .map(|(_, callee)| *callee)
}

fn hot_std_call_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("A", "fs.write_text"),
        ("C", "io.println"),
        ("E", "json.query"),
        ("F", "json.query_bool"),
        ("H", "json.object"),
        ("I", "str.contains"),
        ("J", "str.len"),
        ("M", "json.pair"),
        ("N", "fs.mkdir"),
        ("P", "json.query_at"),
        ("S", "json.contains"),
        ("T", "json.len"),
        ("U", "crypto.sha256_hex"),
        ("X", "json.query_int"),
        ("0", "json.query_len"),
        ("1", "fs.is_file"),
        ("2", "fs.write_json_atomic"),
        ("3", "json.valid"),
        ("4", "crypto.sha256_file_hex"),
        ("5", "fs.exists"),
        ("6", "json.query_has"),
        ("7", "json.compact"),
        ("8", "fs.read_json"),
        ("9", "path.normalize"),
        ("A0", "json.query_kind"),
        ("C0", "fs.write_base64"),
        ("E0", "json.has"),
        ("F0", "json.kind"),
        ("H0", "json.query_keys_json"),
        ("I0", "fs.read_base64_tail"),
        ("J0", "fs.read_base64_range"),
        ("M0", "fs.read_base64"),
        ("N0", "fs.glob"),
        ("P0", "fs.find"),
        ("S0", "fs.walk_stat_json"),
        ("T0", "fs.list_stat_json"),
        ("U0", "fs.walk_json"),
        ("X0", "fs.modified"),
    ]
}

fn std_call_pack(root: &str) -> Option<(&'static str, &'static [&'static str])> {
    std_call_packs()
        .into_iter()
        .find(|(pack_root, _, _)| *pack_root == root)
        .map(|(_, alias_root, operations)| (alias_root, operations))
}

fn std_call_pack_by_alias(alias_root: &str) -> Option<(&'static str, &'static [&'static str])> {
    std_call_packs()
        .into_iter()
        .find(|(_, pack_alias, _)| *pack_alias == alias_root)
        .map(|(pack_root, _, operations)| (pack_root, operations))
}

fn std_call_packs() -> Vec<(&'static str, &'static str, &'static [&'static str])> {
    vec![
        ("io", "I", IO_CALLS),
        ("fs", "F", FS_CALLS),
        ("crypto", "C", CRYPTO_CALLS),
        ("env", "E", ENV_CALLS),
        ("process", "X", PROCESS_CALLS),
        ("cli", "A", CLI_CALLS),
        ("json", "J", JSON_CALLS),
        ("str", "S", STR_CALLS),
        ("path", "P", PATH_CALLS),
        ("url", "U", URL_CALLS),
        ("time", "T", TIME_CALLS),
        ("tcp", "Q", TCP_CALLS),
        ("map", "L", MAP_CALLS),
        ("http", "H", HTTP_CALLS),
        ("heap", "M", HEAP_CALLS),
        ("async", "N", ASYNC_CALLS),
    ]
}

fn encode_alias_index(mut value: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'a' + (value % 26) as u8) as char);
        if value < 26 {
            break;
        }
        value = value / 26 - 1;
    }
    chars.iter().rev().collect()
}

fn decode_alias_index(code: &str) -> Option<usize> {
    if code.is_empty() {
        return None;
    }
    let mut value = 0usize;
    for byte in code.bytes() {
        if !byte.is_ascii_lowercase() {
            return None;
        }
        value = value.checked_mul(26)?;
        value = value.checked_add((byte - b'a' + 1) as usize)?;
    }
    value.checked_sub(1)
}

const IO_CALLS: &[&str] = &["println", "print", "eprintln", "read_line"];

const FS_CALLS: &[&str] = &[
    "mkdir",
    "mkdir_all",
    "ensure_parent",
    "remove",
    "remove_dir",
    "write_json_atomic",
    "write_text",
    "write_text_atomic",
    "append_jsonl",
    "append_text",
    "read_json",
    "read_json_or",
    "read_text",
    "read_text_or",
    "read_text_limit",
    "read_text_range",
    "read_text_tail",
    "read_lines",
    "read_lines_json",
    "read_jsonl",
    "exists",
    "is_file",
    "is_dir",
    "size",
    "stat_json",
    "list_json",
    "walk_json",
    "list_stat_json",
    "walk_stat_json",
    "find",
    "glob",
    "copy",
    "rename",
    "cwd",
    "temp_dir",
    "modified",
    "read_base64",
    "read_base64_range",
    "read_base64_tail",
    "write_base64",
    "list",
    "walk",
];

const CRYPTO_CALLS: &[&str] = &[
    "sha256_hex",
    "sha256_json",
    "sha256_verify_hex",
    "sha256_file_hex",
    "sha256_file_json",
    "sha256_file_verify_hex",
    "sha256_file_range_hex",
    "sha256_file_range_json",
    "sha256_file_range_verify_hex",
    "hmac_sha256_hex",
    "hmac_sha256_json",
    "hmac_sha256_verify_hex",
    "hmac_sha256_file_hex",
    "hmac_sha256_file_json",
    "hmac_sha256_file_verify_hex",
    "hmac_sha256_file_range_hex",
    "hmac_sha256_file_range_json",
    "hmac_sha256_file_range_verify_hex",
    "base64_encode",
    "base64_decode",
    "constant_time_eq",
    "random_hex",
    "random_base64url",
    "uuid_v4",
];

const ENV_CALLS: &[&str] = &[
    "get",
    "has",
    "set",
    "get_or",
    "snapshot_json",
    "load_dotenv",
    "load_dotenv_json",
];

const PROCESS_CALLS: &[&str] = &[
    "exec",
    "exec_limit",
    "status",
    "run_json",
    "run_log_json",
    "run_lines_json",
    "run_log_lines_json",
];

const CLI_CALLS: &[&str] = &[
    "argc",
    "arg",
    "has",
    "value",
    "value_or",
    "args_json",
    "parse_json",
];

const JSON_CALLS: &[&str] = &[
    "compact",
    "object",
    "pair",
    "string_pair",
    "array",
    "quote",
    "array_push",
    "string_array_push",
    "set",
    "string_set",
    "remove",
    "valid",
    "get",
    "get_or",
    "query",
    "query_or",
    "query_bool",
    "query_int",
    "query_has",
    "query_len",
    "query_at",
    "contains",
    "query_contains",
    "at",
    "keys",
    "keys_json",
    "query_keys_json",
    "kind",
    "query_kind",
    "len",
    "bool",
    "int",
    "has",
    "escape",
];

const STR_CALLS: &[&str] = &[
    "len",
    "contains",
    "index_of",
    "count",
    "starts_with",
    "ends_with",
    "trim",
    "upper",
    "lower",
    "concat",
    "repeat",
    "replace",
    "slice",
    "split_json",
    "lines_json",
    "from_i64",
    "parse_i64",
    "parse_i32",
    "token",
    "line",
    "token_upper",
];

const PATH_CALLS: &[&str] = &[
    "normalize",
    "join",
    "basename",
    "dirname",
    "extname",
    "stem",
    "is_absolute",
];

const URL_CALLS: &[&str] = &[
    "encode",
    "decode",
    "query_get",
    "query_or",
    "query_has",
    "query_json",
    "path",
    "host",
    "scheme",
];

const TIME_CALLS: &[&str] = &["now", "now_ms", "iso_utc", "sleep_ms"];
const TCP_CALLS: &[&str] = &["listen", "connect", "serve_text"];
const MAP_CALLS: &[&str] = &["new"];
const HTTP_CALLS: &[&str] = &["get", "post", "get_json", "post_json"];
const HEAP_CALLS: &[&str] = &["alloc", "free"];
const ASYNC_CALLS: &[&str] = &["cancel", "detach"];

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub uses: Vec<UseDecl>,
    pub items: Vec<Item>,
}

impl Program {
    pub fn has_use(&self, path: &str) -> bool {
        self.uses.iter().any(|use_decl| use_decl.path == path)
    }
}

#[derive(Clone, Debug)]
pub struct UseDecl {
    pub path: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Function(Function),
    Type(TypeDecl),
    Enum(EnumDecl),
    Error(ErrorDecl),
    Server(ServerBlock),
    Tcp(TcpBlock),
    Test(TestBlock),
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub is_async: bool,
    pub params: Vec<Param>,
    pub ret: TypeRef,
    pub effects: Vec<String>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
    pub args: Vec<TypeRef>,
    pub optional: bool,
    pub array: bool,
}

impl TypeRef {
    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
            optional: false,
            array: false,
        }
    }

    pub fn display(&self) -> String {
        let mut base = if self.array {
            format!(
                "[{}]",
                self.args.first().map(TypeRef::display).unwrap_or_default()
            )
        } else if self.args.is_empty() {
            self.name.clone()
        } else {
            format!(
                "{}<{}>",
                self.name,
                self.args
                    .iter()
                    .map(TypeRef::display)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if self.optional {
            base.push('?');
        }
        base
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<TypeRef>,
        expr: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        expr: Expr,
        span: Span,
    },
    Return {
        expr: Option<Expr>,
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_block: Block,
        else_block: Option<Block>,
        span: Span,
    },
    Loop {
        body: Block,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Assert {
        expr: Expr,
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    Str(String, Span),
    Ident(String, Span),
    Member {
        object: Box<Expr>,
        field: String,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Await {
        expr: Box<Expr>,
        span: Span,
    },
    RecordInit {
        name: String,
        fields: Vec<(String, Expr)>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, span)
            | Expr::Float(_, span)
            | Expr::Bool(_, span)
            | Expr::Str(_, span)
            | Expr::Ident(_, span)
            | Expr::Member { span, .. }
            | Expr::Call { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Await { span, .. }
            | Expr::RecordInit { span, .. } => *span,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Clone, Debug)]
pub struct TypeDecl {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ErrorDecl {
    pub name: String,
    pub variants: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ServerBlock {
    pub port: u16,
    pub tls: bool,
    pub routes: Vec<HttpRoute>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct HttpRoute {
    pub method: HttpMethod,
    pub path: String,
    pub response: HttpResponse,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Debug)]
pub enum HttpResponse {
    Text(String),
    Json(Vec<(String, Expr)>),
    StreamBody,
}

#[derive(Clone, Debug)]
pub struct TcpBlock {
    pub port: u16,
    pub tls: bool,
    pub routes: Vec<TcpRoute>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TcpRoute {
    pub pattern: TcpPattern,
    pub response: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TcpPattern {
    Exact(String),
    Wildcard,
}

#[derive(Clone, Debug)]
pub struct TestBlock {
    pub name: String,
    pub body: Block,
    pub span: Span,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_call_aliases_are_bidirectional() {
        let alias = minified_std_call_name("io.println").expect("alias");
        assert_eq!(alias, "C");
        assert_eq!(
            expanded_std_call_alias(&alias).as_deref(),
            Some("io.println")
        );
        assert_eq!(expanded_std_call_alias("Ia").as_deref(), Some("io.println"));
        assert_eq!(
            expanded_std_call_alias("I.a").as_deref(),
            Some("io.println")
        );

        let wide = minified_std_call_name("fs.read_base64_range").expect("wide alias");
        assert_eq!(
            expanded_std_call_alias(&wide).as_deref(),
            Some("fs.read_base64_range")
        );
        assert_eq!(minified_std_call_name("json.query").as_deref(), Some("E"));
        assert_eq!(
            minified_std_call_name("json.query_kind").as_deref(),
            Some("A0")
        );
        assert_eq!(
            minified_std_call_name("json.query_len").as_deref(),
            Some("0")
        );
        assert_eq!(
            expanded_std_call_alias("0").as_deref(),
            Some("json.query_len")
        );
        assert_eq!(
            expanded_std_call_alias("A0").as_deref(),
            Some("json.query_kind")
        );
        assert_eq!(
            expanded_std_call_alias("Jac").as_deref(),
            Some("json.query_kind")
        );
        assert_eq!(
            minified_std_call_name("tcp.serve_text").as_deref(),
            Some("Qc")
        );
        assert_eq!(
            expanded_std_call_alias("Qc").as_deref(),
            Some("tcp.serve_text")
        );
        assert_eq!(
            minified_std_call_name("str.token_upper").as_deref(),
            Some("Su")
        );
        assert_eq!(
            expanded_std_call_alias("Su").as_deref(),
            Some("str.token_upper")
        );
        assert!(minified_std_call_aliases().contains(&"Jo".to_string()));
        assert!(minified_std_call_aliases().contains(&"E".to_string()));
        assert!(minified_std_call_aliases().contains(&"A0".to_string()));
        assert!(minified_std_call_aliases().contains(&"0".to_string()));
    }
}
