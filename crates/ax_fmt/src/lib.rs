use ax_ast::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn format_program(program: &Program) -> String {
    minify_program(program)
}

pub fn compact_program(program: &Program) -> String {
    minify_program(program)
}

pub fn minify_program(program: &Program) -> String {
    let plan = MinifyPlan::new(program);
    let mut out = String::new();
    for use_decl in program
        .uses
        .iter()
        .filter(|use_decl| !use_decl.path.starts_with("std."))
    {
        push_top_level_separator(&mut out);
        out.push('+');
        out.push_str(&use_decl.path);
    }
    for item in &program.items {
        push_top_level_separator(&mut out);
        minify_item(item, &plan, &mut out);
    }
    out
}

fn push_top_level_separator(out: &mut String) {
    if !out.is_empty() {
        out.push(' ');
    }
}

struct MinifyPlan {
    globals: BTreeMap<String, String>,
    global_reserved: BTreeSet<String>,
    pack_roots: BTreeSet<String>,
    function_returns: BTreeMap<String, (bool, TypeRef)>,
    record_fields: BTreeMap<String, BTreeMap<String, String>>,
    record_field_types: BTreeMap<String, BTreeMap<String, TypeRef>>,
    variants: BTreeMap<String, BTreeMap<String, String>>,
}

impl MinifyPlan {
    fn new(program: &Program) -> Self {
        let pack_roots = program
            .uses
            .iter()
            .filter_map(|use_decl| use_decl.path.rsplit('.').next())
            .map(str::to_string)
            .chain(std_pack_roots().into_iter().map(str::to_string))
            .collect::<BTreeSet<_>>();
        let mut reserved = reserved_identifiers();
        reserved.extend(pack_roots.iter().cloned());
        reserved.insert("main".to_string());

        let mut globals = BTreeMap::new();
        let mut global_names = NameGenerator::new(reserved.clone());
        for item in &program.items {
            match item {
                Item::Function(function) if function.name != "main" => {
                    globals.insert(function.name.clone(), global_names.next());
                }
                Item::Type(record) => {
                    globals.insert(record.name.clone(), global_names.next());
                }
                Item::Enum(item) => {
                    globals.insert(item.name.clone(), global_names.next());
                }
                Item::Error(item) => {
                    globals.insert(item.name.clone(), global_names.next());
                }
                _ => {}
            }
        }

        let mut global_reserved = reserved;
        global_reserved.extend(globals.values().cloned());

        let mut function_returns = BTreeMap::new();
        let mut record_fields = BTreeMap::new();
        let mut record_field_types = BTreeMap::new();
        let mut variants = BTreeMap::new();
        for item in &program.items {
            match item {
                Item::Function(function) => {
                    function_returns.insert(
                        function.name.clone(),
                        (function.is_async, function.ret.clone()),
                    );
                }
                Item::Type(record) => {
                    let mut field_names = BTreeMap::new();
                    let mut field_types = BTreeMap::new();
                    let mut names = NameGenerator::new(reserved_identifiers());
                    for field in &record.fields {
                        field_names.insert(field.name.clone(), names.next());
                        field_types.insert(field.name.clone(), field.ty.clone());
                    }
                    record_fields.insert(record.name.clone(), field_names);
                    record_field_types.insert(record.name.clone(), field_types);
                }
                Item::Enum(item) => {
                    variants.insert(item.name.clone(), minified_names(&item.variants));
                }
                Item::Error(item) => {
                    variants.insert(item.name.clone(), minified_names(&item.variants));
                }
                _ => {}
            }
        }

        Self {
            globals,
            global_reserved,
            pack_roots,
            function_returns,
            record_fields,
            record_field_types,
            variants,
        }
    }

    fn global_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.globals.get(name).map(String::as_str).unwrap_or(name)
    }

    fn type_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.global_name(name)
    }

    fn record_field_name<'a>(&'a self, record: &str, field: &'a str) -> &'a str {
        self.record_fields
            .get(record)
            .and_then(|fields| fields.get(field))
            .map(String::as_str)
            .unwrap_or(field)
    }

    fn variant_name<'a>(&'a self, ty: &str, variant: &'a str) -> &'a str {
        self.variants
            .get(ty)
            .and_then(|variants| variants.get(variant))
            .map(String::as_str)
            .unwrap_or(variant)
    }

    fn record_field_type(&self, record: &str, field: &str) -> Option<TypeRef> {
        self.record_field_types
            .get(record)
            .and_then(|fields| fields.get(field))
            .cloned()
    }
}

fn minified_names(items: &[String]) -> BTreeMap<String, String> {
    let mut names = NameGenerator::new(reserved_identifiers());
    items
        .iter()
        .map(|item| (item.clone(), names.next()))
        .collect()
}

#[derive(Clone)]
struct NameGenerator {
    next: usize,
    reserved: BTreeSet<String>,
}

impl NameGenerator {
    fn new(reserved: BTreeSet<String>) -> Self {
        Self { next: 0, reserved }
    }

    fn next(&mut self) -> String {
        loop {
            let name = encode_name(self.next);
            self.next += 1;
            if !self.reserved.contains(&name) {
                self.reserved.insert(name.clone());
                return name;
            }
        }
    }
}

fn encode_name(mut value: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_";
    let mut chars = Vec::new();
    loop {
        chars.push(ALPHABET[value % ALPHABET.len()] as char);
        if value < ALPHABET.len() {
            break;
        }
        value = value / ALPHABET.len() - 1;
    }
    chars.iter().rev().collect()
}

