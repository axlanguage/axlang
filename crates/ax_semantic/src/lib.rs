use ax_ast::*;
use ax_core::Span;
use ax_diag::{AxResult, Diagnostic};
use ax_effects::CORE_EFFECTS;
use ax_packs::registry::PackRegistry;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct CheckOptions {
    pub require_main: bool,
    pub packs: Vec<PackSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackSpec {
    pub name: String,
    pub syntax: Vec<String>,
    pub operations: Vec<String>,
    pub effects: Vec<String>,
    pub native_sources: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SemanticInfo {
    pub functions: BTreeMap<String, FunctionInfo>,
    pub records: BTreeMap<String, RecordInfo>,
    pub enums: BTreeMap<String, VariantInfo>,
    pub errors: BTreeMap<String, VariantInfo>,
    pub servers: Vec<ServerInfo>,
    pub tcp_servers: Vec<TcpInfo>,
    pub packs: Vec<PackSpec>,
}

#[derive(Clone, Debug)]
pub struct FunctionInfo {
    pub name: String,
    pub is_async: bool,
    pub params: Vec<(String, TypeRef)>,
    pub ret: TypeRef,
    pub effects: Vec<String>,
    pub inferred_effects: Vec<String>,
    pub calls: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RecordInfo {
    pub name: String,
    pub fields: BTreeMap<String, TypeRef>,
}

#[derive(Clone, Debug)]
pub struct VariantInfo {
    pub name: String,
    pub variants: BTreeMap<String, i32>,
}

#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub port: u16,
    pub tls: bool,
    pub route_count: usize,
}

#[derive(Clone, Debug)]
pub struct TcpInfo {
    pub port: u16,
    pub tls: bool,
    pub route_count: usize,
}

pub fn check_program(program: &Program) -> AxResult<SemanticInfo> {
    check_program_with_options(
        program,
        CheckOptions {
            require_main: true,
            packs: Vec::new(),
        },
    )
}

pub fn check_program_with_packs(program: &Program, packs: Vec<PackSpec>) -> AxResult<SemanticInfo> {
    check_program_with_options(
        program,
        CheckOptions {
            require_main: true,
            packs,
        },
    )
}

pub fn check_program_with_options(
    program: &Program,
    options: CheckOptions,
) -> AxResult<SemanticInfo> {
    Checker::new(program, options).check()
}

pub fn semantic_graph(path: &str, program: &Program, info: &SemanticInfo) -> String {
    let mut out = format!("program: {}\n", path);
    if !program.uses.is_empty() {
        out.push_str("uses:\n");
        for use_decl in &program.uses {
            out.push_str(&format!("- {}\n", use_decl.path));
        }
    }
    if !info.functions.is_empty() {
        out.push_str("\nfunctions:\n");
        for function in info.functions.values() {
            let params = function
                .params
                .iter()
                .map(|(name, ty)| format!("{}: {}", name, ty.display()))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "- {}({}) -> {}\n",
                function.name,
                params,
                function.ret.display()
            ));
            if !function.effects.is_empty() {
                out.push_str(&format!("  effects: {}\n", function.effects.join(", ")));
            }
            if function.inferred_effects != function.effects {
                out.push_str(&format!(
                    "  inferred: {}\n",
                    function.inferred_effects.join(", ")
                ));
            }
            if !function.calls.is_empty() {
                out.push_str("  calls:\n");
                for call in &function.calls {
                    out.push_str(&format!("  - {}\n", call));
                }
            }
        }
    }
    for server in &info.servers {
        out.push_str("\nservers:\n");
        out.push_str(&format!("- port: {}\n", server.port));
        if server.tls {
            out.push_str("  tls: true\n");
        }
        out.push_str("  effects: net.listen, net.read, net.write\n");
        out.push_str("  routes:\n");
        for item in &program.items {
            if let Item::Server(block) = item {
                for route in &block.routes {
                    out.push_str(&format!(
                        "  - {} {} => {}\n",
                        route.method.as_str(),
                        route.path,
                        response_summary(&route.response)
                    ));
                }
            }
        }
    }
    for tcp in &info.tcp_servers {
        out.push_str("\ntcp:\n");
        out.push_str(&format!("- port: {}\n", tcp.port));
        if tcp.tls {
            out.push_str("  tls: true\n");
        }
        out.push_str("  effects: net.listen, net.read, net.write\n");
        out.push_str("  routes:\n");
        for item in &program.items {
            if let Item::Tcp(block) = item {
                for route in &block.routes {
                    out.push_str(&format!(
                        "  - {} => {:?}\n",
                        tcp_pattern_summary(&route.pattern),
                        route.response
                    ));
                }
            }
        }
    }
    out
}

pub fn explain_program(program: &Program, info: &SemanticInfo) -> String {
    if let Some(server) = info.servers.first() {
        let mut out = String::from("Ax program summary:\n\n");
        if server.tls {
            out.push_str("This program uses std.net.http to create a native HTTPS server.\n");
        } else {
            out.push_str("This program uses std.net.http to create a native HTTP/1.1 server.\n");
        }
        out.push_str(&format!("It listens on port {}.\n", server.port));
        out.push_str("It handles:\n");
        for item in &program.items {
            if let Item::Server(block) = item {
                for route in &block.routes {
                    let content_type = match route.response {
                        HttpResponse::Text(_) => "text/plain",
                        HttpResponse::Json(_) => "application/json",
                        HttpResponse::StreamBody => "application/octet-stream",
                    };
                    out.push_str(&format!(
                        "- {} {} -> 200 {} {}\n",
                        route.method.as_str(),
                        route.path,
                        content_type,
                        response_summary(&route.response)
                    ));
                }
            }
        }
        out.push_str("\nUnknown routes return 404 Not Found.\n\n");
        out.push_str("Effects:\n- net.listen\n- net.read\n- net.write\n\n");
        out.push_str("Native generation:\n- emits LLVM IR\n- links Ax runtime socket implementation\n- produces a native executable\n");
        return out;
    }
    if let Some(tcp) = info.tcp_servers.first() {
        let mut out = String::from("Ax program summary:\n\n");
        if tcp.tls {
            out.push_str("This program uses std.net.tcp to create a native TLS TCP server.\n");
        } else {
            out.push_str("This program uses std.net.tcp to create a native TCP server.\n");
        }
        out.push_str(&format!("It listens on port {}.\n", tcp.port));
        out.push_str("It handles:\n");
        for item in &program.items {
            if let Item::Tcp(block) = item {
                for route in &block.routes {
                    out.push_str(&format!(
                        "- {} -> {:?}\n",
                        tcp_pattern_summary(&route.pattern),
                        route.response
                    ));
                }
            }
        }
        out.push_str("\nEffects:\n- net.listen\n- net.read\n- net.write\n\n");
        out.push_str("Native generation:\n- emits LLVM IR route tables\n- links Ax runtime socket implementation\n- produces a native executable\n");
        return out;
    }
    let mut out = String::from("Ax program summary:\n\n");
    for function in info.functions.values() {
        out.push_str(&format!(
            "- function {} returns {}",
            function.name,
            function.ret.display()
        ));
        if !function.effects.is_empty() {
            out.push_str(&format!(" with effects {}", function.effects.join(", ")));
        }
        if function.inferred_effects != function.effects {
            out.push_str(&format!(
                " (inferred: {})",
                function.inferred_effects.join(", ")
            ));
        }
        out.push('\n');
    }
    out.push_str("\nNative generation:\n- emits LLVM IR\n- links Ax runtime implementation\n- produces a native executable\n");
    out
}