fn reserved_identifiers() -> BTreeSet<String> {
    let words = [
        "use",
        "fn",
        "async",
        "await",
        "type",
        "enum",
        "error",
        "let",
        "return",
        "if",
        "else",
        "loop",
        "while",
        "test",
        "true",
        "false",
        "server",
        "tcp",
        "text",
        "json",
        "assert",
        "i8",
        "i16",
        "i32",
        "i64",
        "u8",
        "u16",
        "u32",
        "u64",
        "f32",
        "f64",
        "bool",
        "str",
        "void",
        "ptr",
        "Future",
        "TcpServer",
        "TcpConn",
        "io",
        "fs",
        "crypto",
        "env",
        "process",
        "cli",
        "path",
        "url",
        "time",
        "http",
        "heap",
        "A",
        "C",
        "E",
        "F",
        "H",
        "I",
        "J",
        "M",
        "N",
        "P",
        "S",
        "T",
        "U",
        "X",
    ];
    words
        .into_iter()
        .map(str::to_string)
        .chain(minified_std_call_aliases())
        .collect()
}

fn std_pack_roots() -> BTreeSet<&'static str> {
    [
        "io", "fs", "crypto", "env", "process", "cli", "json", "str", "path", "url", "time",
        "http", "tcp", "heap", "async",
    ]
    .into_iter()
    .collect()
}

fn minify_item(item: &Item, plan: &MinifyPlan, out: &mut String) {
    match item {
        Item::Function(function) => minify_function(function, plan, out),
        Item::Type(record) => {
            out.push('%');
            out.push_str(plan.type_name(&record.name));
            out.push('{');
            for (idx, field) in record.fields.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(plan.record_field_name(&record.name, &field.name));
                out.push(':');
                minify_type_ref(&field.ty, plan, out);
            }
            out.push('}');
        }
        Item::Enum(item) => minify_variant_item("enum", &item.name, &item.variants, plan, out),
        Item::Error(item) => minify_variant_item("error", &item.name, &item.variants, plan, out),
        Item::Server(server) => {
            out.push('&');
            if server.tls {
                out.push('!');
            }
            out.push_str(&server.port.to_string());
            out.push('{');
            for route in &server.routes {
                out.push_str(minify_http_method(&route.method));
                out.push_str(&route.path);
                out.push('>');
                minify_http_response(&route.response, out);
            }
            out.push('}');
        }
        Item::Tcp(tcp) => {
            out.push_str("&&");
            if tcp.tls {
                out.push('!');
            }
            out.push_str(&tcp.port.to_string());
            out.push('{');
            for route in &tcp.routes {
                match &route.pattern {
                    TcpPattern::Exact(value) => out.push_str(&quote_string(value)),
                    TcpPattern::Wildcard => out.push('*'),
                }
                out.push('>');
                out.push_str(&quote_string(&route.response));
            }
            out.push('}');
        }
        Item::Test(test) => {
            let mut ctx = MinifyCtx::new(plan);
            out.push('?');
            out.push_str(&quote_string(&test.name));
            minify_block(&test.body, &mut ctx, out);
        }
    }
}

fn minify_function(function: &Function, plan: &MinifyPlan, out: &mut String) {
    let mut ctx = MinifyCtx::new(plan);
    ctx.omit_trailing_return_zero = can_omit_trailing_return_zero(function);
    if is_min_main_i32(function) {
        ctx.implicit_lets = plan.globals.is_empty();
        let string_pool = ctx.install_string_pool(&function.body);
        minify_block_with_prefix(&function.body, &string_pool, &mut ctx, out);
        return;
    }
    if function.is_async {
        out.push_str("@@");
    } else {
        out.push('@');
    }
    out.push_str(plan.global_name(&function.name));
    out.push('(');
    for (idx, param) in function.params.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let name = ctx.local_name(&param.name).to_string();
        ctx.local_types.insert(param.name.clone(), param.ty.clone());
        out.push_str(&name);
        out.push(':');
        minify_type_ref(&param.ty, plan, out);
    }
    out.push_str("):");
    minify_type_ref(&function.ret, plan, out);
    let string_pool = ctx.install_string_pool(&function.body);
    minify_block_with_prefix(&function.body, &string_pool, &mut ctx, out);
}

fn is_min_main_i32(function: &Function) -> bool {
    function.name == "main"
        && !function.is_async
        && function.params.is_empty()
        && !function.ret.array
        && !function.ret.optional
        && function.ret.name == "i32"
        && function.ret.args.is_empty()
}

fn can_omit_trailing_return_zero(function: &Function) -> bool {
    !function.is_async
        && !function.ret.array
        && !function.ret.optional
        && function.ret.name == "i32"
        && function.ret.args.is_empty()
}

fn minify_variant_item(
    kind: &str,
    name: &str,
    variants: &[String],
    plan: &MinifyPlan,
    out: &mut String,
) {
    if kind == "enum" {
        out.push_str("%%");
    } else {
        out.push_str("%!");
    }
    out.push_str(plan.type_name(name));
    out.push('{');
    for (idx, variant) in variants.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(plan.variant_name(name, variant));
    }
    out.push('}');
}

#[derive(Clone)]
struct MinifyCtx<'a> {
    plan: &'a MinifyPlan,
    local_names: BTreeMap<String, String>,
    local_types: BTreeMap<String, TypeRef>,
    string_names: BTreeMap<String, String>,
    names: NameGenerator,
    implicit_lets: bool,
    omit_trailing_return_zero: bool,
}

impl<'a> MinifyCtx<'a> {
    fn new(plan: &'a MinifyPlan) -> Self {
        Self {
            plan,
            local_names: BTreeMap::new(),
            local_types: BTreeMap::new(),
            string_names: BTreeMap::new(),
            names: NameGenerator::new(plan.global_reserved.clone()),
            implicit_lets: false,
            omit_trailing_return_zero: false,
        }
    }

    fn local_name(&mut self, name: &str) -> &str {
        self.local_names
            .entry(name.to_string())
            .or_insert_with(|| self.names.next())
    }

    fn mapped_ident<'b>(&'b self, name: &'b str) -> &'b str {
        self.local_names
            .get(name)
            .map(String::as_str)
            .unwrap_or_else(|| self.plan.global_name(name))
    }

    fn mapped_string<'b>(&'b self, value: &'b str) -> Option<&'b str> {
        self.string_names.get(value).map(String::as_str)
    }

    fn install_string_pool(&mut self, block: &Block) -> Vec<(String, String)> {
        let mut counts = BTreeMap::new();
        collect_string_counts_block(block, &mut counts);
        let mut candidates = counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .collect::<Vec<_>>();
        candidates.sort_by(|(left, left_count), (right, right_count)| {
            let left_score = quote_string(left).len() * *left_count;
            let right_score = quote_string(right).len() * *right_count;
            right_score
                .cmp(&left_score)
                .then_with(|| right.len().cmp(&left.len()))
                .then_with(|| left.cmp(right))
        });

        let mut declarations = Vec::new();
        for (value, count) in candidates {
            let mut trial = self.names.clone();
            let name = trial.next();
            let quoted_len = quote_string(&value).len();
            let name_len = name.len();
            let declaration_len = 1 + name_len + 1 + quoted_len;
            let direct_len = quoted_len * count;
            let pooled_len = declaration_len + (name_len * count);
            if pooled_len < direct_len {
                self.names = trial;
                self.string_names.insert(value.clone(), name.clone());
                declarations.push((name, value));
            }
        }
        declarations
    }

    fn infer_expr_type(&self, expr: &Expr) -> Option<TypeRef> {
        match expr {
            Expr::Bool(_, _) => Some(TypeRef::simple("bool")),
            Expr::Int(_, _) => Some(TypeRef::simple("i32")),
            Expr::Float(_, _) => Some(TypeRef::simple("f64")),
            Expr::Str(_, _) => Some(TypeRef::simple("str")),
            Expr::Ident(name, _) => self.local_types.get(name).cloned(),
            Expr::RecordInit { name, .. } => Some(TypeRef::simple(name)),
            Expr::Member { object, field, .. } => {
                if let Expr::Ident(root, _) = object.as_ref() {
                    if let Some(root_ty) = self.local_types.get(root) {
                        return self.plan.record_field_type(&root_ty.name, field);
                    }
                    if self.plan.variants.contains_key(root) {
                        return Some(TypeRef::simple(root));
                    }
                }
                None
            }
            Expr::Call { callee, .. } => {
                let Expr::Ident(name, _) = callee.as_ref() else {
                    return None;
                };
                let (is_async, ret) = self.plan.function_returns.get(name)?;
                if *is_async {
                    let mut future = TypeRef::simple("Future");
                    future.args.push(ret.clone());
                    Some(future)
                } else {
                    Some(ret.clone())
                }
            }
            Expr::Binary { op, .. } => Some(TypeRef::simple(match op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::And
                | BinaryOp::Or => "bool",
                _ => "i32",
            })),
            Expr::Unary { op, expr, .. } => {
                if matches!(op, UnaryOp::Not) {
                    Some(TypeRef::simple("bool"))
                } else {
                    self.infer_expr_type(expr)
                }
            }
            Expr::Await { expr, .. } => {
                let ty = self.infer_expr_type(expr)?;
                if ty.name == "Future" {
                    ty.args.first().cloned()
                } else {
                    None
                }
            }
        }
    }
}

fn collect_string_counts_block(block: &Block, counts: &mut BTreeMap<String, usize>) {
    for stmt in &block.stmts {
        collect_string_counts_stmt(stmt, counts);
    }
}