fn response_summary(response: &HttpResponse) -> String {
    match response {
        HttpResponse::Text(value) => format!("text {:?}", value),
        HttpResponse::Json(fields) => {
            let body = fields
                .iter()
                .map(|(key, value)| format!("{}: {}", key, expr_summary(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("json {{ {} }}", body)
        }
        HttpResponse::StreamBody => "stream body".to_string(),
    }
}

fn tcp_pattern_summary(pattern: &TcpPattern) -> String {
    match pattern {
        TcpPattern::Exact(value) => format!("{:?}", value),
        TcpPattern::Wildcard => "*".to_string(),
    }
}

fn expr_summary(expr: &Expr) -> String {
    match expr {
        Expr::Bool(value, _) => value.to_string(),
        Expr::Str(value, _) => format!("{:?}", value),
        Expr::Int(value, _) => value.to_string(),
        Expr::Float(value, _) => value.to_string(),
        Expr::Ident(value, _) => value.clone(),
        Expr::Await { expr, .. } => format!("await {}", expr_summary(expr)),
        _ => "<expr>".to_string(),
    }
}

struct Checker<'a> {
    program: &'a Program,
    options: CheckOptions,
    functions: BTreeMap<String, FunctionInfo>,
    records: BTreeMap<String, RecordInfo>,
    enums: BTreeMap<String, VariantInfo>,
    errors: BTreeMap<String, VariantInfo>,
    known_effects: BTreeSet<String>,
    packs: BTreeMap<String, PackSpec>,
}

#[derive(Clone)]
struct Env {
    values: BTreeMap<String, TypeRef>,
    moved: BTreeMap<String, Span>,
}

impl Env {
    fn clear_moves_for(&mut self, name: &str) {
        let prefix = format!("{}.", name);
        self.moved
            .retain(|path, _| path != name && !path.starts_with(&prefix));
    }

    fn moved_for_value(&self, name: &str) -> Option<(String, Span)> {
        if let Some(span) = self.moved.get(name) {
            return Some((name.to_string(), *span));
        }
        let prefix = format!("{}.", name);
        self.moved
            .iter()
            .find(|(path, _)| path.starts_with(&prefix))
            .map(|(path, span)| (path.clone(), *span))
    }

    fn moved_for_path(&self, path: &str) -> Option<(String, Span)> {
        if let Some(span) = self.moved.get(path) {
            return Some((path.to_string(), *span));
        }
        let root = path.split('.').next().unwrap_or(path);
        self.moved.get(root).map(|span| (root.to_string(), *span))
    }
}

#[derive(Clone)]
struct ExprInfo {
    ty: TypeRef,
    effects: Vec<(String, Span)>,
    calls: Vec<String>,
}

impl<'a> Checker<'a> {
    fn new(program: &'a Program, options: CheckOptions) -> Self {
        let packs = options
            .packs
            .iter()
            .cloned()
            .map(|pack| (pack.name.clone(), pack))
            .collect();
        Self {
            program,
            options,
            functions: BTreeMap::new(),
            records: BTreeMap::new(),
            enums: BTreeMap::new(),
            errors: BTreeMap::new(),
            known_effects: BTreeSet::new(),
            packs,
        }
    }

    fn check(mut self) -> AxResult<SemanticInfo> {
        let pack_registry = PackRegistry::default();
        self.known_effects = known_effects(&pack_registry, self.packs.values());
        for use_decl in &self.program.uses {
            if pack_registry.get(&use_decl.path).is_none()
                && !self.packs.contains_key(&use_decl.path)
            {
                return Err(Diagnostic::error(
                    "AX_UNKNOWN_PACK",
                    format!("unknown pack `{}`", use_decl.path),
                    use_decl.span,
                )
                .help("add the pack to ax.toml with `ax add` or run `ax packs` to list available packs"));
            }
        }

        let mut servers = Vec::new();
        let mut tcp_servers = Vec::new();
        for item in &self.program.items {
            match item {
                Item::Type(record) => {
                    let fields = record
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone()))
                        .collect();
                    self.records.insert(
                        record.name.clone(),
                        RecordInfo {
                            name: record.name.clone(),
                            fields,
                        },
                    );
                }
                Item::Function(function) => {
                    self.functions.insert(
                        function.name.clone(),
                        FunctionInfo {
                            name: function.name.clone(),
                            is_async: function.is_async,
                            params: function
                                .params
                                .iter()
                                .map(|param| (param.name.clone(), param.ty.clone()))
                                .collect(),
                            ret: function.ret.clone(),
                            effects: function.effects.clone(),
                            inferred_effects: Vec::new(),
                            calls: Vec::new(),
                        },
                    );
                }
                Item::Enum(item) => {
                    self.enums.insert(
                        item.name.clone(),
                        VariantInfo {
                            name: item.name.clone(),
                            variants: item
                                .variants
                                .iter()
                                .enumerate()
                                .map(|(idx, variant)| (variant.clone(), idx as i32))
                                .collect(),
                        },
                    );
                }
                Item::Error(item) => {
                    self.errors.insert(
                        item.name.clone(),
                        VariantInfo {
                            name: item.name.clone(),
                            variants: item
                                .variants
                                .iter()
                                .enumerate()
                                .map(|(idx, variant)| (variant.clone(), idx as i32))
                                .collect(),
                        },
                    );
                }
                Item::Server(server) => {
                    if server.routes.is_empty() {
                        return Err(Diagnostic::error(
                            "AX_INVALID_ROUTE",
                            "server block must contain at least one route",
                            server.span,
                        ));
                    }
                    servers.push(ServerInfo {
                        port: server.port,
                        tls: server.tls,
                        route_count: server.routes.len(),
                    });
                }
                Item::Tcp(tcp) => {
                    if tcp.routes.is_empty() {
                        return Err(Diagnostic::error(
                            "AX_INVALID_ROUTE",
                            "tcp block must contain at least one route",
                            tcp.span,
                        ));
                    }
                    tcp_servers.push(TcpInfo {
                        port: tcp.port,
                        tls: tcp.tls,
                        route_count: tcp.routes.len(),
                    });
                }
                _ => {}
            }
        }

        if self.options.require_main
            && !self.functions.contains_key("main")
            && servers.is_empty()
            && tcp_servers.is_empty()
        {
            let span = self
                .program
                .uses
                .first()
                .map(|use_decl| use_decl.span)
                .unwrap_or_default();
            return Err(Diagnostic::error(
                "AX_MISSING_MAIN",
                "program requires a main function, server block, or tcp block",
                span,
            ));
        }

        for item in &self.program.items {
            if let Item::Function(function) = item {
                self.validate_declared_effects(function)?;
            }
        }

        for item in &self.program.items {
            if let Item::Function(function) = item {
                let checked = self.check_function(function)?;
                if let Some(info) = self.functions.get_mut(&function.name) {
                    info.calls = checked.calls;
                    info.inferred_effects = unique_effect_names(&checked.effects);
                }
            }
        }

        Ok(SemanticInfo {
            functions: self.functions,
            records: self.records,
            enums: self.enums,
            errors: self.errors,
            servers,
            tcp_servers,
            packs: self.options.packs,
        })
    }

    fn check_function(&self, function: &Function) -> AxResult<ExprInfo> {
        if function.is_async {
            for param in &function.params {
                if !is_async_value_type(&param.ty) {
                    return Err(Diagnostic::error(
                        "AX_ASYNC_UNSUPPORTED",
                        format!(
                            "async parameter {} has unsupported type {}",
                            param.name,
                            param.ty.display()
                        ),
                        param.span,
                    )
                    .help("native async parameters currently support bool, i32, i64, f64, str, ptr, and opaque runtime handles"));
                }
            }
            if function.ret.name != "void" && !is_async_value_type(&function.ret) {
                return Err(Diagnostic::error(
                    "AX_ASYNC_UNSUPPORTED",
                    format!(
                        "async function {} has unsupported return type {}",
                        function.name,
                        function.ret.display()
                    ),
                    function.span,
                )
                .help("native async returns currently support void, bool, i32, i64, f64, str, ptr, and opaque runtime handles"));
            }
        }
        let mut env = Env {
            values: function
                .params
                .iter()
                .map(|param| (param.name.clone(), param.ty.clone()))
                .collect(),
            moved: BTreeMap::new(),
        };
        let info = self.check_block(&function.body, &mut env, Some(&function.ret))?;
        let declared: BTreeSet<_> = function.effects.iter().cloned().collect();
        if !declared.is_empty() {
            for (effect, span) in &info.effects {
                if !declared.contains(effect) {
                    return Err(Diagnostic::error(
                        "AX_EFFECT_MISSING",
                        format!(
                            "function {} uses {} but does not declare it",
                            function.name, effect
                        ),
                        *span,
                    )
                    .help(format!("add `! {}` to function {}", effect, function.name)));
                }
            }
        }
        Ok(info)
    }

    fn validate_declared_effects(&self, function: &Function) -> AxResult<()> {
        for effect in &function.effects {
            if !self.known_effects.contains(effect) {
                return Err(Diagnostic::error(
                    "AX_UNKNOWN_EFFECT",
                    format!(
                        "function {} declares unknown effect {}",
                        function.name, effect
                    ),
                    function.span,
                )
                .help(format!(
                    "known effects: {}",
                    self.known_effects
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(())
    }

    fn check_block(
        &self,
        block: &Block,
        env: &mut Env,
        expected_return: Option<&TypeRef>,
    ) -> AxResult<ExprInfo> {
        let mut effects = Vec::new();
        let mut calls = Vec::new();
        for stmt in &block.stmts {
            let info = self.check_stmt(stmt, env, expected_return)?;
            effects.extend(info.effects);
            calls.extend(info.calls);
        }
        calls.sort();
        calls.dedup();
        Ok(ExprInfo {
            ty: TypeRef::simple("void"),
            effects,
            calls,
        })
    }

    fn check_stmt(
        &self,
        stmt: &Stmt,
        env: &mut Env,
        expected_return: Option<&TypeRef>,
    ) -> AxResult<ExprInfo> {
        match stmt {
            Stmt::Let { name, ty, expr, .. } => {
                let info = self.check_expr(expr, env)?;
                if let Some(expected) = ty {
                    self.require_type(expected, &info.ty, expr.span())?;
                }
                env.values
                    .insert(name.clone(), ty.clone().unwrap_or(info.ty.clone()));
                env.clear_moves_for(name);
                Ok(info)
            }
            Stmt::Assign { target, expr, .. } => {
                let mut info = self.check_expr(expr, env)?;
                if let Expr::Ident(name, _) = target {
                    let Some(expected) = env.values.get(name).cloned() else {
                        return Err(Diagnostic::error(
                            "AX_UNKNOWN_SYMBOL",
                            format!("unknown symbol `{}`", name),
                            target.span(),
                        ));
                    };
                    self.require_type(&expected, &info.ty, expr.span())?;
                    env.clear_moves_for(name);
                } else {
                    let target_info = self.check_expr(target, env)?;
                    self.require_type(&target_info.ty, &info.ty, expr.span())?;
                    info.effects.extend(target_info.effects);
                    info.calls.extend(target_info.calls);
                }
                Ok(info)
            }
            Stmt::Return { expr, span } => {
                let info = if let Some(expr) = expr {
                    self.check_expr(expr, env)?
                } else {
                    ExprInfo {
                        ty: TypeRef::simple("void"),
                        effects: Vec::new(),
                        calls: Vec::new(),
                    }
                };
                if let Some(expected) = expected_return {
                    self.require_type(
                        expected,
                        &info.ty,
                        expr.as_ref().map(Expr::span).unwrap_or(*span),
                    )?;
                }
                Ok(info)
            }
            Stmt::Expr { expr, .. } | Stmt::Assert { expr, .. } => self.check_expr(expr, env),
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let mut info = self.check_expr(cond, env)?;
                let mut then_env = env.clone();
                let then_info = self.check_block(then_block, &mut then_env, expected_return)?;
                info.effects.extend(then_info.effects);
                info.calls.extend(then_info.calls);
                if let Some(else_block) = else_block {
                    let mut else_env = env.clone();
                    let else_info = self.check_block(else_block, &mut else_env, expected_return)?;
                    info.effects.extend(else_info.effects);
                    info.calls.extend(else_info.calls);
                }
                Ok(info)
            }
            Stmt::Loop { body, .. } => self.check_block(body, env, expected_return),
            Stmt::While { cond, body, .. } => {
                let mut info = self.check_expr(cond, env)?;
                let body_info = self.check_block(body, env, expected_return)?;
                info.effects.extend(body_info.effects);
                info.calls.extend(body_info.calls);
                Ok(info)
            }
        }
    }

    fn check_expr(&self, expr: &Expr, env: &mut Env) -> AxResult<ExprInfo> {
        match expr {
            Expr::Int(_, _) => Ok(type_only("i32")),
            Expr::Float(_, _) => Ok(type_only("f64")),
            Expr::Bool(_, _) => Ok(type_only("bool")),
            Expr::Str(_, _) => Ok(type_only("str")),
            Expr::Ident(name, span) => {
                if let Some((moved_path, move_span)) = env.moved_for_value(name) {
                    let message = if moved_path == name.as_str() {
                        format!("use of moved value `{}`", name)
                    } else {
                        format!("use of partially moved value `{}`", name)
                    };
                    return Err(Diagnostic::error("AX_USE_AFTER_MOVE", message, *span).help(
                        format!(
                            "`{}` was moved at line {}, column {}",
                            moved_path, move_span.line, move_span.column
                        ),
                    ));
                }
                if let Some(ty) = env.values.get(name).cloned() {
                    if self.is_move_only_type(&ty) {
                        env.moved.insert(name.clone(), *span);
                    }
                    return Ok(ExprInfo {
                        ty,
                        effects: Vec::new(),
                        calls: Vec::new(),
                    });
                }
                self.functions
                    .get(name)
                    .map(|function| ExprInfo {
                        ty: function.ret.clone(),
                        effects: Vec::new(),
                        calls: Vec::new(),
                    })
                    .ok_or_else(|| {
                        Diagnostic::error(
                            "AX_UNKNOWN_SYMBOL",
                            format!("unknown symbol `{}`", name),
                            *span,
                        )
                    })
            }
            Expr::Member {
                object,
                field,
                span,
            } => {
                let path = expr_name(expr).unwrap_or_else(|| format!("<member:{}>", field));
                if let Some((moved_path, move_span)) = env.moved_for_path(&path) {
                    return Err(Diagnostic::error(
                        "AX_USE_AFTER_MOVE",
                        format!("use of moved value `{}`", path),
                        *span,
                    )
                    .help(format!(
                        "`{}` was moved at line {}, column {}",
                        moved_path, move_span.line, move_span.column
                    )));
                }
                if let Expr::Ident(root, _) = object.as_ref() {
                    let root_ty = env.values.get(root).cloned();
                    if let Some(root_ty) = root_ty {
                        if let Some(record) = self.records.get(&root_ty.name) {
                            if let Some(field_ty) = record.fields.get(field) {
                                if self.is_move_only_type(field_ty) {
                                    env.moved.insert(path, *span);
                                }
                                return Ok(ExprInfo {
                                    ty: field_ty.clone(),
                                    effects: Vec::new(),
                                    calls: Vec::new(),
                                });
                            }
                        }
                    }
                    if self
                        .enums
                        .get(root)
                        .is_some_and(|item| item.variants.contains_key(field))
                    {
                        return Ok(ExprInfo {
                            ty: TypeRef::simple(root),
                            effects: Vec::new(),
                            calls: Vec::new(),
                        });
                    }
                    if self
                        .errors
                        .get(root)
                        .is_some_and(|item| item.variants.contains_key(field))
                    {
                        return Ok(ExprInfo {
                            ty: TypeRef::simple(root),
                            effects: Vec::new(),
                            calls: Vec::new(),
                        });
                    }
                }
                Ok(ExprInfo {
                    ty: TypeRef::simple("member"),
                    effects: Vec::new(),
                    calls: vec![path],
                })
            }
            Expr::Call { callee, args, span } => {
                let callee_name = expr_name(callee).unwrap_or_else(|| "<call>".to_string());
                let mut effects = Vec::new();
                let mut calls = vec![callee_name.clone()];
                let mut arg_types = Vec::new();
                for arg in args {
                    let info = self.check_expr(arg, env)?;
                    arg_types.push(info.ty.clone());
                    effects.extend(info.effects);
                    calls.extend(info.calls);
                }
                let ty = match callee_name.as_str() {
                    "io.println" | "io.print" => {
                        self.require_builtin_call(
                            "std.io",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("io.stdout".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "io.eprintln" => {
                        self.require_builtin_call(
                            "std.io",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("io.stderr".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "io.read_line" => {
                        self.require_builtin_call("std.io", &callee_name, &arg_types, &[], *span)?;
                        effects.push(("io.stdin".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_text" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_text_or" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_text_limit" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_text_range" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_text_tail" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_lines" | "fs.read_lines_json" | "fs.read_jsonl" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_json" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_json_or" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_base64" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_base64_range" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.read_base64_tail" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.write_text"
                    | "fs.write_text_atomic"
                    | "fs.write_json_atomic"
                    | "fs.write_base64"
                    | "fs.append_text"
                    | "fs.append_jsonl" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "fs.exists" | "fs.list" | "fs.list_json" | "fs.list_stat_json" | "fs.walk"
                    | "fs.walk_json" | "fs.walk_stat_json" | "fs.size" | "fs.stat_json"
                    | "fs.is_file" | "fs.is_dir" | "fs.modified" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple(match callee_name.as_str() {
                            "fs.exists" | "fs.is_file" | "fs.is_dir" => "bool",
                            "fs.size" | "fs.modified" => "i64",
                            _ => "str",
                        })
                    }
                    "fs.find" | "fs.glob" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.remove" | "fs.remove_dir" | "fs.mkdir" | "fs.mkdir_all"
                    | "fs.ensure_parent" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "fs.copy" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("fs.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "fs.rename" => {
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "fs.cwd" => {
                        self.require_builtin_call("std.fs", &callee_name, &arg_types, &[], *span)?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "fs.temp_dir" => {
                        self.require_builtin_call("std.fs", &callee_name, &arg_types, &[], *span)?;
                        effects.push(("fs.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.sha256_hex" | "crypto.sha256_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.hmac_sha256_hex" | "crypto.hmac_sha256_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.sha256_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.hmac_sha256_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.sha256_file_hex" | "crypto.sha256_file_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.sha256_file_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.hmac_sha256_file_hex" | "crypto.hmac_sha256_file_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.hmac_sha256_file_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.sha256_file_range_hex" | "crypto.sha256_file_range_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.sha256_file_range_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32", "str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.hmac_sha256_file_range_hex" | "crypto.hmac_sha256_file_range_json" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32", "i32"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32", "i32"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.hmac_sha256_file_range_verify_hex" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32", "i32", "str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32", "i32", "str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("crypto.hash".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "crypto.base64_encode" | "crypto.base64_decode" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "crypto.constant_time_eq" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "crypto.random_hex" | "crypto.random_base64url" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &["i32"],
                            *span,
                        )?;
                        effects.push(("crypto.random".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "crypto.uuid_v4" => {
                        self.require_builtin_call(
                            "std.crypto",
                            &callee_name,
                            &arg_types,
                            &[],
                            *span,
                        )?;
                        effects.push(("crypto.random".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "env.get" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("env.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "env.has" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("env.read".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "env.set" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("env.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "env.get_or" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("env.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "env.snapshot_json" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("env.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "env.load_dotenv" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("env.write".to_string(), *span));
                        TypeRef::simple("i32")
                    }
                    "env.load_dotenv_json" => {
                        self.require_builtin_call(
                            "std.env",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_builtin_call(
                            "std.fs",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("fs.read".to_string(), *span));
                        effects.push(("env.write".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "process.exec" => {
                        self.require_builtin_call(
                            "std.process",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("process.exec".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "process.exec_limit" => {
                        self.require_builtin_call(
                            "std.process",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("process.exec".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "process.status" => {
                        self.require_builtin_call(
                            "std.process",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("process.exec".to_string(), *span));
                        TypeRef::simple("i32")
                    }
                    "process.run_json"
                    | "process.run_log_json"
                    | "process.run_lines_json"
                    | "process.run_log_lines_json" => {
                        self.require_builtin_call(
                            "std.process",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("process.exec".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "cli.argc" => {
                        self.require_builtin_call("std.cli", &callee_name, &arg_types, &[], *span)?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("i32")
                    }
                    "cli.arg" => {
                        self.require_builtin_call(
                            "std.cli",
                            &callee_name,
                            &arg_types,
                            &["i32"],
                            *span,
                        )?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "cli.has" => {
                        self.require_builtin_call(
                            "std.cli",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    "cli.value" => {
                        self.require_builtin_call(
                            "std.cli",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "cli.value_or" => {
                        self.require_builtin_call(
                            "std.cli",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "cli.args_json" | "cli.parse_json" => {
                        self.require_builtin_call("std.cli", &callee_name, &arg_types, &[], *span)?;
                        effects.push(("cli.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "http.get" => {
                        self.require_builtin_call(
                            "std.net.http.client",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "http.post" => {
                        self.require_builtin_call(
                            "std.net.http.client",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "http.get_json" => {
                        self.require_builtin_call(
                            "std.net.http.client",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "http.post_json" => {
                        self.require_builtin_call(
                            "std.net.http.client",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32"],
                            *span,
                        )?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    "json.escape" | "json.quote" | "json.compact" | "json.object"
                    | "json.array" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.pair" | "json.string_pair" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.array_push" | "json.string_array_push" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.valid" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "json.keys" | "json.keys_json" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.query_keys_json" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.get" | "json.query" | "json.kind" | "json.query_kind" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.remove" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.get_or" | "json.query_or" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.set" | "json.string_set" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.int" | "json.query_int" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("i32")
                    }
                    "json.bool" | "json.query_bool" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "json.contains" | "json.query_contains" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "json.len" | "json.query_len" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("i32")
                    }
                    "json.at" | "json.query_at" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "i32"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "json.has" | "json.query_has" => {
                        self.require_builtin_call(
                            "std.json",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "str.len" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("i32")
                    }
                    "str.contains" | "str.starts_with" | "str.ends_with" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "str.index_of" | "str.count" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("i32")
                    }
                    "str.trim" | "str.upper" | "str.lower" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.concat" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.repeat" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.replace" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.slice" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "i32", "i32"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.split_json" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.lines_json" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.from_i64" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["i64"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.parse_i64" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("i64")
                    }
                    "str.parse_i32" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("i32")
                    }
                    "str.token" | "str.token_upper" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "str.line" => {
                        self.require_builtin_call(
                            "std.str",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "path.normalize" | "path.basename" | "path.dirname" | "path.extname"
                    | "path.stem" => {
                        self.require_builtin_call(
                            "std.path",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "path.join" => {
                        self.require_builtin_call(
                            "std.path",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "path.is_absolute" => {
                        self.require_builtin_call(
                            "std.path",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("bool")
                    }
                    "url.encode" | "url.decode" | "url.path" | "url.host" | "url.scheme"
                    | "url.query_json" => {
                        self.require_builtin_call(
                            "std.url",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "url.query_get" | "url.query_has" => {
                        self.require_builtin_call(
                            "std.url",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        if callee_name == "url.query_has" {
                            TypeRef::simple("bool")
                        } else {
                            TypeRef::simple("str")
                        }
                    }
                    "url.query_or" => {
                        self.require_builtin_call(
                            "std.url",
                            &callee_name,
                            &arg_types,
                            &["str", "str", "str"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "time.now" => {
                        self.require_builtin_call(
                            "std.time",
                            &callee_name,
                            &arg_types,
                            &[],
                            *span,
                        )?;
                        effects.push(("time.now".to_string(), *span));
                        TypeRef::simple("i64")
                    }
                    "time.now_ms" => {
                        self.require_builtin_call(
                            "std.time",
                            &callee_name,
                            &arg_types,
                            &[],
                            *span,
                        )?;
                        effects.push(("time.now".to_string(), *span));
                        TypeRef::simple("i64")
                    }
                    "time.iso_utc" => {
                        self.require_builtin_call(
                            "std.time",
                            &callee_name,
                            &arg_types,
                            &["i64"],
                            *span,
                        )?;
                        TypeRef::simple("str")
                    }
                    "time.sleep_ms" => {
                        self.require_builtin_call(
                            "std.time",
                            &callee_name,
                            &arg_types,
                            &["i32"],
                            *span,
                        )?;
                        effects.push(("time.sleep".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "panic" => {
                        effects.push(("panic".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "heap.alloc" => {
                        effects.push(("heap.alloc".to_string(), *span));
                        TypeRef::simple("ptr")
                    }
                    "heap.free" => {
                        effects.push(("heap.free".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "async.cancel" => {
                        if arg_types.len() != 1 {
                            return Err(Diagnostic::error(
                                "AX_ARITY_MISMATCH",
                                format!("async.cancel expects 1 argument, got {}", arg_types.len()),
                                *span,
                            ));
                        }
                        if arg_types[0].name != "Future" {
                            return Err(Diagnostic::error(
                                "AX_TYPE_MISMATCH",
                                format!(
                                    "async.cancel expected Future<T>, found {}",
                                    arg_types[0].display()
                                ),
                                *span,
                            ));
                        }
                        effects.push(("async.cancel".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "async.detach" => {
                        if arg_types.len() != 1 {
                            return Err(Diagnostic::error(
                                "AX_ARITY_MISMATCH",
                                format!("async.detach expects 1 argument, got {}", arg_types.len()),
                                *span,
                            ));
                        }
                        if arg_types[0].name != "Future" {
                            return Err(Diagnostic::error(
                                "AX_TYPE_MISMATCH",
                                format!(
                                    "async.detach expected Future<T>, found {}",
                                    arg_types[0].display()
                                ),
                                *span,
                            ));
                        }
                        effects.push(("async.detach".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    "tcp.listen" => {
                        if arg_types.len() == 1 {
                            self.require_type(&TypeRef::simple("i32"), &arg_types[0], *span)?;
                        } else if arg_types.len() == 2 {
                            self.require_type(&TypeRef::simple("str"), &arg_types[0], *span)?;
                            self.require_type(&TypeRef::simple("i32"), &arg_types[1], *span)?;
                        } else {
                            return Err(Diagnostic::error(
                                "AX_ARITY_MISMATCH",
                                format!(
                                    "tcp.listen expects 1 or 2 argument(s), got {}",
                                    arg_types.len()
                                ),
                                *span,
                            ));
                        }
                        effects.push(("net.listen".to_string(), *span));
                        TypeRef::simple("TcpServer")
                    }
                    "tcp.connect" => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("TcpConn")
                    }
                    "tcp.serve_text" => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &["TcpServer", "Map", "str", "i32"],
                            *span,
                        )?;
                        effects.push(("net.listen".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        effects.push(("net.write".to_string(), *span));
                        TypeRef::simple("i32")
                    }
                    name if name.ends_with(".accept") => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &[],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "TcpServer", *span)?;
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("TcpConn")
                    }
                    name if name.ends_with(".read_text") => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &["i32"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "TcpConn", *span)?;
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    name if name.ends_with(".write_text") => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "TcpConn", *span)?;
                        effects.push(("net.write".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    name if name.ends_with(".request_text") => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &["str", "i32"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "TcpConn", *span)?;
                        effects.push(("net.write".to_string(), *span));
                        effects.push(("net.read".to_string(), *span));
                        TypeRef::simple("str")
                    }
                    name if name.ends_with(".close") => {
                        self.require_builtin_call(
                            "std.net.tcp",
                            &callee_name,
                            &arg_types,
                            &[],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "TcpConn", *span)?;
                        TypeRef::simple("void")
                    }
                    "map.new" => {
                        self.require_builtin_call(
                            "std.map",
                            &callee_name,
                            &arg_types,
                            &["i32"],
                            *span,
                        )?;
                        effects.push(("heap.alloc".to_string(), *span));
                        TypeRef::simple("Map")
                    }
                    name if name.ends_with(".set") => {
                        self.require_builtin_call(
                            "std.map",
                            &callee_name,
                            &arg_types,
                            &["str", "str"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        effects.push(("heap.alloc".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    name if name.ends_with(".get") => {
                        self.require_builtin_call(
                            "std.map",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        TypeRef::simple("str")
                    }
                    name if name.ends_with(".has") => {
                        self.require_builtin_call(
                            "std.map",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        TypeRef::simple("bool")
                    }
                    name if name.ends_with(".del") => {
                        self.require_builtin_call(
                            "std.map",
                            &callee_name,
                            &arg_types,
                            &["str"],
                            *span,
                        )?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        effects.push(("heap.free".to_string(), *span));
                        TypeRef::simple("bool")
                    }
                    name if name.ends_with(".len") => {
                        self.require_builtin_call("std.map", &callee_name, &arg_types, &[], *span)?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        TypeRef::simple("i32")
                    }
                    name if name.ends_with(".clear") => {
                        self.require_builtin_call("std.map", &callee_name, &arg_types, &[], *span)?;
                        self.require_method_receiver(callee, env, "Map", *span)?;
                        effects.push(("heap.free".to_string(), *span));
                        TypeRef::simple("void")
                    }
                    name => {
                        if let Some(function) = self.functions.get(name) {
                            if function.params.len() != arg_types.len() {
                                return Err(Diagnostic::error(
                                    "AX_ARITY_MISMATCH",
                                    format!(
                                        "function {} expects {} argument(s), got {}",
                                        name,
                                        function.params.len(),
                                        arg_types.len()
                                    ),
                                    *span,
                                ));
                            }
                            for ((param_name, expected), actual) in
                                function.params.iter().zip(arg_types.iter())
                            {
                                if !type_compatible(expected, actual) {
                                    return Err(Diagnostic::error(
                                        "AX_TYPE_MISMATCH",
                                        format!(
                                            "argument {} to {} expected {}, found {}",
                                            param_name,
                                            name,
                                            expected.display(),
                                            actual.display()
                                        ),
                                        *span,
                                    ));
                                }
                            }
                            for effect in &function.effects {
                                effects.push((effect.clone(), *span));
                            }
                            if function.is_async {
                                future_type(function.ret.clone())
                            } else {
                                function.ret.clone()
                            }
                        } else if let Some(pack) = self.external_pack_for_call(name) {
                            for effect in &pack.effects {
                                effects.push((effect.clone(), *span));
                            }
                            TypeRef::simple("void")
                        } else {
                            return Err(Diagnostic::error(
                                "AX_UNKNOWN_SYMBOL",
                                format!("unknown function `{}`", name),
                                *span,
                            ));
                        }
                    }
                };
                Ok(ExprInfo { ty, effects, calls })
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let mut left_info = self.check_expr(left, env)?;
                let right_info = self.check_expr(right, env)?;
                left_info.effects.extend(right_info.effects);
                left_info.calls.extend(right_info.calls);
                left_info.ty = match op {
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::And
                    | BinaryOp::Or => TypeRef::simple("bool"),
                    _ => TypeRef::simple("i32"),
                };
                Ok(left_info)
            }
            Expr::Unary { expr, op, .. } => {
                let mut info = self.check_expr(expr, env)?;
                if matches!(op, UnaryOp::Not) {
                    info.ty = TypeRef::simple("bool");
                }
                Ok(info)
            }
            Expr::Await { expr, span } => {
                let mut info = self.check_expr(expr, env)?;
                if info.ty.name != "Future" {
                    return Err(Diagnostic::error(
                        "AX_AWAIT_NON_FUTURE",
                        format!("cannot await {}", info.ty.display()),
                        *span,
                    ));
                }
                info.ty = info
                    .ty
                    .args
                    .first()
                    .cloned()
                    .unwrap_or_else(|| TypeRef::simple("void"));
                Ok(info)
            }
            Expr::RecordInit { name, fields, span } => {
                let record = self.records.get(name).ok_or_else(|| {
                    Diagnostic::error(
                        "AX_UNKNOWN_TYPE",
                        format!("unknown record `{}`", name),
                        *span,
                    )
                })?;
                let mut effects = Vec::new();
                let mut calls = Vec::new();
                let mut seen = BTreeSet::new();
                for (field_name, expr) in fields {
                    if !seen.insert(field_name.clone()) {
                        return Err(Diagnostic::error(
                            "AX_DUPLICATE_FIELD",
                            format!("duplicate field `{}` on `{}`", field_name, name),
                            expr.span(),
                        ));
                    }
                    let expected = record.fields.get(field_name).ok_or_else(|| {
                        Diagnostic::error(
                            "AX_UNKNOWN_SYMBOL",
                            format!("unknown field `{}` on `{}`", field_name, name),
                            expr.span(),
                        )
                    })?;
                    let info = self.check_expr(expr, env)?;
                    self.require_type(expected, &info.ty, expr.span())?;
                    effects.extend(info.effects);
                    calls.extend(info.calls);
                }
                if let Some(missing) = record.fields.keys().find(|field| !seen.contains(*field)) {
                    return Err(Diagnostic::error(
                        "AX_MISSING_FIELD",
                        format!("missing field `{}` on `{}`", missing, name),
                        *span,
                    ));
                }
                Ok(ExprInfo {
                    ty: TypeRef::simple(name),
                    effects,
                    calls,
                })
            }
        }
    }

    fn external_pack_for_call(&self, callee_name: &str) -> Option<&PackSpec> {
        let (root, operation) = callee_name.split_once('.')?;
        self.program.uses.iter().find_map(|use_decl| {
            let pack = self.packs.get(&use_decl.path)?;
            if pack_alias(&pack.name) == root
                && pack_operation_matches(pack, callee_name, operation)
            {
                Some(pack)
            } else {
                None
            }
        })
    }

    fn is_move_only_type(&self, ty: &TypeRef) -> bool {
        self.is_move_only_type_inner(ty, &mut BTreeSet::new())
    }

    fn is_move_only_type_inner(&self, ty: &TypeRef, visiting: &mut BTreeSet<String>) -> bool {
        if matches!(ty.name.as_str(), "ptr" | "Future") {
            return true;
        }
        if ty
            .args
            .iter()
            .any(|arg| self.is_move_only_type_inner(arg, visiting))
        {
            return true;
        }
        if ty.array {
            return ty
                .args
                .iter()
                .any(|arg| self.is_move_only_type_inner(arg, visiting));
        }
        if !visiting.insert(ty.name.clone()) {
            return false;
        }
        let move_only = self.records.get(&ty.name).is_some_and(|record| {
            record
                .fields
                .values()
                .any(|field_ty| self.is_move_only_type_inner(field_ty, visiting))
        });
        visiting.remove(&ty.name);
        move_only
    }

    fn require_type(&self, expected: &TypeRef, actual: &TypeRef, span: Span) -> AxResult<()> {
        if type_compatible(expected, actual) {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "AX_TYPE_MISMATCH",
                format!(
                    "expected {}, found {}",
                    expected.display(),
                    actual.display()
                ),
                span,
            ))
        }
    }

    fn require_builtin_call(
        &self,
        _pack: &str,
        callee_name: &str,
        arg_types: &[TypeRef],
        expected: &[&str],
        span: Span,
    ) -> AxResult<()> {
        if arg_types.len() != expected.len() {
            return Err(Diagnostic::error(
                "AX_ARITY_MISMATCH",
                format!(
                    "{} expects {} argument(s), got {}",
                    callee_name,
                    expected.len(),
                    arg_types.len()
                ),
                span,
            ));
        }
        for (actual, expected_name) in arg_types.iter().zip(expected.iter()) {
            self.require_type(&TypeRef::simple(*expected_name), actual, span)?;
        }
        Ok(())
    }

    fn require_method_receiver(
        &self,
        callee: &Expr,
        env: &Env,
        expected: &str,
        span: Span,
    ) -> AxResult<()> {
        let Expr::Member { object, .. } = callee else {
            return Err(Diagnostic::error(
                "AX_TYPE_MISMATCH",
                format!("method call expected receiver {}", expected),
                span,
            ));
        };
        let Expr::Ident(name, _) = object.as_ref() else {
            return Err(Diagnostic::error(
                "AX_TYPE_MISMATCH",
                format!("method call expected receiver {}", expected),
                span,
            ));
        };
        let Some(actual) = env.values.get(name) else {
            return Err(Diagnostic::error(
                "AX_UNKNOWN_SYMBOL",
                format!("unknown symbol `{}`", name),
                span,
            ));
        };
        self.require_type(&TypeRef::simple(expected), actual, span)
    }
}

fn type_only(name: &str) -> ExprInfo {
    ExprInfo {
        ty: TypeRef::simple(name),
        effects: Vec::new(),
        calls: Vec::new(),
    }
}

fn future_type(inner: TypeRef) -> TypeRef {
    TypeRef {
        name: "Future".to_string(),
        args: vec![inner],
        optional: false,
        array: false,
    }
}

fn type_compatible(expected: &TypeRef, actual: &TypeRef) -> bool {
    expected == actual
        || expected.name == actual.name
        || (is_integer_type(&expected.name) && actual.name == "i32" && actual.args.is_empty())
        || (expected.name == "f32" && actual.name == "f64" && actual.args.is_empty())
}

fn is_integer_type(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
    )
}

fn is_async_value_type(ty: &TypeRef) -> bool {
    matches!(
        ty.name.as_str(),
        "bool" | "i32" | "i64" | "f64" | "str" | "ptr" | "TcpServer" | "TcpConn" | "Map"
    ) && ty.args.is_empty()
        && !ty.optional
        && !ty.array
}

fn expr_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Member { object, field, .. } => Some(format!("{}.{}", expr_name(object)?, field)),
        _ => None,
    }
}

fn known_effects<'a>(
    pack_registry: &PackRegistry,
    packs: impl Iterator<Item = &'a PackSpec>,
) -> BTreeSet<String> {
    CORE_EFFECTS
        .iter()
        .copied()
        .chain(pack_registry.effects())
        .chain(packs.flat_map(|pack| pack.effects.iter().map(String::as_str)))
        .map(str::to_string)
        .collect()
}

fn pack_alias(pack_name: &str) -> &str {
    pack_name.rsplit('.').next().unwrap_or(pack_name)
}

fn pack_operation_matches(pack: &PackSpec, callee_name: &str, operation: &str) -> bool {
    if pack.operations.is_empty() {
        return true;
    }
    let pack_qualified = format!("{}.{}", pack.name, operation);
    pack.operations.iter().any(|declared| {
        declared == callee_name || declared == operation || declared == &pack_qualified
    })
}

fn unique_effect_names(effects: &[(String, Span)]) -> Vec<String> {
    effects
        .iter()
        .map(|(effect, _)| effect.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_core::SourceFile;
    use ax_parser::parse_source;

    #[test]
    fn infers_effect_without_declaration() {
        let source = SourceFile::new("test.ax", "{;\"hello\"}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").effects,
            Vec::<String>::new()
        );
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["io.stdout".to_string()]
        );
    }

    #[test]
    fn validates_http_server_without_import() {
        let source = SourceFile::new("test.ax", "&3000{G/ping>\"pong\"}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(info.servers.len(), 1);
    }

    #[test]
    fn rejects_unknown_std_pack_import() {
        let source = SourceFile::new("test.ax", "+std.nope {}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("pack error");
        assert_eq!(err.code, "AX_UNKNOWN_PACK");
    }

    #[test]
    fn rejects_unknown_declared_effect() {
        let source = SourceFile::new("test.ax", "@main():#!db.magic{^0}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("effect error");
        assert_eq!(err.code, "AX_UNKNOWN_EFFECT");
    }

    #[test]
    fn rejects_assignment_to_unknown_symbol() {
        let source = SourceFile::new("test.ax", "{value=1}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("assignment error");
        assert_eq!(err.code, "AX_UNKNOWN_SYMBOL");
    }

    #[test]
    fn records_transitive_inferred_effects() {
        let source = SourceFile::new("test.ax", "@log():void!io.stdout{;\"hello\"} {log()}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["io.stdout".to_string()]
        );
    }

    #[test]
    fn types_io_pack_with_stdout_stderr_and_stdin_effects() {
        let source = SourceFile::new("test.ax", "{Ib(\"prompt:\")Ic(\"diagnostic\")$Id;a}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec![
                "io.stderr".to_string(),
                "io.stdin".to_string(),
                "io.stdout".to_string()
            ]
        );
    }

    #[test]
    fn types_io_without_import() {
        let source = SourceFile::new("test.ax", "{Ib(\"x\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["io.stdout".to_string()]
        );
    }

    #[test]
    fn types_enum_variants() {
        let source = SourceFile::new(
            "test.ax",
            "%%Status{Ok,Fail} {$s=Status.Ok?s:Status.Ok{^0}|{^1}}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(info.enums["Status"].variants["Ok"], 0);
        assert_eq!(info.enums["Status"].variants["Fail"], 1);
    }

    #[test]
    fn types_error_variants() {
        let source = SourceFile::new(
            "test.ax",
            "%!AuthErr{Denied,Expired} @main():AuthErr{^AuthErr.Denied}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(info.errors["AuthErr"].variants["Denied"], 0);
    }

    #[test]
    fn types_primitive_width_literals() {
        let source = SourceFile::new(
            "test.ax",
            "%Packet{tag:u8,count:i64,ratio:f32} {$tiny:i8=7$short:i16=300$count:i64=41$byte:u8=42$word:u16=600$id:u32=70000$wide:u64=100000$ratio:f32=1.5$exact:f64=2.5$packet=Packet{tag:42,count:9,ratio:0.5}?packet.count:9{^0}^1}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(info.records["Packet"].fields["tag"].display(), "u8");
        assert_eq!(info.records["Packet"].fields["count"].display(), "i64");
        assert_eq!(info.records["Packet"].fields["ratio"].display(), "f32");
    }

    #[test]
    fn infers_core_non_io_effects() {
        let source = SourceFile::new("test.ax", "{Fm(\"x\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["fs.read".to_string()]
        );
    }

    #[test]
    fn types_time_pack_with_now_and_sleep_effects() {
        let source = SourceFile::new("test.ax", "{$Ta$Tb$Tc(a)Td(1)$Ta}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["time.now".to_string(), "time.sleep".to_string()]
        );
    }

    #[test]
    fn types_time_without_import() {
        let source = SourceFile::new("test.ax", "{Ta}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["time.now".to_string()]
        );
    }

    #[test]
    fn types_url_pack_as_pure_operations() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"https://agent.local/tools/search?q=Ax%20language&mode=fast\"Ui(a)Uh(a)Ug(a)$Ua(\"Ax language\")Ub(b)Uc(a,\"q\")Ud(a,\"missing\",\"fallback\")Ue(a,\"mode\")Uf(a)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_url_without_import() {
        let source = SourceFile::new("test.ax", "{Ua(\"x\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_fs_and_crypto_packs() {
        let source = SourceFile::new(
            "test.ax",
            "{#(\".ax-out/x\",\".ax-out\",\".ax-out/agent.json\",\".ax-out/atomic\",\".ax-out/trash\",\"k\",\".ax-out/y\",\".ax-out/z\",\"hello\");+b Fb(\".ax-out/nested/artifacts\")Fc(\".ax-out/generated/state.json\");a,i Fh(d,i);;c,\"{ \\\"ok\\\" : true }\"Fj(a,\" agent\")$Fm(a)$Fn(\".ax-out/missing.txt\",\"fallback\")$Fo(a,5)$Fp(a,6,5)$Fq(a,5)$Fr(a,0,1)$Fs(a,0,1)$8(c)$Fl(\".ax-out/missing.json\",\"{ \\\"ok\\\" : false }\")$5(a)$a@$Fw(b)$Fx(a)$Fy(a)$X0(a)$Fah$Fai Faf(a,g)Fag(g,h);+e;\".ax-out/trash/note.txt\",z Fe(e)$U(j)$Cb(k)$Cc(j,B)$4(a)$Ce(a)$Cf(a,K)$Cg(a,6,5)$Ch(a,6,5)$Ci(a,6,5,Q)$Cj(f,B)$Ck(f,B)$Cl(f,B,W)$Cm(f,a)$Cn(f,a)$Co(f,a,_)$Cp(f,a,6,5)$Cq(f,a,6,5)$Cr(f,a,6,5,ac)$Cs(j)$Ct(af)$Cu(ag,j)$Cv(16)$Cx Fao(b)Fz(b)T0(b)Fap(b)U0(b)S0(b)P0(b,\"x\")N0(b,\"*.txt\")Fe(\".ax-out/nested\")Fd(a)Fd(d)Fd(c)Fd(h)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec![
                "crypto.hash".to_string(),
                "crypto.random".to_string(),
                "fs.read".to_string(),
                "fs.write".to_string()
            ]
        );
    }

    #[test]
    fn types_fs_base64_file_helpers() {
        let source = SourceFile::new(
            "test.ax",
            "{$\".ax-out/input.bin\"$M0(a)$J0(a,2,8)$I0(a,8)C0(\".ax-out/output.bin\",b)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["fs.read".to_string(), "fs.write".to_string()]
        );
    }

    #[test]
    fn types_fs_append_jsonl() {
        let source = SourceFile::new(
            "test.ax",
            "{$\".ax-out/events.jsonl\"Fi(a,\"{ \\\"ok\\\" : true }\")$Ft(a,0,1)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["fs.read".to_string(), "fs.write".to_string()]
        );
    }

    #[test]
    fn types_crypto_random_base64url() {
        let source = SourceFile::new("test.ax", "{$Cw(16)}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["crypto.random".to_string()]
        );
    }

    #[test]
    fn types_env_pack() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"AX_TEST\"Ec(a,\"ok\")$Ea(a)$Ed(\"AX_MISSING\",\"fallback\")$Ee(\"AX_\")$Eb(a)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["env.read".to_string(), "env.write".to_string()]
        );
    }

    #[test]
    fn types_env_dotenv_with_fs_pack() {
        let source = SourceFile::new("test.ax", "{$\".env\"$Ef(a)$Eg(a)^b}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["env.write".to_string(), "fs.read".to_string()]
        );
    }

    #[test]
    fn types_env_without_import() {
        let source = SourceFile::new("test.ax", "{Ea(\"AX_TEST\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["env.read".to_string()]
        );
    }

    #[test]
    fn types_process_pack() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"printf ax\"$Xa(a)$Xb(a,4)$Xd(a,4)$Xe(a,4)$Xf(\"echo ax\",16)$Xg(\"echo ax 1>&2\",16)$Xc(a)^h}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["process.exec".to_string()]
        );
    }

    #[test]
    fn types_process_without_import() {
        let source = SourceFile::new("test.ax", "{Xa(\"printf ax\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["process.exec".to_string()]
        );
    }

    #[test]
    fn types_cli_pack_with_read_effect() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"--input\"$Aa$Ab(0)$Ac(a)$Ad(a)$Ae(\"--mode\",\"default\")$Af$Ag^b}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["cli.read".to_string()]
        );
    }

    #[test]
    fn types_cli_without_import() {
        let source = SourceFile::new("test.ax", "{^Aa}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["cli.read".to_string()]
        );
    }

    #[test]
    fn types_http_client_pack_with_net_effects() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"http://127.0.0.1:3010/health\"$\"http://127.0.0.1:3010/echo\"$Ha(a)$Hb(b,c)$Hc(a,256)$Hd(b,c,256)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["net.read".to_string(), "net.write".to_string()]
        );
    }

    #[test]
    fn types_http_client_without_import() {
        let source = SourceFile::new("test.ax", "{Ha(\"http://127.0.0.1\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["net.read".to_string(), "net.write".to_string()]
        );
    }

    #[test]
    fn types_json_pack_as_pure_operations() {
        let source = SourceFile::new(
            "test.ax",
            "{#(\"items\",\"meta.owner\",\"limit\",\"fallback\",\"status\",\"verify\",\"steps\",\"tools\",\"true\",\"ok\")$7(\"{ \\\"ok\\\" : true, \\\"limit\\\" : 2, \\\"meta\\\" : { \\\"owner\\\" : \\\"agent\\\", \\\"enabled\\\" : true }, \\\"steps\\\" : [{ \\\"name\\\" : \\\"read\\\" }], \\\"items\\\" : [1, 2], \\\"tools\\\" : [\\\"read\\\", \\\"verify\\\"] }\")$3(k)$Jm(k,j)$Jn(k,\"missing\",d)$Jf(m)$Jd(\"value\",m)$M(j,i)$H(p)$Je(o)$Jh(Jg(s,i),\"done\")$Ji(k,c,\"3\")$Jj(u,e,\"ready\")$Jk(v,e)$k`b$Jp(k,\"meta.missing\",d)$A0(k,b)$k'\"meta.enabled\"$k#c$k`\"steps.0.name\"$E0(k,a)$6(k,b)$Jae(k,j)$Jaf(k,c)$S(k,h,f)$Jw(k,h,f)$F0(k,a)$T(k,a)$k\\g$k[a,0]$P(k,g,0)$Jy(k)$Jz(k)$H0(k,\"meta\")$Jah(m)^Y+Q+D+Z+{a:t}\\a}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_json_without_import() {
        let source = SourceFile::new("test.ax", "{Jm(\"{}\",\"ok\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_str_pack_as_pure_operations() {
        let source = SourceFile::new(
            "test.ax",
            "{$\"ax\"$Sg(\" Ax Native \")$Si(b)$Sh(b)$Sj(c,d)$Sk(a,2)$Sl(e,\" \",\"-\")$Sn(g,\"-\")$So(\"a\\nb\")$str.token_upper(\"get key\",0)$Sm(g,0,4)$k!$f~a$Sc(f,\"x\")$Sd(f,a)$Se(k,a)$Sf(k,\"AX\")^l+n+o}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_str_without_import() {
        let source = SourceFile::new("test.ax", "{^\"ax\"!}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_path_pack_as_pure_operations() {
        let source = SourceFile::new(
            "test.ax",
            "{$9(\"examples//agents/../hello.ax\")$Pb(\"examples\",\"hello.ax\")$Pc(b)$Pd(b)$Pe(b)$Pf(b)$Pg(b)}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_path_without_import() {
        let source = SourceFile::new("test.ax", "{Pc(\"hello.ax\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            Vec::<String>::new()
        );
    }

    #[test]
    fn types_crypto_without_import() {
        let source = SourceFile::new("test.ax", "{U(\"hello\")}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["crypto.hash".to_string()]
        );
    }

    #[test]
    fn types_crypto_file_hash_without_fs_import() {
        let source = SourceFile::new("test.ax", "{Cg(\"hello\",0,5)}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["crypto.hash".to_string(), "fs.read".to_string()]
        );
    }

    #[test]
    fn rejects_ptr_use_after_move() {
        let source = SourceFile::new("test.ax", "{$Ma(8)$a$a}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("move error");
        assert_eq!(err.code, "AX_USE_AFTER_MOVE");
    }

    #[test]
    fn rejects_ptr_use_after_free() {
        let source = SourceFile::new("test.ax", "{$Ma(8)Mb(a)Mb(a)}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("move error");
        assert_eq!(err.code, "AX_USE_AFTER_MOVE");
    }

    #[test]
    fn rejects_record_ptr_field_use_after_free() {
        let source = SourceFile::new("test.ax", "%a{a:ptr} {$b=a{a:Ma(8)}Mb(b.a)Mb(b.a)}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("field move error");
        assert_eq!(err.code, "AX_USE_AFTER_MOVE");
    }

    #[test]
    fn rejects_record_use_after_field_move() {
        let source = SourceFile::new(
            "test.ax",
            "%a{a:ptr} @b(c:a):void{} {$c=a{a:Ma(8)}$d=c.a b(c)}",
        );
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("partial move error");
        assert_eq!(err.code, "AX_USE_AFTER_MOVE");
    }

    #[test]
    fn rejects_record_missing_field() {
        let source = SourceFile::new("test.ax", "%a{a:i64,b:$} {$b=a{a:1}}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("missing field");
        assert_eq!(err.code, "AX_MISSING_FIELD");
    }

    #[test]
    fn types_async_await() {
        let source = SourceFile::new(
            "test.ax",
            "@@add(x:#,y:#):#{^x+y} {$value=@(add(20,22))^value}",
        );
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert!(info.functions.get("add").expect("add").is_async);
    }

    #[test]
    fn types_async_cancel() {
        let source = SourceFile::new("test.ax", "@@a(b:#,c:#):#{^b+c} {$b=a(20,22)Na(b)}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["async.cancel".to_string()]
        );
    }

    #[test]
    fn types_async_detach() {
        let source = SourceFile::new("test.ax", "@@a():void{} {$b=a()Nb(b)}");
        let program = parse_source(&source).expect("parse");
        let info = check_program(&program).expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["async.detach".to_string()]
        );
    }

    #[test]
    fn rejects_future_use_after_cancel() {
        let source = SourceFile::new(
            "test.ax",
            "@@a(b:#,c:#):#{^b+c} {$b=a(20,22)Na(b)$c=@(b)^c}",
        );
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("future move error");
        assert_eq!(err.code, "AX_USE_AFTER_MOVE");
    }

    #[test]
    fn rejects_await_non_future() {
        let source = SourceFile::new("test.ax", "{^@(1)}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("await error");
        assert_eq!(err.code, "AX_AWAIT_NON_FUTURE");
    }

    #[test]
    fn accepts_external_pack_effects_from_manifest_specs() {
        let source = SourceFile::new("test.ax", "+acme.telemetry {telemetry.track()}");
        let program = parse_source(&source).expect("parse");
        let info = check_program_with_packs(
            &program,
            vec![PackSpec {
                name: "acme.telemetry".to_string(),
                syntax: Vec::new(),
                operations: vec!["telemetry.track".to_string()],
                effects: vec!["telemetry.write".to_string()],
                native_sources: Vec::new(),
            }],
        )
        .expect("semantic");
        assert_eq!(
            info.functions.get("main").expect("main").inferred_effects,
            vec!["telemetry.write".to_string()]
        );
    }

    #[test]
    fn rejects_external_pack_operation_missing_from_manifest_specs() {
        let source = SourceFile::new("test.ax", "+acme.telemetry {telemetry.missing()}");
        let program = parse_source(&source).expect("parse");
        let err = check_program_with_packs(
            &program,
            vec![PackSpec {
                name: "acme.telemetry".to_string(),
                syntax: Vec::new(),
                operations: vec!["telemetry.track".to_string()],
                effects: vec!["telemetry.write".to_string()],
                native_sources: Vec::new(),
            }],
        )
        .expect_err("operation error");
        assert_eq!(err.code, "AX_UNKNOWN_SYMBOL");
    }

    #[test]
    fn rejects_external_pack_import_missing_from_specs() {
        let source = SourceFile::new("test.ax", "+acme.telemetry {}");
        let program = parse_source(&source).expect("parse");
        let err = check_program(&program).expect_err("pack error");
        assert_eq!(err.code, "AX_UNKNOWN_PACK");
    }
}