fn collect_string_counts_stmt(stmt: &Stmt, counts: &mut BTreeMap<String, usize>) {
    match stmt {
        Stmt::Let { expr, .. } | Stmt::Expr { expr, .. } | Stmt::Assert { expr, .. } => {
            collect_string_counts_expr(expr, counts);
        }
        Stmt::Assign { target, expr, .. } => {
            collect_string_counts_expr(target, counts);
            collect_string_counts_expr(expr, counts);
        }
        Stmt::Return { expr, .. } => {
            if let Some(expr) = expr {
                collect_string_counts_expr(expr, counts);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            collect_string_counts_expr(cond, counts);
            collect_string_counts_block(then_block, counts);
            if let Some(else_block) = else_block {
                collect_string_counts_block(else_block, counts);
            }
        }
        Stmt::Loop { body, .. } => collect_string_counts_block(body, counts),
        Stmt::While { cond, body, .. } => {
            collect_string_counts_expr(cond, counts);
            collect_string_counts_block(body, counts);
        }
    }
}

fn collect_string_counts_expr(expr: &Expr, counts: &mut BTreeMap<String, usize>) {
    match expr {
        Expr::Str(value, _) => {
            *counts.entry(value.clone()).or_insert(0) += 1;
        }
        Expr::Member { object, .. } => collect_string_counts_expr(object, counts),
        Expr::Call { callee, args, .. } => {
            collect_string_counts_expr(callee, counts);
            for arg in args {
                collect_string_counts_expr(arg, counts);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_string_counts_expr(left, counts);
            collect_string_counts_expr(right, counts);
        }
        Expr::Unary { expr, .. } | Expr::Await { expr, .. } => {
            collect_string_counts_expr(expr, counts);
        }
        Expr::RecordInit { fields, .. } => {
            for (_, expr) in fields {
                collect_string_counts_expr(expr, counts);
            }
        }
        Expr::Int(_, _) | Expr::Float(_, _) | Expr::Bool(_, _) | Expr::Ident(_, _) => {}
    }
}

fn minify_block_with_prefix(
    block: &Block,
    prefix: &[(String, String)],
    ctx: &mut MinifyCtx<'_>,
    out: &mut String,
) {
    out.push('{');
    let mut previous = String::new();
    if prefix.len() >= 3 {
        let text =
            implicit_string_pool_text(prefix).unwrap_or_else(|| named_string_pool_text(prefix));
        push_minified_stmt(&mut previous, &text, out);
    } else {
        for (name, value) in prefix {
            let text = if ctx.implicit_lets {
                format!("${}", quote_string(value))
            } else {
                format!("${}={}", name, quote_string(value))
            };
            push_minified_stmt(&mut previous, &text, out);
        }
    }
    let skipped = if prefix.is_empty() {
        minify_leading_string_pool(block, ctx, &mut previous, out)
    } else {
        0
    };
    let mut end = block.stmts.len();
    if ctx.omit_trailing_return_zero
        && end > skipped
        && is_trailing_return_zero(&block.stmts[end - 1])
    {
        end -= 1;
    }
    for stmt in block.stmts.iter().take(end).skip(skipped) {
        let mut text = String::new();
        minify_stmt(stmt, ctx, &mut text);
        push_minified_stmt(&mut previous, &text, out);
    }
    out.push('}');
}

fn minify_leading_string_pool(
    block: &Block,
    ctx: &mut MinifyCtx<'_>,
    previous: &mut String,
    out: &mut String,
) -> usize {
    let mut entries = Vec::new();
    for stmt in &block.stmts {
        let Stmt::Let {
            name,
            ty: None,
            expr: Expr::Str(value, _),
            ..
        } = stmt
        else {
            break;
        };
        entries.push((name, value));
    }
    if entries.len() < 3 {
        return 0;
    }

    let entry_count = entries.len();
    let mut named_entries = Vec::new();
    for (name, value) in entries {
        let mapped = ctx.local_name(name).to_string();
        ctx.local_types
            .insert((*name).clone(), TypeRef::simple("str"));
        named_entries.push((mapped, (*value).clone()));
    }
    let text = implicit_string_pool_text(&named_entries)
        .unwrap_or_else(|| named_string_pool_text(&named_entries));
    push_minified_stmt(previous, &text, out);
    entry_count
}

fn named_string_pool_text(entries: &[(String, String)]) -> String {
    let mut text = String::from("#(");
    for (index, (name, value)) in entries.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(name);
        text.push_str(&quote_string(value));
    }
    text.push(')');
    text
}

fn implicit_string_pool_text(entries: &[(String, String)]) -> Option<String> {
    let mut names = NameGenerator::new(implicit_string_pool_reserved());
    let mut text = String::from("#(");
    for (index, (name, value)) in entries.iter().enumerate() {
        if *name != names.next() {
            return None;
        }
        if index > 0 {
            text.push(',');
        }
        text.push_str(&quote_string(value));
    }
    text.push(')');
    Some(text)
}

fn implicit_string_pool_reserved() -> BTreeSet<String> {
    let mut reserved = reserved_identifiers();
    reserved.extend(std_pack_roots().into_iter().map(str::to_string));
    reserved.insert("main".to_string());
    reserved
}

fn minify_block(block: &Block, ctx: &mut MinifyCtx<'_>, out: &mut String) {
    minify_block_with_prefix(block, &[], ctx, out);
}

fn is_trailing_return_zero(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Return {
            expr: Some(Expr::Int(0, _)),
            ..
        }
    )
}

fn push_minified_stmt(previous: &mut String, next: &str, out: &mut String) {
    if !previous.is_empty() && needs_minified_separator(previous, next) {
        out.push(' ');
    }
    out.push_str(next);
    previous.clear();
    previous.push_str(next);
}

fn needs_minified_separator(previous: &str, next: &str) -> bool {
    let Some(prev) = previous.chars().last() else {
        return false;
    };
    let Some(next) = next.chars().next() else {
        return false;
    };
    if is_min_ident_continue(prev) && is_min_ident_continue(next) {
        return true;
    }
    if matches!(next, '~' | '!' | '@') && is_min_postfix_lhs_end(prev) {
        return true;
    }
    matches!(
        (prev, next),
        ('-', '>')
            | ('=', '>')
            | ('=', '=')
            | ('!', '=')
            | ('<', '=')
            | ('>', '=')
            | ('&', '&')
            | ('|', '|')
            | ('/', '/')
    )
}

fn is_min_postfix_lhs_end(ch: char) -> bool {
    is_min_ident_continue(ch) || matches!(ch, '"' | ')' | ']' | '}' | '!' | '@')
}

fn is_min_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn minify_stmt(stmt: &Stmt, ctx: &mut MinifyCtx<'_>, out: &mut String) {
    match stmt {
        Stmt::Let { name, ty, expr, .. } => {
            let inferred = ty.clone().or_else(|| ctx.infer_expr_type(expr));
            let mapped = ctx.local_name(name).to_string();
            let expr_text = minify_expr(expr, ctx);
            out.push('$');
            if !ctx.implicit_lets || ty.is_some() {
                out.push_str(&mapped);
            }
            if let Some(ty) = ty {
                out.push(':');
                minify_type_ref(ty, ctx.plan, out);
            }
            if !ctx.implicit_lets || ty.is_some() || !can_omit_implicit_let_equals(&expr_text) {
                out.push('=');
            }
            out.push_str(&expr_text);
            if let Some(ty) = inferred {
                ctx.local_types.insert(name.clone(), ty);
            }
        }
        Stmt::Assign { target, expr, .. } => {
            out.push_str(&minify_expr(target, ctx));
            out.push('=');
            out.push_str(&minify_expr(expr, ctx));
        }
        Stmt::Return { expr, .. } => {
            out.push('^');
            if let Some(expr) = expr {
                out.push_str(&minify_expr(expr, ctx));
            }
        }
        Stmt::Expr { expr, .. } => {
            if minify_semicolon_stmt(expr, ctx, out) {
                return;
            }
            out.push_str(&minify_expr(expr, ctx));
        }
        Stmt::Assert { expr, .. } => {
            out.push(':');
            out.push_str(&minify_expr(expr, ctx));
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            out.push('?');
            out.push_str(&minify_expr(cond, ctx));
            out.push_str(&minify_isolated_block(then_block, ctx));
            if let Some(else_block) = else_block {
                out.push('|');
                out.push_str(&minify_isolated_block(else_block, ctx));
            }
        }
        Stmt::Loop { body, .. } => {
            out.push('~');
            let omit_trailing_return_zero = ctx.omit_trailing_return_zero;
            ctx.omit_trailing_return_zero = false;
            minify_block(body, ctx, out);
            ctx.omit_trailing_return_zero = omit_trailing_return_zero;
        }
        Stmt::While { cond, body, .. } => {
            let cond = minify_expr(cond, ctx);
            if cond.starts_with('{') {
                out.push_str("~(");
                out.push_str(&cond);
                out.push(')');
            } else {
                out.push('~');
                out.push_str(&cond);
            }
            let omit_trailing_return_zero = ctx.omit_trailing_return_zero;
            ctx.omit_trailing_return_zero = false;
            minify_block(body, ctx, out);
            ctx.omit_trailing_return_zero = omit_trailing_return_zero;
        }
    }
}

fn minify_semicolon_stmt(expr: &Expr, ctx: &mut MinifyCtx<'_>, out: &mut String) -> bool {
    let Expr::Call { callee, args, .. } = expr else {
        return false;
    };
    match (expr_path(callee).as_deref(), args.as_slice()) {
        (Some("io.println"), [arg]) => {
            out.push(';');
            out.push_str(&minify_expr(arg, ctx));
            true
        }
        (Some("fs.write_text"), [path, value]) => {
            out.push(';');
            out.push_str(&minify_expr(path, ctx));
            out.push(',');
            out.push_str(&minify_expr(value, ctx));
            true
        }
        (Some("fs.write_json_atomic"), [path, value]) => {
            out.push_str(";;");
            out.push_str(&minify_expr(path, ctx));
            out.push(',');
            out.push_str(&minify_expr(value, ctx));
            true
        }
        (Some("fs.mkdir"), [path]) => {
            out.push_str(";+");
            out.push_str(&minify_expr(path, ctx));
            true
        }
        _ => false,
    }
}

fn minify_isolated_block(block: &Block, ctx: &MinifyCtx<'_>) -> String {
    let mut implicit_ctx = ctx.clone();
    implicit_ctx.omit_trailing_return_zero = false;
    let mut implicit = String::new();
    minify_block(block, &mut implicit_ctx, &mut implicit);
    if !ctx.implicit_lets {
        return implicit;
    }

    let mut explicit_ctx = ctx.clone();
    explicit_ctx.implicit_lets = false;
    explicit_ctx.omit_trailing_return_zero = false;
    let mut explicit = String::new();
    minify_block(block, &mut explicit_ctx, &mut explicit);
    if explicit.len() < implicit.len() {
        explicit
    } else {
        implicit
    }
}

fn can_omit_implicit_let_equals(expr_text: &str) -> bool {
    let Some((_, first)) = expr_text.char_indices().next() else {
        return false;
    };
    if !is_min_ident_start(first) {
        return true;
    }
    for (_, ch) in expr_text.char_indices().skip(1) {
        if !is_min_ident_continue(ch) {
            return !matches!(ch, ':' | '=');
        }
    }
    true
}

fn is_min_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn minify_http_response(response: &HttpResponse, out: &mut String) {
    match response {
        HttpResponse::Text(value) => out.push_str(&quote_string(value)),
        HttpResponse::Json(fields) => {
            out.push_str("#{");
            for (idx, (key, value)) in fields.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(key);
                out.push(':');
                out.push_str(&minify_http_expr(value));
            }
            out.push('}');
        }
        HttpResponse::StreamBody => out.push('~'),
    }
}

fn minify_http_expr(expr: &Expr) -> String {
    match expr {
        Expr::Bool(value, _) => minify_bool(*value).to_string(),
        _ => compact_expr(expr),
    }
}

fn minify_bool(value: bool) -> &'static str {
    if value {
        "!1"
    } else {
        "!0"
    }
}

fn minify_http_method(method: &HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "G",
        HttpMethod::Post => "P",
        HttpMethod::Put => "U",
        HttpMethod::Patch => "A",
        HttpMethod::Delete => "D",
    }
}

fn compact_expr(expr: &Expr) -> String {
    compact_expr_with_parent(expr, 0, ExprSide::Left)
}

fn minify_expr(expr: &Expr, ctx: &MinifyCtx<'_>) -> String {
    minify_expr_with_parent(expr, 0, ExprSide::Left, ctx)
}

fn compact_expr_with_parent(expr: &Expr, parent_prec: u8, side: ExprSide) -> String {
    let own_prec = expr_precedence(expr);
    let mut text = match expr {
        Expr::Int(value, _) => value.to_string(),
        Expr::Float(value, _) => compact_float(*value),
        Expr::Bool(value, _) => minify_bool(*value).to_string(),
        Expr::Str(value, _) => quote_string(value),
        Expr::Ident(value, _) => value.clone(),
        Expr::Member { object, field, .. } => {
            format!(
                "{}.{}",
                compact_expr_with_parent(object, own_prec, ExprSide::Left),
                field
            )
        }
        Expr::Call { callee, args, .. } => format!(
            "{}({})",
            compact_expr_with_parent(callee, own_prec, ExprSide::Left),
            args.iter().map(compact_expr).collect::<Vec<_>>().join(",")
        ),
        Expr::Binary {
            op, left, right, ..
        } => {
            let prec = binary_precedence(*op);
            let left_text = compact_expr_with_parent(left, prec, ExprSide::Left);
            let op_text = minified_binary_op(*op);
            let right_text = compact_expr_with_parent(right, prec, ExprSide::Right);
            let separator = if left_text.ends_with('!') && op_text == ":" {
                " "
            } else {
                ""
            };
            format!("{}{}{}{}", left_text, separator, op_text, right_text)
        }
        Expr::Unary { op, expr, .. } => match op {
            UnaryOp::Not => format!(
                "!{}",
                compact_expr_with_parent(expr, own_prec, ExprSide::Right)
            ),
            UnaryOp::Neg => format!(
                "-{}",
                compact_expr_with_parent(expr, own_prec, ExprSide::Right)
            ),
        },
        Expr::Await { expr, .. } => format!("@({})", compact_expr(expr)),
        Expr::RecordInit { name, fields, .. } => {
            let body = fields
                .iter()
                .map(|(field, expr)| format!("{}:{}", field, compact_expr(expr)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{}{{{}}}", name, body)
        }
    };
    if needs_parens(own_prec, parent_prec, side) {
        text.insert(0, '(');
        text.push(')');
    }
    text
}

fn minify_expr_with_parent(
    expr: &Expr,
    parent_prec: u8,
    side: ExprSide,
    ctx: &MinifyCtx<'_>,
) -> String {
    let own_prec = expr_precedence(expr);
    let mut text = match expr {
        Expr::Int(value, _) => value.to_string(),
        Expr::Float(value, _) => compact_float(*value),
        Expr::Bool(value, _) => minify_bool(*value).to_string(),
        Expr::Str(value, _) => ctx
            .mapped_string(value)
            .map(str::to_string)
            .unwrap_or_else(|| quote_string(value)),
        Expr::Ident(value, _) => ctx.mapped_ident(value).to_string(),
        Expr::Member { object, field, .. } => minify_member_expr(object, field, own_prec, ctx),
        Expr::Call { callee, args, .. } => {
            if let Some(text) = minify_json_object_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_json_at_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_json_query_postfix_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_str_contains_postfix_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_str_len_postfix_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_fs_is_file_postfix_expr(callee, args, ctx) {
                text
            } else if let Some(text) = minify_zero_arg_std_call_expr(callee, args) {
                text
            } else {
                format!(
                    "{}({})",
                    minify_call_callee(callee, own_prec, ctx),
                    args.iter()
                        .map(|arg| minify_expr(arg, ctx))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
        Expr::Binary {
            op, left, right, ..
        } => {
            let prec = binary_precedence(*op);
            let left_text = minify_expr_with_parent(left, prec, ExprSide::Left, ctx);
            let op_text = minified_binary_op(*op);
            let right_text = minify_expr_with_parent(right, prec, ExprSide::Right, ctx);
            let separator = if left_text.ends_with('!') && op_text == ":" {
                " "
            } else {
                ""
            };
            format!("{}{}{}{}", left_text, separator, op_text, right_text)
        }
        Expr::Unary { op, expr, .. } => match op {
            UnaryOp::Not => format!(
                "!{}",
                minify_expr_with_parent(expr, own_prec, ExprSide::Right, ctx)
            ),
            UnaryOp::Neg => format!(
                "-{}",
                minify_expr_with_parent(expr, own_prec, ExprSide::Right, ctx)
            ),
        },
        Expr::Await { expr, .. } => format!("@({})", minify_expr(expr, ctx)),
        Expr::RecordInit { name, fields, .. } => {
            let body = fields
                .iter()
                .map(|(field, expr)| {
                    format!(
                        "{}:{}",
                        ctx.plan.record_field_name(name, field),
                        minify_expr(expr, ctx)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{}{{{}}}", ctx.plan.type_name(name), body)
        }
    };
    if needs_parens(own_prec, parent_prec, side) {
        text.insert(0, '(');
        text.push(')');
    }
    text
}

fn minify_json_query_postfix_expr(
    callee: &Expr,
    args: &[Expr],
    ctx: &MinifyCtx<'_>,
) -> Option<String> {
    if args.len() != 2 || expr_precedence(&args[1]) < 7 {
        return None;
    }
    let op = match expr_path(callee).as_deref() {
        Some("json.query") => "`",
        Some("json.query_bool") => "'",
        Some("json.query_int") => "#",
        Some("json.query_len") => "\\",
        _ => return None,
    };
    Some(format!(
        "{}{}{}",
        minify_expr_with_parent(&args[0], 8, ExprSide::Left, ctx),
        op,
        minify_expr(&args[1], ctx)
    ))
}

fn minify_str_contains_postfix_expr(
    callee: &Expr,
    args: &[Expr],
    ctx: &MinifyCtx<'_>,
) -> Option<String> {
    if expr_path(callee).as_deref() != Some("str.contains")
        || args.len() != 2
        || expr_precedence(&args[1]) < 7
    {
        return None;
    }
    Some(format!(
        "{}~{}",
        minify_expr_with_parent(&args[0], 8, ExprSide::Left, ctx),
        minify_expr(&args[1], ctx)
    ))
}

fn minify_str_len_postfix_expr(
    callee: &Expr,
    args: &[Expr],
    ctx: &MinifyCtx<'_>,
) -> Option<String> {
    if expr_path(callee).as_deref() != Some("str.len") || args.len() != 1 {
        return None;
    }
    Some(format!(
        "{}!",
        minify_expr_with_parent(&args[0], 8, ExprSide::Left, ctx)
    ))
}

fn minify_fs_is_file_postfix_expr(
    callee: &Expr,
    args: &[Expr],
    ctx: &MinifyCtx<'_>,
) -> Option<String> {
    if expr_path(callee).as_deref() != Some("fs.is_file") || args.len() != 1 {
        return None;
    }
    Some(format!(
        "{}@",
        minify_expr_with_parent(&args[0], 8, ExprSide::Left, ctx)
    ))
}

fn minify_zero_arg_std_call_expr(callee: &Expr, args: &[Expr]) -> Option<String> {
    if !args.is_empty() {
        return None;
    }
    let alias = minified_std_call_name(&expr_path(callee)?)?;
    alias
        .chars()
        .next()
        .filter(|ch| is_min_ident_start(*ch))
        .map(|_| alias)
}

fn minify_json_at_expr(callee: &Expr, args: &[Expr], ctx: &MinifyCtx<'_>) -> Option<String> {
    if expr_path(callee).as_deref() != Some("json.at") || args.len() != 3 {
        return None;
    }
    Some(format!(
        "{}[{},{}]",
        minify_expr_with_parent(&args[0], 8, ExprSide::Left, ctx),
        minify_expr(&args[1], ctx),
        minify_expr(&args[2], ctx)
    ))
}

fn minify_json_object_expr(callee: &Expr, args: &[Expr], ctx: &MinifyCtx<'_>) -> Option<String> {
    if expr_path(callee).as_deref() != Some("json.object") || args.len() != 1 {
        return None;
    }
    let Expr::Call {
        callee: pair_callee,
        args: pair_args,
        ..
    } = &args[0]
    else {
        return None;
    };
    if pair_args.len() != 2 || expr_precedence(&pair_args[0]) < 7 {
        return None;
    }
    let delimiter = match expr_path(pair_callee).as_deref() {
        Some("json.pair") => ':',
        Some("json.string_pair") => '=',
        _ => return None,
    };
    Some(format!(
        "{{{}{}{}}}",
        minify_expr(&pair_args[0], ctx),
        delimiter,
        minify_expr(&pair_args[1], ctx)
    ))
}

fn minify_call_callee(callee: &Expr, own_prec: u8, ctx: &MinifyCtx<'_>) -> String {
    if let Some(path) = expr_path(callee) {
        if let Some(alias) = minified_std_call_name(&path) {
            return alias;
        }
    }
    minify_expr_with_parent(callee, own_prec, ExprSide::Left, ctx)
}

fn expr_path(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Member { object, field, .. } => Some(format!("{}.{}", expr_path(object)?, field)),
        _ => None,
    }
}

fn minify_member_expr(object: &Expr, field: &str, own_prec: u8, ctx: &MinifyCtx<'_>) -> String {
    if let Expr::Ident(root, _) = object {
        if ctx.plan.pack_roots.contains(root) {
            return format!("{}.{}", root, field);
        }
        if let Some(root_ty) = ctx.local_types.get(root) {
            let object = ctx.mapped_ident(root);
            let field = ctx.plan.record_field_name(&root_ty.name, field);
            return format!("{}.{}", object, field);
        }
        if ctx.plan.variants.contains_key(root) {
            return format!(
                "{}.{}",
                ctx.plan.type_name(root),
                ctx.plan.variant_name(root, field)
            );
        }
    }
    format!(
        "{}.{}",
        minify_expr_with_parent(object, own_prec, ExprSide::Left, ctx),
        field
    )
}

#[derive(Clone, Copy)]
enum ExprSide {
    Left,
    Right,
}

fn needs_parens(own_prec: u8, parent_prec: u8, side: ExprSide) -> bool {
    own_prec > 0
        && parent_prec > 0
        && (own_prec < parent_prec || (own_prec == parent_prec && matches!(side, ExprSide::Right)))
}

fn expr_precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Binary { op, .. } => binary_precedence(*op),
        Expr::Unary { .. } | Expr::Await { .. } => 7,
        Expr::Member { .. } | Expr::Call { .. } => 8,
        _ => 9,
    }
}

fn binary_precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne => 3,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 4,
        BinaryOp::Add | BinaryOp::Sub => 5,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 6,
    }
}

fn minified_binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::And => "&",
        BinaryOp::Or => "|",
        BinaryOp::Eq => ":",
        BinaryOp::Ne => "!:",
        _ => op.as_str(),
    }
}

fn minify_type_ref(ty: &TypeRef, plan: &MinifyPlan, out: &mut String) {
    if ty.array {
        out.push('[');
        if let Some(inner) = ty.args.first() {
            minify_type_ref(inner, plan, out);
        }
        out.push(']');
    } else {
        if ty.args.is_empty() {
            if let Some(code) = minified_builtin_type(&ty.name) {
                out.push_str(code);
            } else {
                out.push_str(plan.type_name(&ty.name));
            }
        } else {
            out.push_str(plan.type_name(&ty.name));
        }
        if !ty.args.is_empty() {
            out.push('<');
            for (idx, arg) in ty.args.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                minify_type_ref(arg, plan, out);
            }
            out.push('>');
        }
    }
    if ty.optional {
        out.push('?');
    }
}

fn minified_builtin_type(name: &str) -> Option<&'static str> {
    match name {
        "i32" => Some("#"),
        "str" => Some("$"),
        _ => None,
    }
}

fn compact_float(value: f64) -> String {
    let text = value.to_string();
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{}.0", text)
    }
}

fn quote_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_core::SourceFile;
    use ax_parser::parse_source;
    use ax_semantic::check_program;

    fn min_roundtrip(source: &str) -> String {
        let source = SourceFile::new("test.ax", source);
        let program = parse_source(&source).expect("parse min source");
        check_program(&program).expect("check min source");
        let minified = minify_program(&program);
        let minified_source = SourceFile::new("min.ax", &minified);
        let reparsed = parse_source(&minified_source).expect("parse minified source");
        check_program(&reparsed).expect("check minified source");
        assert_eq!(minify_program(&reparsed), minified);
        minified
    }

    fn syntax_min_roundtrip(source: &str) -> String {
        let source = SourceFile::new("test.ax", source);
        let program = parse_source(&source).expect("parse min source");
        let minified = minify_program(&program);
        let minified_source = SourceFile::new("min.ax", &minified);
        let reparsed = parse_source(&minified_source).expect("parse minified source");
        assert_eq!(minify_program(&reparsed), minified);
        minified
    }

    #[test]
    fn public_formatters_emit_min_source() {
        let source = SourceFile::new("test.ax", "{;\"hello world\"}");
        let program = parse_source(&source).expect("parse source");
        assert_eq!(format_program(&program), "{;\"hello world\"}");
        assert_eq!(compact_program(&program), "{;\"hello world\"}");
        assert_eq!(minify_program(&program), "{;\"hello world\"}");
    }

    #[test]
    fn min_source_roundtrips_core_items() {
        let minified = min_roundtrip(
            "%a{a:#,b:$} %%b{a,b} %!c{a,b} @@d(e:#):#{^e+1} {$f=a{a:7,b:\"agent\"}$g=b.a?f.a>0{;f.b}|{^1}^0} &!3443{G/ping>\"pong\"P/echo>~G/state>#{ok:!1,value:1.0}} &&!3444{\"ping\">\"pong\"*>\"fallback\"} ?\"math\"{:1+2*3:7}",
        );

        assert!(minified.contains("%a{a:#,b:$}"));
        assert!(minified.contains("%%b{a,b}"));
        assert!(minified.contains("%!c{a,b}"));
        assert!(minified.contains("@@d(e:#):#{^e+1}"));
        assert!(minified.contains("&!3443{G/ping>\"pong\"P/echo>~G/state>#{ok:!1,value:1.0}}"));
        assert!(minified.contains("&&!3444{\"ping\">\"pong\"*>\"fallback\"}"));
        assert!(minified.contains("?\"math\"{:1+2*3:7}"));
    }

    #[test]
    fn min_source_roundtrips_external_pack_imports() {
        let minified = syntax_min_roundtrip("+acme.telemetry {telemetry.track()}");
        assert_eq!(minified, "+acme.telemetry {telemetry.track()}");
    }

    #[test]
    fn string_pool_and_semicolon_forms_are_canonical() {
        assert_eq!(
            min_roundtrip("{$\"repeat-value\";a;a}"),
            "{$\"repeat-value\";a;a}"
        );
        assert_eq!(
            min_roundtrip("{#(\"charlie\",\"alpha\",\"bravo\");b;b;c;c;a;a}"),
            "{#(\"charlie\",\"alpha\",\"bravo\");b;b;c;c;a;a}"
        );
        assert_eq!(
            min_roundtrip("{;\"x\";\"p\",\"v\";;\"j\",\"k\";+\"d\"}"),
            "{;\"x\";\"p\",\"v\";;\"j\",\"k\";+\"d\"}"
        );
    }

    #[test]
    fn implicit_let_and_control_forms_are_canonical() {
        assert_eq!(min_roundtrip("{$1$2^a+b}"), "{$1$2^a+b}");
        assert_eq!(min_roundtrip("{$1?a:1{$2^b}$3^b}"), "{$1?a:1{$2^b}$3^b}");
        assert_eq!(min_roundtrip("{~{$7^a}}"), "{~{$7^a}}");
        assert_eq!(min_roundtrip("{$1$=a:1?a{^0}^1}"), "{$1$=a:1?a{^0}^1}");
    }

    #[test]
    fn json_postfix_forms_are_canonical() {
        assert_eq!(
            min_roundtrip("{${\"items\":\"true\"}${\"name\"=\"agent\"}}"),
            "{${\"items\":\"true\"}${\"name\"=\"agent\"}}"
        );
        assert_eq!(
            min_roundtrip("{$\"items\"[\"tools\",0]}"),
            "{$\"items\"[\"tools\",0]}"
        );
        assert_eq!(
            min_roundtrip("{$\"doc\"$\"items\"$a`\"name\"$a'\"ok\"$a#\"count\"$a\\b$A0(a,b)}"),
            "{$\"doc\"$\"items\"$a`\"name\"$a'\"ok\"$a#\"count\"$a\\b$A0(a,b)}"
        );
    }

    #[test]
    fn string_and_file_postfix_forms_are_canonical() {
        assert_eq!(
            min_roundtrip("{$\"agent\"~\"g\"?a{^0}^1}"),
            "{$\"agent\"~\"g\"?a{^0}^1}"
        );
        assert_eq!(min_roundtrip("{$\"agent\"!^a}"), "{$\"agent\"!^a}");
        assert_eq!(
            min_roundtrip("{$\"agent\"!?a>0{^0}^a}"),
            "{$\"agent\"!?a>0{^0}^a}"
        );
        assert_eq!(
            min_roundtrip("{?\"agent\"! :5{^0}^1}"),
            "{?\"agent\"! :5{^0}^1}"
        );
        assert_eq!(min_roundtrip("{$\"agent\"! !!0^a}"), "{$\"agent\"! !!0^a}");
        assert_eq!(
            min_roundtrip("{$\"agent\"@?a{^0}^1}"),
            "{$\"agent\"@?a{^0}^1}"
        );
        assert_eq!(
            min_roundtrip("@@a():#{^1} {$b=\"agent\"@ @(a())}"),
            "@@a():#{^1} {$b=\"agent\"@ @(a())}"
        );
    }

    #[test]
    fn uses_one_byte_names_beyond_lowercase_alphabet() {
        let mut names = NameGenerator::new(reserved_identifiers());
        let generated = (0..30).map(|_| names.next()).collect::<Vec<_>>();

        assert_eq!(generated[0], "a");
        assert_eq!(generated[25], "z");
        assert_eq!(generated[26], "B");
        assert_eq!(generated[29], "K");
    }
}
