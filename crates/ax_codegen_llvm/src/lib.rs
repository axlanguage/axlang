use ax_ast::*;
use ax_core::Span;
use ax_diag::{AxResult, Diagnostic};
use ax_ir::IrProgram;
use ax_semantic::SemanticInfo;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub trait NativeBackend {
    fn build(
        &self,
        input: &Path,
        program: &Program,
        semantic: &SemanticInfo,
        output: &Path,
    ) -> AxResult<BuildArtifact>;
}

#[derive(Clone, Debug)]
pub struct LlvmBackend;

#[derive(Clone, Debug)]
pub struct CustomBackend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Llvm,
    Custom,
}

#[derive(Clone, Debug)]
pub struct BuildArtifact {
    pub backend: &'static str,
    pub ll_path: PathBuf,
    pub object_path: PathBuf,
    pub binary_path: PathBuf,
    pub output_path: PathBuf,
}

impl NativeBackend for LlvmBackend {
    fn build(
        &self,
        input: &Path,
        program: &Program,
        semantic: &SemanticInfo,
        output: &Path,
    ) -> AxResult<BuildArtifact> {
        build_program(input, program, semantic, output)
    }
}

impl NativeBackend for CustomBackend {
    fn build(
        &self,
        input: &Path,
        program: &Program,
        semantic: &SemanticInfo,
        output: &Path,
    ) -> AxResult<BuildArtifact> {
        build_program_custom(input, program, semantic, output)
    }
}

pub fn build_program_with_backend(
    input: &Path,
    program: &Program,
    semantic: &SemanticInfo,
    output: &Path,
    backend: BackendKind,
) -> AxResult<BuildArtifact> {
    match backend {
        BackendKind::Llvm => build_program(input, program, semantic, output),
        BackendKind::Custom => build_program_custom(input, program, semantic, output),
    }
}

pub fn build_program(
    input: &Path,
    program: &Program,
    semantic: &SemanticInfo,
    output: &Path,
) -> AxResult<BuildArtifact> {
    let ir = IrProgram::lower(program, semantic);
    let llvm = LlvmModule::new(&ir).generate()?;
    let out_dir = PathBuf::from(".ax-out");
    fs::create_dir_all(&out_dir).map_err(codegen_io)?;
    let base = output
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .or_else(|| {
            input
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "ax-out".to_string());
    let ll_path = out_dir.join(format!("{}.ll", base));
    let object_path = out_dir.join(format!("{}.o", base));
    let binary_path = output.to_path_buf();
    fs::write(&ll_path, llvm).map_err(codegen_io)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(codegen_io)?;
        }
    }

    run_command(
        Command::new("clang")
            .arg("-Wno-override-module")
            .arg("-O2")
            .arg("-ffunction-sections")
            .arg("-fdata-sections")
            .arg("-c")
            .arg(&ll_path)
            .arg("-o")
            .arg(&object_path),
        "AX_CODEGEN_ERROR",
    )?;

    let runtime_dir = runtime_dir()?;
    let mut link_command = Command::new("clang");
    link_command
        .arg("-O2")
        .arg("-ffunction-sections")
        .arg("-fdata-sections")
        .arg(&object_path)
        .arg("-I")
        .arg(&runtime_dir);
    add_runtime_sources(&mut link_command, &runtime_dir, program, semantic);
    add_link_dead_strip_flags(&mut link_command);
    if program_needs_http_tls(program) {
        link_command.arg(runtime_dir.join("net_http_tls.c"));
    }
    if program_needs_tcp_tls(program) {
        link_command.arg(runtime_dir.join("net_tcp_tls.c"));
    }
    for native_source in linked_native_sources(semantic) {
        link_command.arg(native_source);
    }
    if program_needs_tls(program) {
        add_openssl_flags(&mut link_command)?;
    }
    link_command.arg("-o").arg(&binary_path);
    run_command(&mut link_command, "AX_LINK_ERROR")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(output).map_err(codegen_io)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output, perms).map_err(codegen_io)?;
    }

    Ok(BuildArtifact {
        backend: "llvm",
        ll_path,
        object_path,
        binary_path,
        output_path: output.to_path_buf(),
    })
}

pub fn build_program_custom(
    input: &Path,
    program: &Program,
    semantic: &SemanticInfo,
    output: &Path,
) -> AxResult<BuildArtifact> {
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        return Err(custom_unsupported(
            "custom backend currently supports macOS arm64 hosts",
            Span::default(),
        ));
    }
    let asm = CustomAsmModule::new(program, semantic).generate()?;
    let out_dir = PathBuf::from(".ax-out");
    fs::create_dir_all(&out_dir).map_err(codegen_io)?;
    let base = output
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .or_else(|| {
            input
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "ax-out".to_string());
    let asm_path = out_dir.join(format!("{}.custom.s", base));
    let object_path = out_dir.join(format!("{}.custom.o", base));
    let binary_path = output.to_path_buf();
    fs::write(&asm_path, asm).map_err(codegen_io)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(codegen_io)?;
        }
    }

    run_command(
        Command::new("clang")
            .arg("-O2")
            .arg("-ffunction-sections")
            .arg("-fdata-sections")
            .arg("-c")
            .arg(&asm_path)
            .arg("-o")
            .arg(&object_path),
        "AX_CODEGEN_ERROR",
    )?;

    let runtime_dir = runtime_dir()?;
    let mut link_command = Command::new("clang");
    link_command
        .arg("-O2")
        .arg("-ffunction-sections")
        .arg("-fdata-sections")
        .arg(&object_path)
        .arg("-I")
        .arg(&runtime_dir);
    add_runtime_sources(&mut link_command, &runtime_dir, program, semantic);
    add_link_dead_strip_flags(&mut link_command);
    link_command.arg("-o").arg(&binary_path);
    run_command(&mut link_command, "AX_LINK_ERROR")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(output).map_err(codegen_io)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output, perms).map_err(codegen_io)?;
    }

    Ok(BuildArtifact {
        backend: "custom",
        ll_path: asm_path,
        object_path,
        binary_path,
        output_path: output.to_path_buf(),
    })
}

fn run_command(command: &mut Command, code: &'static str) -> AxResult<()> {
    let output = command.output().map_err(codegen_io)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(Diagnostic::error(
        code,
        stderr.trim().to_string(),
        ax_core::Span::default(),
    ))
}

fn add_openssl_flags(command: &mut Command) -> AxResult<()> {
    let output = Command::new("pkg-config")
        .args(["--cflags", "--libs", "openssl"])
        .output()
        .map_err(|err| {
            Diagnostic::error(
                "AX_LINK_ERROR",
                format!("TLS servers require OpenSSL pkg-config metadata: {}", err),
                ax_core::Span::default(),
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Diagnostic::error(
            "AX_LINK_ERROR",
            if stderr.is_empty() {
                "TLS servers require OpenSSL pkg-config metadata".to_string()
            } else {
                stderr
            },
            ax_core::Span::default(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    command.args(stdout.split_whitespace());
    Ok(())
}

fn codegen_io(err: std::io::Error) -> Diagnostic {
    Diagnostic::error(
        "AX_CODEGEN_ERROR",
        err.to_string(),
        ax_core::Span::default(),
    )
}

fn custom_unsupported(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error("AX_CUSTOM_BACKEND_UNSUPPORTED", message, span)
        .help("use the default LLVM backend or restrict the program to scalar functions and std.io output")
}

fn runtime_dir() -> AxResult<PathBuf> {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ax_runtime/src");
    if source_dir.join("ax_runtime.h").exists() {
        return Ok(source_dir);
    }
    embedded_runtime_dir()
}

fn embedded_runtime_dir() -> AxResult<PathBuf> {
    let runtime_dir = PathBuf::from(".ax-out").join("runtime");
    fs::create_dir_all(&runtime_dir).map_err(codegen_io)?;
    for (name, text) in EMBEDDED_RUNTIME_SOURCES {
        fs::write(runtime_dir.join(name), text).map_err(codegen_io)?;
    }
    Ok(runtime_dir)
}

const EMBEDDED_RUNTIME_SOURCES: &[(&str, &str)] = &[
    ("async.c", include_str!("../../ax_runtime/src/async.c")),
    (
        "ax_runtime.h",
        include_str!("../../ax_runtime/src/ax_runtime.h"),
    ),
    ("core.c", include_str!("../../ax_runtime/src/core.c")),
    ("crypto.c", include_str!("../../ax_runtime/src/crypto.c")),
    ("io.c", include_str!("../../ax_runtime/src/io.c")),
    ("map.c", include_str!("../../ax_runtime/src/map.c")),
    (
        "net_http.c",
        include_str!("../../ax_runtime/src/net_http.c"),
    ),
    (
        "net_http_tls.c",
        include_str!("../../ax_runtime/src/net_http_tls.c"),
    ),
    ("net_tcp.c", include_str!("../../ax_runtime/src/net_tcp.c")),
    (
        "net_tcp_tls.c",
        include_str!("../../ax_runtime/src/net_tcp_tls.c"),
    ),
];

struct CustomAsmModule<'a> {
    program: &'a Program,
    semantic: &'a SemanticInfo,
    strings: Vec<(String, String)>,
    string_count: usize,
}

#[derive(Clone)]
struct AsmLocal {
    offset: i32,
    ty: TypeRef,
}

struct CustomFunctionAsm<'m, 'a> {
    module: &'m mut CustomAsmModule<'a>,
    function: &'a Function,
    locals: BTreeMap<String, AsmLocal>,
    next_slot: i32,
    stack_size: i32,
    return_label: String,
}

impl<'a> CustomAsmModule<'a> {
    fn new(program: &'a Program, semantic: &'a SemanticInfo) -> Self {
        Self {
            program,
            semantic,
            strings: Vec::new(),
            string_count: 0,
        }
    }

    fn generate(&mut self) -> AxResult<String> {
        for item in &self.program.items {
            match item {
                Item::Server(server) => {
                    return Err(custom_unsupported(
                        "custom backend does not support HTTP server blocks yet",
                        server.span,
                    ))
                }
                Item::Tcp(tcp) => {
                    return Err(custom_unsupported(
                        "custom backend does not support TCP blocks yet",
                        tcp.span,
                    ))
                }
                _ => {}
            }
        }

        let mut out = String::from(".section __TEXT,__text,regular,pure_instructions\n");
        for item in &self.program.items {
            if let Item::Function(function) = item {
                if function.is_async {
                    return Err(custom_unsupported(
                        "custom backend does not support async functions yet",
                        function.span,
                    ));
                }
                out.push_str(&self.generate_function(function)?);
            }
        }
        if !self.strings.is_empty() {
            out.push_str("\n.section __TEXT,__cstring,cstring_literals\n");
            for (label, value) in &self.strings {
                out.push_str(&format!("{}:\n  .asciz \"{}\"\n", label, asm_escape(value)));
            }
        }
        Ok(out)
    }

    fn generate_function(&mut self, function: &'a Function) -> AxResult<String> {
        let mut codegen = CustomFunctionAsm::new(self, function);
        codegen.generate()
    }

    fn add_string(&mut self, value: &str) -> String {
        let label = format!("L_ax_str_{}", self.string_count);
        self.string_count += 1;
        self.strings.push((label.clone(), value.to_string()));
        label
    }

    fn variant_index(&self, name: &str, variant: &str) -> Option<i32> {
        self.semantic
            .enums
            .get(name)
            .or_else(|| self.semantic.errors.get(name))
            .and_then(|item| item.variants.get(variant).copied())
    }
}

impl<'m, 'a> CustomFunctionAsm<'m, 'a> {
    fn new(module: &'m mut CustomAsmModule<'a>, function: &'a Function) -> Self {
        let slots = function.params.len() + count_top_level_lets(&function.body);
        let stack_size = align16((slots as i32) * 8);
        Self {
            module,
            function,
            locals: BTreeMap::new(),
            next_slot: 1,
            stack_size,
            return_label: format!("L_{}_return", sanitize_ident(&function.name)),
        }
    }

    fn generate(&mut self) -> AxResult<String> {
        if self.function.params.len() > 8 {
            return Err(custom_unsupported(
                "custom backend supports up to 8 function parameters",
                self.function.span,
            ));
        }
        let mut out = String::new();
        out.push_str(&format!(
            "\n.globl {}\n.p2align 2\n{}:\n",
            asm_symbol(&self.function.name),
            asm_symbol(&self.function.name)
        ));
        out.push_str("  stp x29, x30, [sp, #-16]!\n  mov x29, sp\n");
        if self.stack_size > 0 {
            out.push_str(&format!("  sub sp, sp, #{}\n", self.stack_size));
        }
        for (idx, param) in self.function.params.iter().enumerate() {
            let offset = self.alloc_local(param.name.clone(), param.ty.clone());
            out.push_str(&format!("  stur x{}, [x29, #-{}]\n", idx, offset));
        }
        for stmt in &self.function.body.stmts {
            self.emit_stmt(stmt, &mut out)?;
        }
        out.push_str("  mov w0, #0\n");
        out.push_str(&format!("  b {}\n", self.return_label));
        out.push_str(&format!("{}:\n", self.return_label));
        if self.stack_size > 0 {
            out.push_str(&format!("  add sp, sp, #{}\n", self.stack_size));
        }
        out.push_str("  ldp x29, x30, [sp], #16\n  ret\n");
        Ok(out)
    }

    fn emit_stmt(&mut self, stmt: &Stmt, out: &mut String) -> AxResult<()> {
        match stmt {
            Stmt::Let { name, ty, expr, .. } => {
                let actual = self.emit_expr(expr, out)?;
                let local_ty = ty.clone().unwrap_or(actual);
                let offset = self.alloc_local(name.clone(), local_ty);
                out.push_str(&format!("  stur x0, [x29, #-{}]\n", offset));
                Ok(())
            }
            Stmt::Return { expr, span } => {
                if let Some(expr) = expr {
                    self.emit_expr(expr, out)?;
                } else if self.function.ret.name != "void" {
                    return Err(custom_unsupported(
                        "custom backend requires a value for non-void returns",
                        *span,
                    ));
                }
                out.push_str(&format!("  b {}\n", self.return_label));
                Ok(())
            }
            Stmt::Expr { expr, .. } => {
                self.emit_expr(expr, out)?;
                Ok(())
            }
            Stmt::Assign { span, .. }
            | Stmt::If { span, .. }
            | Stmt::Loop { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Assert { span, .. } => Err(custom_unsupported(
                "custom backend currently supports let, expression, and return statements",
                *span,
            )),
        }
    }

    fn emit_expr(&mut self, expr: &Expr, out: &mut String) -> AxResult<TypeRef> {
        match expr {
            Expr::Int(value, span) => {
                if *value < 0 || *value > 65535 {
                    return Err(custom_unsupported(
                        "custom backend integer literals currently support 0..65535",
                        *span,
                    ));
                }
                out.push_str(&format!("  mov w0, #{}\n", value));
                Ok(TypeRef::simple("i32"))
            }
            Expr::Bool(value, _) => {
                out.push_str(&format!("  mov w0, #{}\n", if *value { 1 } else { 0 }));
                Ok(TypeRef::simple("bool"))
            }
            Expr::Str(value, _) => {
                let label = self.module.add_string(value);
                out.push_str(&format!("  adrp x0, {}@PAGE\n", label));
                out.push_str(&format!("  add x0, x0, {}@PAGEOFF\n", label));
                Ok(TypeRef::simple("str"))
            }
            Expr::Ident(name, span) => {
                let local = self.locals.get(name).ok_or_else(|| {
                    Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("unknown local `{}`", name),
                        *span,
                    )
                })?;
                if matches!(local.ty.name.as_str(), "str" | "ptr" | "Future") {
                    out.push_str(&format!("  ldur x0, [x29, #-{}]\n", local.offset));
                } else {
                    out.push_str(&format!("  ldur w0, [x29, #-{}]\n", local.offset));
                }
                Ok(local.ty.clone())
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                self.emit_expr(left, out)?;
                out.push_str("  sub sp, sp, #16\n  str x0, [sp]\n");
                self.emit_expr(right, out)?;
                out.push_str("  ldr x9, [sp]\n  add sp, sp, #16\n");
                let result = match op {
                    BinaryOp::Add => {
                        out.push_str("  add w0, w9, w0\n");
                        "i32"
                    }
                    BinaryOp::Sub => {
                        out.push_str("  sub w0, w9, w0\n");
                        "i32"
                    }
                    BinaryOp::Mul => {
                        out.push_str("  mul w0, w9, w0\n");
                        "i32"
                    }
                    BinaryOp::Div => {
                        out.push_str("  sdiv w0, w9, w0\n");
                        "i32"
                    }
                    BinaryOp::Mod => {
                        out.push_str("  sdiv w10, w9, w0\n  msub w0, w10, w0, w9\n");
                        "i32"
                    }
                    BinaryOp::And => {
                        out.push_str("  and w0, w9, w0\n");
                        "bool"
                    }
                    BinaryOp::Or => {
                        out.push_str("  orr w0, w9, w0\n");
                        "bool"
                    }
                    BinaryOp::Eq => {
                        out.push_str("  cmp w9, w0\n  cset w0, eq\n");
                        "bool"
                    }
                    BinaryOp::Ne => {
                        out.push_str("  cmp w9, w0\n  cset w0, ne\n");
                        "bool"
                    }
                    BinaryOp::Lt => {
                        out.push_str("  cmp w9, w0\n  cset w0, lt\n");
                        "bool"
                    }
                    BinaryOp::Le => {
                        out.push_str("  cmp w9, w0\n  cset w0, le\n");
                        "bool"
                    }
                    BinaryOp::Gt => {
                        out.push_str("  cmp w9, w0\n  cset w0, gt\n");
                        "bool"
                    }
                    BinaryOp::Ge => {
                        out.push_str("  cmp w9, w0\n  cset w0, ge\n");
                        "bool"
                    }
                };
                Ok(TypeRef::simple(result))
            }
            Expr::Unary { op, expr, .. } => {
                let ty = self.emit_expr(expr, out)?;
                match op {
                    UnaryOp::Not => {
                        out.push_str("  cmp w0, #0\n  cset w0, eq\n");
                        Ok(TypeRef::simple("bool"))
                    }
                    UnaryOp::Neg => {
                        out.push_str("  neg w0, w0\n");
                        Ok(ty)
                    }
                }
            }
            Expr::Call { callee, args, span } => {
                let Some(name) = expr_name(callee) else {
                    return Err(custom_unsupported(
                        "custom backend call target is unsupported",
                        *span,
                    ));
                };
                if matches!(name.as_str(), "io.println" | "io.print" | "io.eprintln") {
                    let Some(arg) = args.first() else {
                        return Err(custom_unsupported(
                            format!("{} expects one argument", name),
                            *span,
                        ));
                    };
                    self.emit_expr(arg, out)?;
                    let symbol = match name.as_str() {
                        "io.println" => "_ax_io_println",
                        "io.print" => "_ax_io_print",
                        _ => "_ax_io_eprintln",
                    };
                    out.push_str(&format!("  bl {}\n", symbol));
                    return Ok(TypeRef::simple("void"));
                }
                let (is_async, ret) = self
                    .module
                    .semantic
                    .functions
                    .get(&name)
                    .map(|function| (function.is_async, function.ret.clone()))
                    .ok_or_else(|| {
                        custom_unsupported(format!("custom backend cannot call `{}`", name), *span)
                    })?;
                if is_async {
                    return Err(custom_unsupported(
                        "custom backend does not support async calls yet",
                        *span,
                    ));
                }
                if args.len() > 8 {
                    return Err(custom_unsupported(
                        "custom backend supports up to 8 call arguments",
                        *span,
                    ));
                }
                for arg in args {
                    self.emit_expr(arg, out)?;
                    out.push_str("  sub sp, sp, #16\n  str x0, [sp]\n");
                }
                for idx in (0..args.len()).rev() {
                    out.push_str(&format!("  ldr x{}, [sp]\n  add sp, sp, #16\n", idx));
                }
                out.push_str(&format!("  bl {}\n", asm_symbol(&name)));
                Ok(ret)
            }
            Expr::Member {
                object,
                field,
                span,
            } => {
                if let Expr::Ident(root, _) = object.as_ref() {
                    if let Some(index) = self.module.variant_index(root, field) {
                        out.push_str(&format!("  mov w0, #{}\n", index));
                        return Ok(TypeRef::simple(root));
                    }
                }
                Err(custom_unsupported(
                    "custom backend does not support this member expression yet",
                    *span,
                ))
            }
            Expr::Float(_, span) | Expr::Await { span, .. } | Expr::RecordInit { span, .. } => Err(
                custom_unsupported("custom backend does not support this expression yet", *span),
            ),
        }
    }

    fn alloc_local(&mut self, name: String, ty: TypeRef) -> i32 {
        let offset = self.next_slot * 8;
        self.next_slot += 1;
        self.locals.insert(name, AsmLocal { offset, ty });
        offset
    }
}

fn count_top_level_lets(block: &Block) -> usize {
    block
        .stmts
        .iter()
        .filter(|stmt| matches!(stmt, Stmt::Let { .. }))
        .count()
}

fn align16(value: i32) -> i32 {
    if value == 0 {
        0
    } else {
        ((value + 15) / 16) * 16
    }
}

fn asm_symbol(name: &str) -> String {
    format!("_{}", sanitize_ident(name))
}

fn asm_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_ascii_graphic() || ch == ' ' => out.push(ch),
            ch => out.push_str(&format!("\\{:03o}", ch as u32)),
        }
    }
    out
}

struct LlvmModule<'a> {
    ir: &'a IrProgram,
    globals: Vec<String>,
    external_pack_decls: BTreeSet<String>,
    string_count: usize,
}

impl<'a> LlvmModule<'a> {
    fn new(ir: &'a IrProgram) -> Self {
        Self {
            ir,
            globals: Vec::new(),
            external_pack_decls: BTreeSet::new(),
            string_count: 0,
        }
    }

    fn generate(mut self) -> AxResult<String> {
        let mut body = String::new();
        if let Some(server) = self.ir.program.items.iter().find_map(|item| match item {
            Item::Server(server) => Some(server),
            _ => None,
        }) {
            body.push_str(&self.generate_http_server(server));
        } else if let Some(tcp) = self.ir.program.items.iter().find_map(|item| match item {
            Item::Tcp(tcp) => Some(tcp),
            _ => None,
        }) {
            body.push_str(&self.generate_tcp_server(tcp));
        } else {
            for item in &self.ir.program.items {
                if let Item::Function(function) = item {
                    body.push_str(&self.generate_function(function)?);
                    if function.is_async {
                        body.push_str(&self.generate_async_entry(function));
                    }
                    body.push('\n');
                }
            }
        }

        let mut out = String::new();
        out.push_str("; Ax v1.0 LLVM IR\n");
        out.push_str("%ax_http_route = type { ptr, ptr, i32, ptr }\n");
        out.push_str("%ax_tcp_route = type { ptr, i32, ptr }\n");
        out.push_str("declare void @ax_io_println(ptr)\n");
        out.push_str("declare void @ax_io_print(ptr)\n");
        out.push_str("declare void @ax_io_eprintln(ptr)\n");
        out.push_str("declare ptr @ax_io_read_line()\n");
        out.push_str("declare ptr @ax_fs_read_text(ptr)\n");
        out.push_str("declare ptr @ax_fs_read_text_or(ptr, ptr)\n");
        out.push_str("declare ptr @ax_fs_read_text_limit(ptr, i32)\n");
        out.push_str("declare ptr @ax_fs_read_text_range(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_fs_read_text_tail(ptr, i32)\n");
        out.push_str("declare ptr @ax_fs_read_lines(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_fs_read_lines_json(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_fs_read_jsonl(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_fs_read_json(ptr)\n");
        out.push_str("declare ptr @ax_fs_read_json_or(ptr, ptr)\n");
        out.push_str("declare ptr @ax_fs_read_base64(ptr)\n");
        out.push_str("declare ptr @ax_fs_read_base64_range(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_fs_read_base64_tail(ptr, i32)\n");
        out.push_str("declare void @ax_fs_write_text(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_write_text_atomic(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_write_json_atomic(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_write_base64(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_append_text(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_append_jsonl(ptr, ptr)\n");
        out.push_str("declare i32 @ax_fs_exists(ptr)\n");
        out.push_str("declare void @ax_fs_remove(ptr)\n");
        out.push_str("declare void @ax_fs_mkdir(ptr)\n");
        out.push_str("declare void @ax_fs_mkdir_all(ptr)\n");
        out.push_str("declare void @ax_fs_ensure_parent(ptr)\n");
        out.push_str("declare ptr @ax_fs_list(ptr)\n");
        out.push_str("declare ptr @ax_fs_list_json(ptr)\n");
        out.push_str("declare ptr @ax_fs_list_stat_json(ptr)\n");
        out.push_str("declare ptr @ax_fs_walk(ptr)\n");
        out.push_str("declare ptr @ax_fs_walk_json(ptr)\n");
        out.push_str("declare ptr @ax_fs_walk_stat_json(ptr)\n");
        out.push_str("declare void @ax_fs_copy(ptr, ptr)\n");
        out.push_str("declare i64 @ax_fs_size(ptr)\n");
        out.push_str("declare ptr @ax_fs_cwd()\n");
        out.push_str("declare ptr @ax_fs_temp_dir()\n");
        out.push_str("declare void @ax_fs_rename(ptr, ptr)\n");
        out.push_str("declare void @ax_fs_remove_dir(ptr)\n");
        out.push_str("declare i32 @ax_fs_is_file(ptr)\n");
        out.push_str("declare i32 @ax_fs_is_dir(ptr)\n");
        out.push_str("declare i64 @ax_fs_modified(ptr)\n");
        out.push_str("declare ptr @ax_fs_find(ptr, ptr)\n");
        out.push_str("declare ptr @ax_fs_glob(ptr, ptr)\n");
        out.push_str("declare ptr @ax_fs_stat_json(ptr)\n");
        out.push_str("declare ptr @ax_crypto_sha256_hex(ptr)\n");
        out.push_str("declare ptr @ax_crypto_sha256_json(ptr)\n");
        out.push_str("declare i32 @ax_crypto_sha256_verify_hex(ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_hex(ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_json(ptr, ptr)\n");
        out.push_str("declare i32 @ax_crypto_hmac_sha256_verify_hex(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_file_hex(ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_file_json(ptr, ptr)\n");
        out.push_str("declare i32 @ax_crypto_hmac_sha256_file_verify_hex(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_file_range_hex(ptr, ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_crypto_hmac_sha256_file_range_json(ptr, ptr, i32, i32)\n");
        out.push_str(
            "declare i32 @ax_crypto_hmac_sha256_file_range_verify_hex(ptr, ptr, i32, i32, ptr)\n",
        );
        out.push_str("declare ptr @ax_crypto_sha256_file_hex(ptr)\n");
        out.push_str("declare ptr @ax_crypto_sha256_file_json(ptr)\n");
        out.push_str("declare i32 @ax_crypto_sha256_file_verify_hex(ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_sha256_file_range_hex(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_crypto_sha256_file_range_json(ptr, i32, i32)\n");
        out.push_str("declare i32 @ax_crypto_sha256_file_range_verify_hex(ptr, i32, i32, ptr)\n");
        out.push_str("declare ptr @ax_crypto_base64_encode(ptr)\n");
        out.push_str("declare ptr @ax_crypto_base64_decode(ptr)\n");
        out.push_str("declare i32 @ax_crypto_constant_time_eq(ptr, ptr)\n");
        out.push_str("declare ptr @ax_crypto_random_hex(i32)\n");
        out.push_str("declare ptr @ax_crypto_random_base64url(i32)\n");
        out.push_str("declare ptr @ax_crypto_uuid_v4()\n");
        out.push_str("declare ptr @ax_env_get(ptr)\n");
        out.push_str("declare i32 @ax_env_has(ptr)\n");
        out.push_str("declare void @ax_env_set(ptr, ptr)\n");
        out.push_str("declare ptr @ax_env_get_or(ptr, ptr)\n");
        out.push_str("declare ptr @ax_env_snapshot_json(ptr)\n");
        out.push_str("declare i32 @ax_env_load_dotenv(ptr)\n");
        out.push_str("declare ptr @ax_env_load_dotenv_json(ptr)\n");
        out.push_str("declare ptr @ax_process_exec(ptr)\n");
        out.push_str("declare ptr @ax_process_exec_limit(ptr, i32)\n");
        out.push_str("declare i32 @ax_process_status(ptr)\n");
        out.push_str("declare ptr @ax_process_run_json(ptr, i32)\n");
        out.push_str("declare ptr @ax_process_run_log_json(ptr, i32)\n");
        out.push_str("declare ptr @ax_process_run_lines_json(ptr, i32)\n");
        out.push_str("declare ptr @ax_process_run_log_lines_json(ptr, i32)\n");
        out.push_str("declare void @ax_cli_init(i32, ptr)\n");
        out.push_str("declare i32 @ax_cli_argc()\n");
        out.push_str("declare ptr @ax_cli_arg(i32)\n");
        out.push_str("declare i32 @ax_cli_has(ptr)\n");
        out.push_str("declare ptr @ax_cli_value(ptr)\n");
        out.push_str("declare ptr @ax_cli_value_or(ptr, ptr)\n");
        out.push_str("declare ptr @ax_cli_args_json()\n");
        out.push_str("declare ptr @ax_cli_parse_json()\n");
        out.push_str("declare ptr @ax_http_get(ptr)\n");
        out.push_str("declare ptr @ax_http_post(ptr, ptr)\n");
        out.push_str("declare ptr @ax_http_get_json(ptr, i32)\n");
        out.push_str("declare ptr @ax_http_post_json(ptr, ptr, i32)\n");
        out.push_str("declare ptr @ax_json_escape(ptr)\n");
        out.push_str("declare ptr @ax_json_quote(ptr)\n");
        out.push_str("declare ptr @ax_json_pair(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_string_pair(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_set(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_string_set(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_remove(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_object(ptr)\n");
        out.push_str("declare ptr @ax_json_array(ptr)\n");
        out.push_str("declare ptr @ax_json_array_push(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_string_array_push(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_compact(ptr)\n");
        out.push_str("declare i32 @ax_json_valid(ptr)\n");
        out.push_str("declare ptr @ax_json_get(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_query(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_get_or(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_query_or(ptr, ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_has(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_query_has(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_int(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_bool(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_query_int(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_query_bool(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_contains(ptr, ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_query_contains(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_kind(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_query_kind(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_keys(ptr)\n");
        out.push_str("declare ptr @ax_json_keys_json(ptr)\n");
        out.push_str("declare ptr @ax_json_query_keys_json(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_len(ptr, ptr)\n");
        out.push_str("declare i32 @ax_json_query_len(ptr, ptr)\n");
        out.push_str("declare ptr @ax_json_at(ptr, ptr, i32)\n");
        out.push_str("declare ptr @ax_json_query_at(ptr, ptr, i32)\n");
        out.push_str("declare i32 @ax_str_len(ptr)\n");
        out.push_str("declare i32 @ax_str_eq(ptr, ptr)\n");
        out.push_str("declare i32 @ax_str_contains(ptr, ptr)\n");
        out.push_str("declare i32 @ax_str_index_of(ptr, ptr)\n");
        out.push_str("declare i32 @ax_str_count(ptr, ptr)\n");
        out.push_str("declare i32 @ax_str_starts_with(ptr, ptr)\n");
        out.push_str("declare i32 @ax_str_ends_with(ptr, ptr)\n");
        out.push_str("declare ptr @ax_str_trim(ptr)\n");
        out.push_str("declare ptr @ax_str_upper(ptr)\n");
        out.push_str("declare ptr @ax_str_lower(ptr)\n");
        out.push_str("declare ptr @ax_str_concat(ptr, ptr)\n");
        out.push_str("declare ptr @ax_str_repeat(ptr, i32)\n");
        out.push_str("declare ptr @ax_str_replace(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_str_slice(ptr, i32, i32)\n");
        out.push_str("declare ptr @ax_str_split_json(ptr, ptr)\n");
        out.push_str("declare ptr @ax_str_lines_json(ptr)\n");
        out.push_str("declare ptr @ax_str_from_i64(i64)\n");
        out.push_str("declare i64 @ax_str_parse_i64(ptr)\n");
        out.push_str("declare i32 @ax_str_parse_i32(ptr)\n");
        out.push_str("declare ptr @ax_str_token(ptr, i32)\n");
        out.push_str("declare ptr @ax_str_token_upper(ptr, i32)\n");
        out.push_str("declare ptr @ax_str_line(ptr, i32)\n");
        out.push_str("declare ptr @ax_path_normalize(ptr)\n");
        out.push_str("declare ptr @ax_path_join(ptr, ptr)\n");
        out.push_str("declare ptr @ax_path_basename(ptr)\n");
        out.push_str("declare ptr @ax_path_dirname(ptr)\n");
        out.push_str("declare ptr @ax_path_extname(ptr)\n");
        out.push_str("declare ptr @ax_path_stem(ptr)\n");
        out.push_str("declare i32 @ax_path_is_absolute(ptr)\n");
        out.push_str("declare ptr @ax_url_encode(ptr)\n");
        out.push_str("declare ptr @ax_url_decode(ptr)\n");
        out.push_str("declare ptr @ax_url_query_get(ptr, ptr)\n");
        out.push_str("declare ptr @ax_url_query_or(ptr, ptr, ptr)\n");
        out.push_str("declare i32 @ax_url_query_has(ptr, ptr)\n");
        out.push_str("declare ptr @ax_url_query_json(ptr)\n");
        out.push_str("declare ptr @ax_url_path(ptr)\n");
        out.push_str("declare ptr @ax_url_host(ptr)\n");
        out.push_str("declare ptr @ax_url_scheme(ptr)\n");
        out.push_str("declare i64 @ax_time_now()\n");
        out.push_str("declare i64 @ax_time_now_ms()\n");
        out.push_str("declare ptr @ax_time_iso_utc(i64)\n");
        out.push_str("declare void @ax_time_sleep_ms(i32)\n");
        out.push_str("declare ptr @ax_heap_alloc(i32)\n");
        out.push_str("declare void @ax_heap_free(ptr)\n");
        out.push_str("declare void @ax_panic(ptr)\n");
        out.push_str("declare ptr @ax_async_spawn(ptr)\n");
        out.push_str("declare ptr @ax_async_spawn_with_context(ptr, ptr)\n");
        out.push_str("declare ptr @ax_async_context_new(i32)\n");
        out.push_str("declare void @ax_async_context_set_i32(ptr, i32, i32)\n");
        out.push_str("declare void @ax_async_context_set_i64(ptr, i32, i64)\n");
        out.push_str("declare void @ax_async_context_set_f64(ptr, i32, double)\n");
        out.push_str("declare void @ax_async_context_set_ptr(ptr, i32, ptr)\n");
        out.push_str("declare i32 @ax_async_context_get_i32(ptr, i32)\n");
        out.push_str("declare i64 @ax_async_context_get_i64(ptr, i32)\n");
        out.push_str("declare double @ax_async_context_get_f64(ptr, i32)\n");
        out.push_str("declare ptr @ax_async_context_get_ptr(ptr, i32)\n");
        out.push_str("declare void @ax_async_await_void(ptr)\n");
        out.push_str("declare i32 @ax_async_await_i32(ptr)\n");
        out.push_str("declare i64 @ax_async_await_i64(ptr)\n");
        out.push_str("declare double @ax_async_await_f64(ptr)\n");
        out.push_str("declare ptr @ax_async_await_ptr(ptr)\n");
        out.push_str("declare void @ax_async_cancel(ptr)\n");
        out.push_str("declare void @ax_async_detach(ptr)\n");
        out.push_str("declare ptr @ax_async_box_i32(i32)\n");
        out.push_str("declare ptr @ax_async_box_i64(i64)\n");
        out.push_str("declare ptr @ax_async_box_f64(double)\n");
        out.push_str("declare ptr @ax_async_box_bool(i32)\n");
        out.push_str("declare i32 @ax_http_server_start(i32, ptr, i32)\n");
        out.push_str("declare i32 @ax_http_tls_server_start(i32, ptr, i32)\n");
        out.push_str("declare i32 @ax_tcp_server_start(i32, ptr, i32)\n");
        out.push_str("declare i32 @ax_tcp_tls_server_start(i32, ptr, i32)\n");
        out.push_str("declare i32 @ax_tcp_ping_server(i32)\n");
        out.push_str("declare ptr @ax_tcp_listen(i32)\n");
        out.push_str("declare ptr @ax_tcp_listen_on(ptr, i32)\n");
        out.push_str("declare ptr @ax_tcp_connect(ptr, i32)\n");
        out.push_str("declare i32 @ax_tcp_serve_text(ptr, ptr, ptr, i32)\n");
        out.push_str("declare ptr @ax_tcp_accept(ptr)\n");
        out.push_str("declare ptr @ax_tcp_read_text(ptr, i32)\n");
        out.push_str("declare void @ax_tcp_write_text(ptr, ptr)\n");
        out.push_str("declare ptr @ax_tcp_request_text(ptr, ptr, i32)\n");
        out.push_str("declare void @ax_tcp_close(ptr)\n");
        out.push_str("declare ptr @ax_map_new(i32)\n");
        out.push_str("declare void @ax_map_set(ptr, ptr, ptr)\n");
        out.push_str("declare ptr @ax_map_get(ptr, ptr)\n");
        out.push_str("declare i32 @ax_map_has(ptr, ptr)\n");
        out.push_str("declare i32 @ax_map_del(ptr, ptr)\n");
        out.push_str("declare i32 @ax_map_len(ptr)\n");
        out.push_str("declare void @ax_map_clear(ptr)\n");
        for symbol in &self.external_pack_decls {
            out.push_str(&format!("declare void @{}()\n", symbol));
        }
        out.push('\n');
        for global in &self.globals {
            out.push_str(global);
            out.push('\n');
        }
        if !self.globals.is_empty() {
            out.push('\n');
        }
        out.push_str(&body);
        Ok(out)
    }

    fn generate_http_server(&mut self, server: &ServerBlock) -> String {
        let mut route_entries = Vec::new();
        for route in &server.routes {
            let method = self.add_string(route.method.as_str());
            let path = self.add_string(&route.path);
            let (kind, body) = match &route.response {
                HttpResponse::Text(value) => (0, self.add_string(value)),
                HttpResponse::Json(fields) => (1, self.add_string(&json_body(fields))),
                HttpResponse::StreamBody => (2, "null".to_string()),
            };
            route_entries.push(format!(
                "%ax_http_route {{ ptr {}, ptr {}, i32 {}, ptr {} }}",
                method, path, kind, body
            ));
        }
        let route_array = format!("@.routes.{}", self.string_count);
        self.globals.push(format!(
            "{} = private constant [{} x %ax_http_route] [{}]",
            route_array,
            route_entries.len(),
            route_entries.join(", ")
        ));
        let entry = if server.tls {
            "ax_http_tls_server_start"
        } else {
            "ax_http_server_start"
        };
        format!(
            "define i32 @main() {{\nentry:\n  %r = call i32 @ax_http_server_start(i32 {}, ptr {}, i32 {})\n  ret i32 %r\n}}\n",
            server.port, route_array, route_entries.len()
        )
        .replace("@ax_http_server_start", &format!("@{}", entry))
    }

    fn generate_tcp_server(&mut self, tcp: &TcpBlock) -> String {
        let mut route_entries = Vec::new();
        for route in &tcp.routes {
            let (pattern, is_wildcard) = match &route.pattern {
                TcpPattern::Exact(value) => (self.add_string(value), 0),
                TcpPattern::Wildcard => (self.add_string(""), 1),
            };
            let response = self.add_string(&route.response);
            route_entries.push(format!(
                "%ax_tcp_route {{ ptr {}, i32 {}, ptr {} }}",
                pattern, is_wildcard, response
            ));
        }
        let route_array = format!("@.tcp.routes.{}", self.string_count);
        self.globals.push(format!(
            "{} = private constant [{} x %ax_tcp_route] [{}]",
            route_array,
            route_entries.len(),
            route_entries.join(", ")
        ));
        let entry = if tcp.tls {
            "ax_tcp_tls_server_start"
        } else {
            "ax_tcp_server_start"
        };
        format!(
            "define i32 @main() {{\nentry:\n  %r = call i32 @ax_tcp_server_start(i32 {}, ptr {}, i32 {})\n  ret i32 %r\n}}\n",
            tcp.port, route_array, route_entries.len()
        )
        .replace("@ax_tcp_server_start", &format!("@{}", entry))
    }

    fn generate_function(&mut self, function: &'a Function) -> AxResult<String> {
        let mut function_codegen = FunctionCodegen::new(self, function);
        function_codegen.generate()
    }

    fn variant_index(&self, name: &str, variant: &str) -> Option<i32> {
        self.ir
            .semantic
            .enums
            .get(name)
            .or_else(|| self.ir.semantic.errors.get(name))
            .and_then(|item| item.variants.get(variant).copied())
    }

    fn generate_async_entry(&self, function: &Function) -> String {
        let wrapper = async_entry_name(&function.name);
        let function_name = sanitize_ident(&function.name);
        let ret_ty = llvm_type(&function.ret);
        let mut out = format!("\ndefine ptr @{}(ptr %ctx) {{\nentry:\n", wrapper);
        out.push_str("  %ctx.ignore = ptrtoint ptr %ctx to i64\n");
        let mut args = Vec::new();
        for (idx, param) in function.params.iter().enumerate() {
            let arg_name = format!("%async.arg{}", idx);
            match llvm_type(&param.ty).as_str() {
                "i1" => {
                    out.push_str(&format!(
                        "  %async.raw{} = call i32 @ax_async_context_get_i32(ptr %ctx, i32 {})\n",
                        idx, idx
                    ));
                    out.push_str(&format!(
                        "  {} = icmp ne i32 %async.raw{}, 0\n",
                        arg_name, idx
                    ));
                    args.push(format!("i1 {}", arg_name));
                }
                "i32" => {
                    out.push_str(&format!(
                        "  {} = call i32 @ax_async_context_get_i32(ptr %ctx, i32 {})\n",
                        arg_name, idx
                    ));
                    args.push(format!("i32 {}", arg_name));
                }
                "i64" => {
                    out.push_str(&format!(
                        "  {} = call i64 @ax_async_context_get_i64(ptr %ctx, i32 {})\n",
                        arg_name, idx
                    ));
                    args.push(format!("i64 {}", arg_name));
                }
                "double" => {
                    out.push_str(&format!(
                        "  {} = call double @ax_async_context_get_f64(ptr %ctx, i32 {})\n",
                        arg_name, idx
                    ));
                    args.push(format!("double {}", arg_name));
                }
                _ => {
                    out.push_str(&format!(
                        "  {} = call ptr @ax_async_context_get_ptr(ptr %ctx, i32 {})\n",
                        arg_name, idx
                    ));
                    args.push(format!("ptr {}", arg_name));
                }
            }
        }
        let args = args.join(", ");
        match ret_ty.as_str() {
            "void" => {
                out.push_str(&format!("  call void @{}({})\n", function_name, args));
                out.push_str("  ret ptr null\n");
            }
            "i32" => {
                out.push_str(&format!("  %r = call i32 @{}({})\n", function_name, args));
                out.push_str("  %boxed = call ptr @ax_async_box_i32(i32 %r)\n");
                out.push_str("  ret ptr %boxed\n");
            }
            "i64" => {
                out.push_str(&format!("  %r = call i64 @{}({})\n", function_name, args));
                out.push_str("  %boxed = call ptr @ax_async_box_i64(i64 %r)\n");
                out.push_str("  ret ptr %boxed\n");
            }
            "i1" => {
                out.push_str(&format!("  %r = call i1 @{}({})\n", function_name, args));
                out.push_str("  %r.i32 = zext i1 %r to i32\n");
                out.push_str("  %boxed = call ptr @ax_async_box_bool(i32 %r.i32)\n");
                out.push_str("  ret ptr %boxed\n");
            }
            "double" => {
                out.push_str(&format!(
                    "  %r = call double @{}({})\n",
                    function_name, args
                ));
                out.push_str("  %boxed = call ptr @ax_async_box_f64(double %r)\n");
                out.push_str("  ret ptr %boxed\n");
            }
            "ptr" => {
                out.push_str(&format!("  %r = call ptr @{}({})\n", function_name, args));
                out.push_str("  ret ptr %r\n");
            }
            _ => {
                out.push_str(&format!(
                    "  %r = call {} @{}({})\n",
                    ret_ty, function_name, args
                ));
                out.push_str("  ret ptr null\n");
            }
        }
        out.push_str("}\n");
        out
    }

    fn external_native_symbol_for_call(&self, callee_name: &str) -> Option<String> {
        let (root, operation) = callee_name.split_once('.')?;
        if operation.is_empty() {
            return None;
        }
        let pack = self
            .ir
            .semantic
            .packs
            .iter()
            .find(|pack| !pack.native_sources.is_empty() && pack_alias(&pack.name) == root)?;
        Some(format!(
            "ax_pack_{}_{}",
            sanitize_ident(&pack.name),
            sanitize_ident(operation)
        ))
    }

    fn add_string(&mut self, value: &str) -> String {
        let name = format!("@.str.{}", self.string_count);
        self.string_count += 1;
        let escaped = llvm_escape_string(value);
        let len = value.as_bytes().len() + 1;
        self.globals.push(format!(
            "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\", align 1",
            name, len, escaped
        ));
        name
    }
}

struct FunctionCodegen<'m, 'a> {
    module: &'m mut LlvmModule<'a>,
    function: &'a Function,
    out: String,
    temp: usize,
    label: usize,
    locals: BTreeMap<String, LlValue>,
    terminated: bool,
}

#[derive(Clone, Debug)]
struct LlValue {
    llvm_ty: String,
    repr: String,
    ax_ty: String,
    fields: BTreeMap<String, LlValue>,
    storage: Option<String>,
    const_str: Option<String>,
}

impl LlValue {
    fn scalar(
        llvm_ty: impl Into<String>,
        repr: impl Into<String>,
        ax_ty: impl Into<String>,
    ) -> Self {
        Self {
            llvm_ty: llvm_ty.into(),
            repr: repr.into(),
            ax_ty: ax_ty.into(),
            fields: BTreeMap::new(),
            storage: None,
            const_str: None,
        }
    }

    fn with_const_str(mut self, value: impl Into<String>) -> Self {
        self.const_str = Some(value.into());
        self
    }

    fn with_optional_const_str(mut self, value: Option<String>) -> Self {
        self.const_str = value;
        self
    }

    fn void() -> Self {
        Self::scalar("void", "", "void")
    }

    fn record(name: impl Into<String>, fields: BTreeMap<String, LlValue>) -> Self {
        Self {
            llvm_ty: "record".to_string(),
            repr: String::new(),
            ax_ty: name.into(),
            fields,
            storage: None,
            const_str: None,
        }
    }

    fn slot(
        llvm_ty: impl Into<String>,
        ptr: impl Into<String>,
        ax_ty: impl Into<String>,
        const_str: Option<String>,
    ) -> Self {
        Self {
            llvm_ty: llvm_ty.into(),
            repr: String::new(),
            ax_ty: ax_ty.into(),
            fields: BTreeMap::new(),
            storage: Some(ptr.into()),
            const_str,
        }
    }
}

impl<'m, 'a> FunctionCodegen<'m, 'a> {
    fn new(module: &'m mut LlvmModule<'a>, function: &'a Function) -> Self {
        Self {
            module,
            function,
            out: String::new(),
            temp: 0,
            label: 0,
            locals: BTreeMap::new(),
            terminated: false,
        }
    }

    fn generate(&mut self) -> AxResult<String> {
        let is_entry_main = self.function.name == "main" && self.function.params.is_empty();
        let entry_needs_cli =
            is_entry_main && semantic_uses_effect(&self.module.ir.semantic, "main", "cli.read");
        let params = if entry_needs_cli {
            "i32 %argc, ptr %argv".to_string()
        } else if is_entry_main {
            String::new()
        } else {
            self.function
                .params
                .iter()
                .map(|param| format!("{} %{}", llvm_type(&param.ty), sanitize_ident(&param.name)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.out.push_str(&format!(
            "define {} @{}({}) {{\nentry:\n",
            llvm_type(&self.function.ret),
            sanitize_ident(&self.function.name),
            params
        ));
        if entry_needs_cli {
            self.out
                .push_str("  call void @ax_cli_init(i32 %argc, ptr %argv)\n");
        }
        for param in &self.function.params {
            let llvm_ty = llvm_type(&param.ty);
            let ptr = self.next_temp();
            let param_name = format!("%{}", sanitize_ident(&param.name));
            self.out.push_str(&format!(
                "  {} = alloca {}\n  store {} {}, ptr {}\n",
                ptr, llvm_ty, llvm_ty, param_name, ptr
            ));
            self.locals.insert(
                param.name.clone(),
                LlValue::slot(llvm_ty, ptr, param.ty.name.clone(), None),
            );
        }
        for stmt in &self.function.body.stmts {
            self.emit_stmt(stmt)?;
        }
        if !self.terminated {
            self.emit_default_return();
        }
        self.out.push_str("}\n");
        Ok(std::mem::take(&mut self.out))
    }

    fn emit_stmt(&mut self, stmt: &Stmt) -> AxResult<()> {
        if self.terminated {
            return Ok(());
        }
        match stmt {
            Stmt::Let { name, ty, expr, .. } => {
                let value = if let Some(expected) = ty {
                    let value = self.emit_expr(expr)?;
                    self.coerce_value(value, expected)?
                } else {
                    self.emit_expr(expr)?
                };
                if value.llvm_ty == "record" || value.llvm_ty == "void" {
                    self.locals.insert(name.clone(), value);
                } else {
                    let ptr = self.next_temp();
                    self.out.push_str(&format!(
                        "  {} = alloca {}\n  store {} {}, ptr {}\n",
                        ptr, value.llvm_ty, value.llvm_ty, value.repr, ptr
                    ));
                    let const_str = value.const_str.clone();
                    self.locals.insert(
                        name.clone(),
                        LlValue::slot(value.llvm_ty, ptr, value.ax_ty, const_str),
                    );
                }
            }
            Stmt::Assign { target, expr, span } => {
                let value = self.emit_expr(expr)?;
                if let Expr::Ident(name, _) = target {
                    if let Some(local) = self.locals.get(name).cloned() {
                        if let Some(ptr) = local.storage {
                            let expected = TypeRef::simple(local.ax_ty.clone());
                            let value = self.coerce_value(value, &expected)?;
                            self.out.push_str(&format!(
                                "  store {} {}, ptr {}\n",
                                local.llvm_ty, value.repr, ptr
                            ));
                            self.locals.insert(
                                name.clone(),
                                LlValue::slot(local.llvm_ty, ptr, local.ax_ty, value.const_str),
                            );
                        } else {
                            self.locals.insert(name.clone(), value);
                        }
                    } else {
                        return Err(Diagnostic::error(
                            "AX_CODEGEN_ERROR",
                            format!("unknown local `{}`", name),
                            *span,
                        ));
                    }
                } else {
                    return Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        "unsupported assignment target",
                        *span,
                    ));
                }
            }
            Stmt::Return { expr, .. } => {
                if let Some(expr) = expr {
                    let value = self.emit_expr(expr)?;
                    let value = self.coerce_value(value, &self.function.ret)?;
                    self.out
                        .push_str(&format!("  ret {} {}\n", value.llvm_ty, value.repr));
                } else {
                    self.out.push_str("  ret void\n");
                }
                self.terminated = true;
            }
            Stmt::Expr { expr, .. } | Stmt::Assert { expr, .. } => {
                self.emit_expr(expr)?;
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => self.emit_if(cond, then_block, else_block.as_ref())?,
            Stmt::Loop { body, .. } => self.emit_loop(body)?,
            Stmt::While { cond, body, .. } => self.emit_while(cond, body)?,
        }
        Ok(())
    }

    fn emit_if(
        &mut self,
        cond: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
    ) -> AxResult<()> {
        let cond = self.emit_expr(cond)?;
        let then_label = self.next_label("if.then");
        let else_label = self.next_label("if.else");
        let end_label = self.next_label("if.end");
        self.out.push_str(&format!(
            "  br i1 {}, label %{}, label %{}\n{}:\n",
            cond.repr, then_label, else_label, then_label
        ));
        self.terminated = false;
        for stmt in &then_block.stmts {
            self.emit_stmt(stmt)?;
        }
        if !self.terminated {
            self.out.push_str(&format!("  br label %{}\n", end_label));
        }
        self.out.push_str(&format!("{}:\n", else_label));
        self.terminated = false;
        if let Some(else_block) = else_block {
            for stmt in &else_block.stmts {
                self.emit_stmt(stmt)?;
            }
        }
        if !self.terminated {
            self.out.push_str(&format!("  br label %{}\n", end_label));
        }
        self.out.push_str(&format!("{}:\n", end_label));
        self.terminated = false;
        Ok(())
    }

    fn emit_loop(&mut self, body: &Block) -> AxResult<()> {
        let loop_label = self.next_label("loop");
        self.out
            .push_str(&format!("  br label %{}\n{}:\n", loop_label, loop_label));
        self.terminated = false;
        for stmt in &body.stmts {
            self.emit_stmt(stmt)?;
        }
        if !self.terminated {
            self.out.push_str(&format!("  br label %{}\n", loop_label));
        }
        self.terminated = true;
        Ok(())
    }

    fn emit_while(&mut self, cond: &Expr, body: &Block) -> AxResult<()> {
        let cond_label = self.next_label("while.cond");
        let body_label = self.next_label("while.body");
        let end_label = self.next_label("while.end");
        self.out
            .push_str(&format!("  br label %{}\n{}:\n", cond_label, cond_label));
        let cond = self.emit_expr(cond)?;
        self.out.push_str(&format!(
            "  br i1 {}, label %{}, label %{}\n{}:\n",
            cond.repr, body_label, end_label, body_label
        ));
        self.terminated = false;
        for stmt in &body.stmts {
            self.emit_stmt(stmt)?;
        }
        if !self.terminated {
            self.out.push_str(&format!("  br label %{}\n", cond_label));
        }
        self.out.push_str(&format!("{}:\n", end_label));
        self.terminated = false;
        Ok(())
    }

    fn coerce_value(&mut self, value: LlValue, expected: &TypeRef) -> AxResult<LlValue> {
        self.coerce_value_to(value, &llvm_type(expected), &expected.name)
    }

    fn coerce_value_to(
        &mut self,
        value: LlValue,
        expected_llvm_ty: &str,
        expected_ax_ty: &str,
    ) -> AxResult<LlValue> {
        if value.llvm_ty == expected_llvm_ty {
            let const_str = value.const_str.clone();
            return Ok(LlValue::scalar(
                expected_llvm_ty.to_string(),
                value.repr,
                expected_ax_ty.to_string(),
            )
            .with_optional_const_str(const_str));
        }
        if value.llvm_ty == "record" || value.llvm_ty == "void" {
            return Ok(value);
        }

        let temp = self.next_temp();
        if let (Some(from_width), Some(to_width)) = (
            integer_llvm_width(&value.llvm_ty),
            integer_llvm_width(expected_llvm_ty),
        ) {
            let inst = if from_width < to_width {
                if is_unsigned_ax_type(&value.ax_ty) {
                    "zext"
                } else {
                    "sext"
                }
            } else if from_width > to_width {
                "trunc"
            } else {
                "bitcast"
            };
            self.out.push_str(&format!(
                "  {} = {} {} {} to {}\n",
                temp, inst, value.llvm_ty, value.repr, expected_llvm_ty
            ));
            return Ok(LlValue::scalar(
                expected_llvm_ty.to_string(),
                temp,
                expected_ax_ty.to_string(),
            ));
        }

        let float_inst = match (value.llvm_ty.as_str(), expected_llvm_ty) {
            ("double", "float") => Some("fptrunc"),
            ("float", "double") => Some("fpext"),
            _ => None,
        };
        if let Some(inst) = float_inst {
            self.out.push_str(&format!(
                "  {} = {} {} {} to {}\n",
                temp, inst, value.llvm_ty, value.repr, expected_llvm_ty
            ));
            return Ok(LlValue::scalar(
                expected_llvm_ty.to_string(),
                temp,
                expected_ax_ty.to_string(),
            ));
        }

        Err(Diagnostic::error(
            "AX_CODEGEN_ERROR",
            format!("cannot coerce {} to {}", value.llvm_ty, expected_llvm_ty),
            self.function.span,
        ))
    }

    fn emit_async_context_store(
        &mut self,
        ctx: &str,
        idx: usize,
        ty: &TypeRef,
        value: &LlValue,
    ) -> AxResult<()> {
        match llvm_type(ty).as_str() {
            "i1" => {
                let raw = if value.llvm_ty == "i1" {
                    let temp = self.next_temp();
                    self.out
                        .push_str(&format!("  {} = zext i1 {} to i32\n", temp, value.repr));
                    temp
                } else {
                    value.repr.clone()
                };
                self.out.push_str(&format!(
                    "  call void @ax_async_context_set_i32(ptr {}, i32 {}, i32 {})\n",
                    ctx, idx, raw
                ));
            }
            "i32" => {
                self.out.push_str(&format!(
                    "  call void @ax_async_context_set_i32(ptr {}, i32 {}, i32 {})\n",
                    ctx, idx, value.repr
                ));
            }
            "i64" => {
                let raw = if value.llvm_ty == "i32" {
                    let temp = self.next_temp();
                    self.out
                        .push_str(&format!("  {} = sext i32 {} to i64\n", temp, value.repr));
                    temp
                } else {
                    value.repr.clone()
                };
                self.out.push_str(&format!(
                    "  call void @ax_async_context_set_i64(ptr {}, i32 {}, i64 {})\n",
                    ctx, idx, raw
                ));
            }
            "double" => {
                self.out.push_str(&format!(
                    "  call void @ax_async_context_set_f64(ptr {}, i32 {}, double {})\n",
                    ctx, idx, value.repr
                ));
            }
            _ => {
                self.out.push_str(&format!(
                    "  call void @ax_async_context_set_ptr(ptr {}, i32 {}, ptr {})\n",
                    ctx, idx, value.repr
                ));
            }
        }
        Ok(())
    }

    fn emit_expr(&mut self, expr: &Expr) -> AxResult<LlValue> {
        match expr {
            Expr::Int(value, _) => Ok(LlValue::scalar("i32", value.to_string(), "i32")),
            Expr::Float(value, _) => Ok(LlValue::scalar("double", value.to_string(), "f64")),
            Expr::Bool(value, _) => Ok(LlValue::scalar(
                "i1",
                if *value { "1" } else { "0" },
                "bool",
            )),
            Expr::Str(value, _) => {
                let global = self.module.add_string(value);
                Ok(LlValue::scalar("ptr", global, "str").with_const_str(value.clone()))
            }
            Expr::Ident(name, span) => {
                let value = self.locals.get(name).cloned().ok_or_else(|| {
                    Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("unknown local `{}`", name),
                        *span,
                    )
                })?;
                if let Some(ptr) = value.storage {
                    let temp = self.next_temp();
                    self.out.push_str(&format!(
                        "  {} = load {}, ptr {}\n",
                        temp, value.llvm_ty, ptr
                    ));
                    Ok(LlValue::scalar(value.llvm_ty, temp, value.ax_ty)
                        .with_optional_const_str(value.const_str))
                } else {
                    Ok(value)
                }
            }
            Expr::Member {
                object,
                field,
                span,
            } => {
                if let Expr::Ident(root, _) = object.as_ref() {
                    if let Some(index) = self.module.variant_index(root, field) {
                        return Ok(LlValue::scalar("i32", index.to_string(), root.clone()));
                    }
                }
                let value = self.emit_expr(object)?;
                value.fields.get(field).cloned().ok_or_else(|| {
                    Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("cannot codegen member `{}` on `{}`", field, value.ax_ty),
                        *span,
                    )
                })
            }
            Expr::Call { callee, args, span } => {
                let Some(name) = expr_name(callee) else {
                    return Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        "unsupported call expression",
                        *span,
                    ));
                };
                if matches!(name.as_str(), "io.println" | "io.print" | "io.eprintln") {
                    let arg = args.first().ok_or_else(|| {
                        Diagnostic::error(
                            "AX_CODEGEN_ERROR",
                            format!("{} expects one argument", name),
                            *span,
                        )
                    })?;
                    let value = self.emit_expr(arg)?;
                    let symbol = match name.as_str() {
                        "io.println" => "ax_io_println",
                        "io.print" => "ax_io_print",
                        _ => "ax_io_eprintln",
                    };
                    self.out
                        .push_str(&format!("  call void @{}(ptr {})\n", symbol, value.repr));
                    return Ok(LlValue::void());
                }
                let mut arg_values = Vec::new();
                let mut arg_ll_values = Vec::new();
                for arg in args {
                    let value = self.emit_expr(arg)?;
                    arg_values.push(format!("{} {}", value.llvm_ty, value.repr));
                    arg_ll_values.push(value);
                }
                match name.as_str() {
                    "io.read_line" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_io_read_line()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_text" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_text expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_text({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_text_or" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_text_or expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_text_or({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_text_limit" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_text_limit expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_text_limit({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_text_range" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_text_range expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_text_range({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_text_tail" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_text_tail expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_text_tail({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_lines" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_lines expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_lines({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_lines_json" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_lines_json expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_lines_json({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_jsonl" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_jsonl expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_jsonl({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_json_or" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_json_or expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_json_or({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_base64" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_base64 expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_base64({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_base64_range" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_base64_range expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_base64_range({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.read_base64_tail" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.read_base64_tail expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_read_base64_tail({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.write_text" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.write_text expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_write_text({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.write_text_atomic" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.write_text_atomic expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_write_text_atomic({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.write_json_atomic" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.write_json_atomic expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_write_json_atomic({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.write_base64" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.write_base64 expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_write_base64({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.append_text" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.append_text expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_append_text({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.append_jsonl" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.append_jsonl expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_append_jsonl({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.exists" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.exists expects one argument",
                                *span,
                            ));
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_fs_exists({})\n", raw, path));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "fs.remove" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.remove expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_fs_remove({})\n", path));
                        return Ok(LlValue::void());
                    }
                    "fs.remove_dir" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.remove_dir expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_fs_remove_dir({})\n", path));
                        return Ok(LlValue::void());
                    }
                    "fs.mkdir" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.mkdir expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_fs_mkdir({})\n", path));
                        return Ok(LlValue::void());
                    }
                    "fs.mkdir_all" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.mkdir_all expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_fs_mkdir_all({})\n", path));
                        return Ok(LlValue::void());
                    }
                    "fs.ensure_parent" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.ensure_parent expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_fs_ensure_parent({})\n", path));
                        return Ok(LlValue::void());
                    }
                    "fs.copy" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.copy expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_copy({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.rename" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.rename expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_fs_rename({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "fs.size" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.size expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i64 @ax_fs_size({})\n", temp, path));
                        return Ok(LlValue::scalar("i64", temp, "i64"));
                    }
                    "fs.is_file" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.is_file expects one argument",
                                *span,
                            ));
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_fs_is_file({})\n", raw, path));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "fs.is_dir" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.is_dir expects one argument",
                                *span,
                            ));
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_fs_is_dir({})\n", raw, path));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "fs.modified" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.modified expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i64 @ax_fs_modified({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("i64", temp, "i64"));
                    }
                    "fs.stat_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.stat_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_stat_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.cwd" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_fs_cwd()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.temp_dir" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_fs_temp_dir()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.list" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.list expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_fs_list({})\n", temp, path));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.list_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.list_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_list_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.list_stat_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.list_stat_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_list_stat_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.walk" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.walk expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_fs_walk({})\n", temp, path));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.walk_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.walk_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_walk_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.walk_stat_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.walk_stat_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_walk_stat_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.find" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.find expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_find({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "fs.glob" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "fs.glob expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_fs_glob({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.sha256_hex" | "crypto.sha256_json" => {
                        let Some(input) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects one argument", name),
                                *span,
                            ));
                        };
                        let runtime = if name == "crypto.sha256_json" {
                            "ax_crypto_sha256_json"
                        } else {
                            "ax_crypto_sha256_hex"
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @{}({})\n", temp, runtime, input));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.hmac_sha256_hex" | "crypto.hmac_sha256_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        let runtime = if name == "crypto.hmac_sha256_json" {
                            "ax_crypto_hmac_sha256_json"
                        } else {
                            "ax_crypto_hmac_sha256_hex"
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {})\n",
                            temp, runtime, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.sha256_verify_hex" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.sha256_verify_hex expects two arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_sha256_verify_hex({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.hmac_sha256_verify_hex" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.hmac_sha256_verify_hex expects three arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_hmac_sha256_verify_hex({}, {}, {})\n",
                            raw, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.hmac_sha256_file_hex" | "crypto.hmac_sha256_file_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        let runtime = if name == "crypto.hmac_sha256_file_json" {
                            "ax_crypto_hmac_sha256_file_json"
                        } else {
                            "ax_crypto_hmac_sha256_file_hex"
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {})\n",
                            temp, runtime, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.hmac_sha256_file_verify_hex" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.hmac_sha256_file_verify_hex expects three arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_hmac_sha256_file_verify_hex({}, {}, {})\n",
                            raw, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.hmac_sha256_file_range_hex" | "crypto.hmac_sha256_file_range_json" => {
                        if arg_values.len() < 4 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects four arguments", name),
                                *span,
                            ));
                        }
                        let runtime = if name == "crypto.hmac_sha256_file_range_json" {
                            "ax_crypto_hmac_sha256_file_range_json"
                        } else {
                            "ax_crypto_hmac_sha256_file_range_hex"
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {}, {}, {})\n",
                            temp,
                            runtime,
                            arg_values[0],
                            arg_values[1],
                            arg_values[2],
                            arg_values[3]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.hmac_sha256_file_range_verify_hex" => {
                        if arg_values.len() < 5 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.hmac_sha256_file_range_verify_hex expects five arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_hmac_sha256_file_range_verify_hex({}, {}, {}, {}, {})\n",
                            raw,
                            arg_values[0],
                            arg_values[1],
                            arg_values[2],
                            arg_values[3],
                            arg_values[4]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.sha256_file_hex" | "crypto.sha256_file_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects one argument", name),
                                *span,
                            ));
                        };
                        let runtime = if name == "crypto.sha256_file_json" {
                            "ax_crypto_sha256_file_json"
                        } else {
                            "ax_crypto_sha256_file_hex"
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @{}({})\n", temp, runtime, path));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.sha256_file_verify_hex" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.sha256_file_verify_hex expects two arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_sha256_file_verify_hex({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.sha256_file_range_hex" | "crypto.sha256_file_range_json" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects three arguments", name),
                                *span,
                            ));
                        }
                        let runtime = if name == "crypto.sha256_file_range_json" {
                            "ax_crypto_sha256_file_range_json"
                        } else {
                            "ax_crypto_sha256_file_range_hex"
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {}, {})\n",
                            temp, runtime, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.sha256_file_range_verify_hex" => {
                        if arg_values.len() < 4 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.sha256_file_range_verify_hex expects four arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_sha256_file_range_verify_hex({}, {}, {}, {})\n",
                            raw, arg_values[0], arg_values[1], arg_values[2], arg_values[3]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.base64_encode" => {
                        let Some(input) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.base64_encode expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_crypto_base64_encode({})\n",
                            temp, input
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.base64_decode" => {
                        let Some(input) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.base64_decode expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_crypto_base64_decode({})\n",
                            temp, input
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.constant_time_eq" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.constant_time_eq expects two arguments",
                                *span,
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_crypto_constant_time_eq({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "crypto.random_hex" => {
                        let Some(byte_count) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.random_hex expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_crypto_random_hex({})\n",
                            temp, byte_count
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.random_base64url" => {
                        let Some(byte_count) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "crypto.random_base64url expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_crypto_random_base64url({})\n",
                            temp, byte_count
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "crypto.uuid_v4" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_crypto_uuid_v4()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "env.get" => {
                        let Some(name) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.get expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_env_get({})\n", temp, name));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "env.has" => {
                        let Some(name) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.has expects one argument",
                                *span,
                            ));
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_env_has({})\n", raw, name));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "env.set" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.set expects two arguments",
                                *span,
                            ));
                        }
                        self.out.push_str(&format!(
                            "  call void @ax_env_set({}, {})\n",
                            arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    "env.get_or" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.get_or expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_env_get_or({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "env.snapshot_json" => {
                        let Some(prefix) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.snapshot_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_env_snapshot_json({})\n",
                            temp, prefix
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "env.load_dotenv" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.load_dotenv expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_env_load_dotenv({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "env.load_dotenv_json" => {
                        let Some(path) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "env.load_dotenv_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_env_load_dotenv_json({})\n",
                            temp, path
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "process.exec" => {
                        let Some(command) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "process.exec expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_process_exec({})\n",
                            temp, command
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "process.exec_limit" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "process.exec_limit expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_process_exec_limit({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "process.status" => {
                        let Some(command) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "process.status expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_process_status({})\n",
                            temp, command
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "process.run_json"
                    | "process.run_log_json"
                    | "process.run_lines_json"
                    | "process.run_log_lines_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        let symbol = match name.as_str() {
                            "process.run_log_json" => "ax_process_run_log_json",
                            "process.run_lines_json" => "ax_process_run_lines_json",
                            "process.run_log_lines_json" => "ax_process_run_log_lines_json",
                            _ => "ax_process_run_json",
                        };
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {})\n",
                            temp, symbol, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "cli.argc" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_cli_argc()\n", temp));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "cli.arg" => {
                        let Some(index) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "cli.arg expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_cli_arg({})\n", temp, index));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "cli.has" => {
                        let Some(name) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "cli.has expects one argument",
                                *span,
                            ));
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_cli_has({})\n", raw, name));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "cli.value" => {
                        let Some(name) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "cli.value expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_cli_value({})\n", temp, name));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "cli.value_or" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "cli.value_or expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_cli_value_or({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "cli.args_json" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_cli_args_json()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "cli.parse_json" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_cli_parse_json()\n", temp));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "http.get" => {
                        let Some(url) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "http.get expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_http_get({})\n", temp, url));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "http.post" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "http.post expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_http_post({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "http.get_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "http.get_json expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_http_get_json({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "http.post_json" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "http.post_json expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_http_post_json({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.escape" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.escape expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_escape({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.quote" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.quote expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_quote({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.pair" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.pair expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_pair({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.string_pair" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.string_pair expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_string_pair({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.set" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.set expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_set({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.string_set" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.string_set expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_string_set({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.remove" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.remove expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_remove({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.object" => {
                        let Some(fields) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.object expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_object({})\n",
                            temp, fields
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.array" => {
                        let Some(items) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.array expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_array({})\n",
                            temp, items
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.array_push" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.array_push expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_array_push({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.string_array_push" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.string_array_push expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_string_array_push({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.compact" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.compact expects one argument",
                                *span,
                            ));
                        };
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_compact({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.valid" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.valid expects one argument",
                                *span,
                            ));
                        };
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_json_valid({})\n", raw, value));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.get" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.get expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_get({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.query" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_query({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.get_or" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.get_or expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_get_or({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.query_or" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_or expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_query_or({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.has" | "json.query_has" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let symbol = match name.as_str() {
                            "json.has" => "ax_json_has",
                            _ => "ax_json_query_has",
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @{}({}, {})\n",
                            raw, symbol, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.int" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.int expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_i32_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar("i32", folded.to_string(), "i32"));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_int({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "json.bool" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.bool expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_bool({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.query_int" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_int expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_i32_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar("i32", folded.to_string(), "i32"));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_query_int({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "json.query_bool" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_bool expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_query_bool({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.contains" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.contains expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_contains({}, {}, {})\n",
                            raw, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.query_contains" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_contains expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_query_contains({}, {}, {})\n",
                            raw, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "json.kind" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.kind expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_kind({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.query_kind" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_kind expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_query_kind({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.len" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.len expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_i32_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar("i32", folded.to_string(), "i32"));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_len({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "json.query_len" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_len expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_i32_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar("i32", folded.to_string(), "i32"));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_json_query_len({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "json.at" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.at expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_at({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.query_at" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_at expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_query_at({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.keys" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.keys expects one argument",
                                *span,
                            ));
                        };
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_json_keys({})\n", temp, value));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.keys_json" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.keys_json expects one argument",
                                *span,
                            ));
                        };
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_keys_json({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "json.query_keys_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "json.query_keys_json expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_json_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_json_query_keys_json({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.len" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.len expects one argument",
                                *span,
                            ));
                        };
                        if let Some(value) = arg_ll_values
                            .first()
                            .and_then(|value| value.const_str.as_ref())
                        {
                            return Ok(LlValue::scalar("i32", value.len().to_string(), "i32"));
                        }
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i32 @ax_str_len({})\n", temp, value));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "str.contains" | "str.starts_with" | "str.ends_with" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        if let (Some(value), Some(needle)) = (
                            arg_ll_values
                                .first()
                                .and_then(|value| value.const_str.as_ref()),
                            arg_ll_values
                                .get(1)
                                .and_then(|value| value.const_str.as_ref()),
                        ) {
                            let folded = match name.as_str() {
                                "str.contains" => value.contains(needle),
                                "str.starts_with" => value.starts_with(needle),
                                _ => value.ends_with(needle),
                            };
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let symbol = match name.as_str() {
                            "str.contains" => "ax_str_contains",
                            "str.starts_with" => "ax_str_starts_with",
                            _ => "ax_str_ends_with",
                        };
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @{}({}, {})\n",
                            raw, symbol, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "str.index_of" | "str.count" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        if let (Some(value), Some(needle)) = (
                            arg_ll_values
                                .first()
                                .and_then(|value| value.const_str.as_ref()),
                            arg_ll_values
                                .get(1)
                                .and_then(|value| value.const_str.as_ref()),
                        ) {
                            let folded = if name == "str.index_of" {
                                value.find(needle).map(|index| index as i32).unwrap_or(-1)
                            } else if needle.is_empty() {
                                0
                            } else {
                                value.matches(needle).count() as i32
                            };
                            return Ok(LlValue::scalar("i32", folded.to_string(), "i32"));
                        }
                        let symbol = match name.as_str() {
                            "str.index_of" => "ax_str_index_of",
                            _ => "ax_str_count",
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @{}({}, {})\n",
                            temp, symbol, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "str.trim" | "str.upper" | "str.lower" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects one argument", name),
                                *span,
                            ));
                        };
                        if let Some(folded) = arg_ll_values
                            .first()
                            .and_then(|value| value.const_str.as_ref())
                            .map(|value| match name.as_str() {
                                "str.trim" => value.trim().to_string(),
                                "str.upper" => value.to_ascii_uppercase(),
                                _ => value.to_ascii_lowercase(),
                            })
                        {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let symbol = match name.as_str() {
                            "str.trim" => "ax_str_trim",
                            "str.upper" => "ax_str_upper",
                            _ => "ax_str_lower",
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @{}({})\n", temp, symbol, value));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.concat" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.concat expects two arguments",
                                *span,
                            ));
                        }
                        if let (Some(left), Some(right)) = (
                            arg_ll_values
                                .first()
                                .and_then(|value| value.const_str.as_ref()),
                            arg_ll_values
                                .get(1)
                                .and_then(|value| value.const_str.as_ref()),
                        ) {
                            let folded = format!("{}{}", left, right);
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_concat({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.repeat" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.repeat expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_repeat({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.replace" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.replace expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_replace({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.slice" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.slice expects three arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_slice({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.split_json" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.split_json expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_split_json({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.lines_json" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.lines_json expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_lines_json({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.from_i64" => {
                        let Some(value) = arg_ll_values.first().cloned() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.from_i64 expects one argument",
                                *span,
                            ));
                        };
                        let value = self.coerce_value_to(value, "i64", "i64")?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_from_i64(i64 {})\n",
                            temp, value.repr
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.parse_i64" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.parse_i64 expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i64 @ax_str_parse_i64({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("i64", temp, "i64"));
                    }
                    "str.parse_i32" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.parse_i32 expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_str_parse_i32({})\n",
                            temp, value
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    "str.token" | "str.token_upper" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects two arguments", name),
                                *span,
                            ));
                        }
                        let symbol = match name.as_str() {
                            "str.token_upper" => "ax_str_token_upper",
                            _ => "ax_str_token",
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @{}({}, {})\n",
                            temp, symbol, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "str.line" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "str.line expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_str_line({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "path.normalize" | "path.basename" | "path.dirname" | "path.extname"
                    | "path.stem" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects one argument", name),
                                *span,
                            ));
                        };
                        if let Some(value) = arg_ll_values
                            .first()
                            .and_then(|value| value.const_str.as_ref())
                        {
                            if let Some(folded) = fold_path_string_call(&name, value) {
                                let global = self.module.add_string(&folded);
                                return Ok(
                                    LlValue::scalar("ptr", global, "str").with_const_str(folded)
                                );
                            }
                        }
                        let symbol = match name.as_str() {
                            "path.normalize" => "ax_path_normalize",
                            "path.basename" => "ax_path_basename",
                            "path.dirname" => "ax_path_dirname",
                            "path.extname" => "ax_path_extname",
                            _ => "ax_path_stem",
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @{}({})\n", temp, symbol, value));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "path.join" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "path.join expects two arguments",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_path_join({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "path.is_absolute" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "path.is_absolute expects one argument",
                                *span,
                            ));
                        };
                        if let Some(value) = arg_ll_values
                            .first()
                            .and_then(|value| value.const_str.as_ref())
                        {
                            let folded = path_is_absolute_const(value);
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_path_is_absolute({})\n",
                            raw, value
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "url.encode" | "url.decode" | "url.path" | "url.host" | "url.scheme"
                    | "url.query_json" => {
                        let Some(value) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("{} expects one argument", name),
                                *span,
                            ));
                        };
                        if let Some(folded) = fold_url_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let symbol = match name.as_str() {
                            "url.encode" => "ax_url_encode",
                            "url.decode" => "ax_url_decode",
                            "url.path" => "ax_url_path",
                            "url.host" => "ax_url_host",
                            "url.query_json" => "ax_url_query_json",
                            _ => "ax_url_scheme",
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @{}({})\n", temp, symbol, value));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "url.query_get" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "url.query_get expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_url_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_url_query_get({}, {})\n",
                            temp, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "url.query_or" => {
                        if arg_values.len() < 3 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "url.query_or expects three arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_url_string_call(&name, &arg_ll_values) {
                            let global = self.module.add_string(&folded);
                            return Ok(LlValue::scalar("ptr", global, "str").with_const_str(folded));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_url_query_or({}, {}, {})\n",
                            temp, arg_values[0], arg_values[1], arg_values[2]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "url.query_has" => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "url.query_has expects two arguments",
                                *span,
                            ));
                        }
                        if let Some(folded) = fold_url_bool_call(&name, &arg_ll_values) {
                            return Ok(LlValue::scalar(
                                "i1",
                                if folded { "1" } else { "0" },
                                "bool",
                            ));
                        }
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_url_query_has({}, {})\n",
                            raw, arg_values[0], arg_values[1]
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    "time.now" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i64 @ax_time_now()\n", temp));
                        return Ok(LlValue::scalar("i64", temp, "i64"));
                    }
                    "time.now_ms" => {
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call i64 @ax_time_now_ms()\n", temp));
                        return Ok(LlValue::scalar("i64", temp, "i64"));
                    }
                    "time.iso_utc" => {
                        let Some(epoch_seconds) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "time.iso_utc expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_time_iso_utc({})\n",
                            temp, epoch_seconds
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    "time.sleep_ms" => {
                        let Some(milliseconds) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "time.sleep_ms expects one argument",
                                *span,
                            ));
                        };
                        self.out.push_str(&format!(
                            "  call void @ax_time_sleep_ms({})\n",
                            milliseconds
                        ));
                        return Ok(LlValue::void());
                    }
                    "tcp.listen" => {
                        if arg_values.len() == 1 {
                            let port = &arg_values[0];
                            let temp = self.next_temp();
                            self.out.push_str(&format!(
                                "  {} = call ptr @ax_tcp_listen({})\n",
                                temp, port
                            ));
                            return Ok(LlValue::scalar("ptr", temp, "TcpServer"));
                        }
                        if arg_values.len() == 2 {
                            let host = &arg_values[0];
                            let port = &arg_values[1];
                            let temp = self.next_temp();
                            self.out.push_str(&format!(
                                "  {} = call ptr @ax_tcp_listen_on({}, {})\n",
                                temp, host, port
                            ));
                            return Ok(LlValue::scalar("ptr", temp, "TcpServer"));
                        }
                        return Err(Diagnostic::error(
                            "AX_CODEGEN_ERROR",
                            "tcp.listen expects one or two arguments",
                            *span,
                        ));
                    }
                    "tcp.connect" => {
                        let Some(host) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "tcp.connect expects two arguments",
                                *span,
                            ));
                        };
                        let Some(port) = arg_values.get(1) else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "tcp.connect expects two arguments",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_tcp_connect({}, {})\n",
                            temp, host, port
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "TcpConn"));
                    }
                    "tcp.serve_text" => {
                        if arg_values.len() < 4 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "tcp.serve_text expects four arguments",
                                *span,
                            ));
                        }
                        let Some(handler_name) = arg_ll_values
                            .get(2)
                            .and_then(|value| value.const_str.as_ref())
                        else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "tcp.serve_text handler must be a string literal",
                                *span,
                            ));
                        };
                        let Some(handler) = self.module.ir.semantic.functions.get(handler_name)
                        else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                format!("tcp.serve_text handler `{}` was not found", handler_name),
                                *span,
                            ));
                        };
                        if handler.params.len() != 2
                            || handler.params[0].1.name != "Map"
                            || handler.params[1].1.name != "str"
                            || handler.ret.name != "str"
                        {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "tcp.serve_text handler must have type (Map, str) -> str",
                                *span,
                            ));
                        }
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_tcp_serve_text({}, {}, ptr @{}, {})\n",
                            temp,
                            arg_values[0],
                            arg_values[1],
                            sanitize_ident(handler_name),
                            arg_values[3]
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    name if name.ends_with(".accept") => {
                        let receiver = self.emit_method_receiver(callee, "accept", *span)?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_tcp_accept(ptr {})\n",
                            temp, receiver.repr
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "TcpConn"));
                    }
                    name if name.ends_with(".read_text") => {
                        let Some(limit) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "read_text expects one argument",
                                *span,
                            ));
                        };
                        let receiver = self.emit_method_receiver(callee, "read_text", *span)?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_tcp_read_text(ptr {}, {})\n",
                            temp, receiver.repr, limit
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    name if name.ends_with(".write_text") => {
                        let Some(text) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "write_text expects one argument",
                                *span,
                            ));
                        };
                        let receiver = self.emit_method_receiver(callee, "write_text", *span)?;
                        self.out.push_str(&format!(
                            "  call void @ax_tcp_write_text(ptr {}, {})\n",
                            receiver.repr, text
                        ));
                        return Ok(LlValue::void());
                    }
                    name if name.ends_with(".request_text") => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "request_text expects two arguments",
                                *span,
                            ));
                        }
                        let receiver = self.emit_method_receiver(callee, "request_text", *span)?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_tcp_request_text(ptr {}, {}, {})\n",
                            temp, receiver.repr, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    name if name.ends_with(".close") => {
                        let receiver = self.emit_method_receiver(callee, "close", *span)?;
                        self.out.push_str(&format!(
                            "  call void @ax_tcp_close(ptr {})\n",
                            receiver.repr
                        ));
                        return Ok(LlValue::void());
                    }
                    "map.new" => {
                        let Some(capacity) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "map.new expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_map_new({})\n",
                            temp, capacity
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "Map"));
                    }
                    name if name.ends_with(".set") => {
                        if arg_values.len() < 2 {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "map.set expects two arguments",
                                *span,
                            ));
                        }
                        let receiver = self.emit_method_receiver(callee, "set", *span)?;
                        self.out.push_str(&format!(
                            "  call void @ax_map_set(ptr {}, {}, {})\n",
                            receiver.repr, arg_values[0], arg_values[1]
                        ));
                        return Ok(LlValue::void());
                    }
                    name if name.ends_with(".get") => {
                        let Some(key) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "map.get expects one argument",
                                *span,
                            ));
                        };
                        let receiver = self.emit_method_receiver(callee, "get", *span)?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_map_get(ptr {}, {})\n",
                            temp, receiver.repr, key
                        ));
                        return Ok(LlValue::scalar("ptr", temp, "str"));
                    }
                    name if name.ends_with(".has") => {
                        let Some(key) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "map.has expects one argument",
                                *span,
                            ));
                        };
                        let receiver = self.emit_method_receiver(callee, "has", *span)?;
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_map_has(ptr {}, {})\n",
                            raw, receiver.repr, key
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    name if name.ends_with(".del") => {
                        let Some(key) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "map.del expects one argument",
                                *span,
                            ));
                        };
                        let receiver = self.emit_method_receiver(callee, "del", *span)?;
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_map_del(ptr {}, {})\n",
                            raw, receiver.repr, key
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        return Ok(LlValue::scalar("i1", temp, "bool"));
                    }
                    name if name.ends_with(".len") => {
                        let receiver = self.emit_method_receiver(callee, "len", *span)?;
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_map_len(ptr {})\n",
                            temp, receiver.repr
                        ));
                        return Ok(LlValue::scalar("i32", temp, "i32"));
                    }
                    name if name.ends_with(".clear") => {
                        let receiver = self.emit_method_receiver(callee, "clear", *span)?;
                        self.out.push_str(&format!(
                            "  call void @ax_map_clear(ptr {})\n",
                            receiver.repr
                        ));
                        return Ok(LlValue::void());
                    }
                    "heap.alloc" => {
                        let Some(size) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "heap.alloc expects one argument",
                                *span,
                            ));
                        };
                        let temp = self.next_temp();
                        self.out
                            .push_str(&format!("  {} = call ptr @ax_heap_alloc({})\n", temp, size));
                        return Ok(LlValue::scalar("ptr", temp, "ptr"));
                    }
                    "heap.free" => {
                        let Some(ptr) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "heap.free expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_heap_free({})\n", ptr));
                        return Ok(LlValue::void());
                    }
                    "async.cancel" => {
                        let Some(future) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "async.cancel expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_async_cancel({})\n", future));
                        return Ok(LlValue::void());
                    }
                    "async.detach" => {
                        let Some(future) = arg_values.first() else {
                            return Err(Diagnostic::error(
                                "AX_CODEGEN_ERROR",
                                "async.detach expects one argument",
                                *span,
                            ));
                        };
                        self.out
                            .push_str(&format!("  call void @ax_async_detach({})\n", future));
                        return Ok(LlValue::void());
                    }
                    "panic" => {
                        let message = if let Some(arg) = arg_values.first() {
                            arg.clone()
                        } else {
                            format!("ptr {}", self.module.add_string("panic"))
                        };
                        self.out
                            .push_str(&format!("  call void @ax_panic({})\n", message));
                        return Ok(LlValue::void());
                    }
                    _ => {}
                }
                if let Some(value) =
                    const_eval_user_call_i32(&self.module.ir.program, &name, &arg_ll_values)
                {
                    return Ok(LlValue::scalar("i32", value.to_string(), "i32"));
                }
                let Some(function) = self.module.ir.semantic.functions.get(&name) else {
                    if name.contains('.') {
                        if let Some(symbol) = self.module.external_native_symbol_for_call(&name) {
                            self.module.external_pack_decls.insert(symbol.clone());
                            self.out.push_str(&format!("  call void @{}()\n", symbol));
                            return Ok(LlValue::void());
                        }
                        return Err(Diagnostic::error(
                            "AX_CODEGEN_ERROR",
                            format!(
                                "external operation `{}` requires pack native sources",
                                name
                            ),
                            *span,
                        )
                        .help("add `native = [\"native.c\"]` to the pack manifest and provide the expected ax_pack_* symbol"));
                    }
                    return Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("unknown function `{}`", name),
                        *span,
                    ));
                };
                let ret_ty = llvm_type(&function.ret);
                if function.is_async {
                    if function.params.len() != arg_ll_values.len() {
                        return Err(Diagnostic::error(
                            "AX_CODEGEN_ERROR",
                            format!(
                                "async function `{}` expects {} argument(s), got {}",
                                name,
                                function.params.len(),
                                arg_ll_values.len()
                            ),
                            *span,
                        ));
                    }
                    let entry = async_entry_name(&name);
                    let temp = self.next_temp();
                    if arg_ll_values.is_empty() {
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_async_spawn(ptr @{})\n",
                            temp, entry
                        ));
                    } else {
                        let ctx = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_async_context_new(i32 {})\n",
                            ctx,
                            arg_ll_values.len()
                        ));
                        for (idx, ((_, ty), value)) in
                            function.params.iter().zip(arg_ll_values.iter()).enumerate()
                        {
                            self.emit_async_context_store(&ctx, idx, ty, value)?;
                        }
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_async_spawn_with_context(ptr @{}, ptr {})\n",
                            temp, entry, ctx
                        ));
                    }
                    return Ok(LlValue::scalar("ptr", temp, future_ax_ty(&function.ret)));
                }
                if function.params.len() != arg_ll_values.len() {
                    return Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!(
                            "function `{}` expects {} argument(s), got {}",
                            name,
                            function.params.len(),
                            arg_ll_values.len()
                        ),
                        *span,
                    ));
                }
                let mut call_args = Vec::new();
                for ((_, expected), value) in function.params.iter().zip(arg_ll_values.into_iter())
                {
                    let value = self.coerce_value(value, expected)?;
                    call_args.push(format!("{} {}", value.llvm_ty, value.repr));
                }
                if ret_ty == "void" {
                    self.out.push_str(&format!(
                        "  call void @{}({})\n",
                        sanitize_ident(&name),
                        call_args.join(", ")
                    ));
                    Ok(LlValue::void())
                } else {
                    let temp = self.next_temp();
                    self.out.push_str(&format!(
                        "  {} = call {} @{}({})\n",
                        temp,
                        ret_ty,
                        sanitize_ident(&name),
                        call_args.join(", ")
                    ));
                    Ok(LlValue::scalar(ret_ty, temp, function.ret.name.clone()))
                }
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let left = self.emit_expr(left)?;
                let mut right = self.emit_expr(right)?;
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                    && left.ax_ty == "str"
                    && right.ax_ty == "str"
                {
                    let raw = self.next_temp();
                    let temp = self.next_temp();
                    self.out.push_str(&format!(
                        "  {} = call i32 @ax_str_eq({} {}, {} {})\n",
                        raw, left.llvm_ty, left.repr, right.llvm_ty, right.repr
                    ));
                    let cmp = if matches!(op, BinaryOp::Eq) {
                        "icmp ne"
                    } else {
                        "icmp eq"
                    };
                    self.out
                        .push_str(&format!("  {} = {} i32 {}, 0\n", temp, cmp, raw));
                    return Ok(LlValue::scalar("i1", temp, "bool"));
                }
                if left.llvm_ty != right.llvm_ty
                    && can_coerce_numeric(&right.llvm_ty, &left.llvm_ty)
                {
                    right = self.coerce_value_to(right, &left.llvm_ty, &left.ax_ty)?;
                }
                let temp = self.next_temp();
                let (inst, result_ty) = match op {
                    BinaryOp::Add => ("add", left.llvm_ty.as_str()),
                    BinaryOp::Sub => ("sub", left.llvm_ty.as_str()),
                    BinaryOp::Mul => ("mul", left.llvm_ty.as_str()),
                    BinaryOp::Div => ("sdiv", left.llvm_ty.as_str()),
                    BinaryOp::Mod => ("srem", left.llvm_ty.as_str()),
                    BinaryOp::And => ("and", "i1"),
                    BinaryOp::Or => ("or", "i1"),
                    BinaryOp::Eq => ("icmp eq", "i1"),
                    BinaryOp::Ne => ("icmp ne", "i1"),
                    BinaryOp::Lt => ("icmp slt", "i1"),
                    BinaryOp::Le => ("icmp sle", "i1"),
                    BinaryOp::Gt => ("icmp sgt", "i1"),
                    BinaryOp::Ge => ("icmp sge", "i1"),
                };
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                ) {
                    self.out.push_str(&format!(
                        "  {} = {} {} {}, {}\n",
                        temp, inst, left.llvm_ty, left.repr, right.repr
                    ));
                    Ok(LlValue::scalar(result_ty, temp, "bool"))
                } else {
                    self.out.push_str(&format!(
                        "  {} = {} {} {}, {}\n",
                        temp, inst, left.llvm_ty, left.repr, right.repr
                    ));
                    Ok(LlValue::scalar(result_ty, temp, left.ax_ty))
                }
            }
            Expr::Unary { op, expr, .. } => {
                let value = self.emit_expr(expr)?;
                let temp = self.next_temp();
                match op {
                    UnaryOp::Not => {
                        self.out
                            .push_str(&format!("  {} = xor i1 {}, 1\n", temp, value.repr));
                        Ok(LlValue::scalar("i1", temp, "bool"))
                    }
                    UnaryOp::Neg => {
                        self.out.push_str(&format!(
                            "  {} = sub {} 0, {}\n",
                            temp, value.llvm_ty, value.repr
                        ));
                        Ok(LlValue::scalar(value.llvm_ty, temp, value.ax_ty))
                    }
                }
            }
            Expr::Await { expr, span } => {
                let future = self.emit_expr(expr)?;
                let Some(inner) = future_inner_ax_ty(&future.ax_ty) else {
                    return Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("cannot await {}", future.ax_ty),
                        *span,
                    ));
                };
                match llvm_type_name(&inner).as_str() {
                    "void" => {
                        self.out.push_str(&format!(
                            "  call void @ax_async_await_void(ptr {})\n",
                            future.repr
                        ));
                        Ok(LlValue::void())
                    }
                    "i32" => {
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_async_await_i32(ptr {})\n",
                            temp, future.repr
                        ));
                        Ok(LlValue::scalar("i32", temp, inner))
                    }
                    "i64" => {
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i64 @ax_async_await_i64(ptr {})\n",
                            temp, future.repr
                        ));
                        Ok(LlValue::scalar("i64", temp, inner))
                    }
                    "i1" => {
                        let raw = self.next_temp();
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call i32 @ax_async_await_i32(ptr {})\n",
                            raw, future.repr
                        ));
                        self.out
                            .push_str(&format!("  {} = icmp ne i32 {}, 0\n", temp, raw));
                        Ok(LlValue::scalar("i1", temp, inner))
                    }
                    "double" => {
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call double @ax_async_await_f64(ptr {})\n",
                            temp, future.repr
                        ));
                        Ok(LlValue::scalar("double", temp, inner))
                    }
                    "ptr" => {
                        let temp = self.next_temp();
                        self.out.push_str(&format!(
                            "  {} = call ptr @ax_async_await_ptr(ptr {})\n",
                            temp, future.repr
                        ));
                        Ok(LlValue::scalar("ptr", temp, inner))
                    }
                    _ => Err(Diagnostic::error(
                        "AX_CODEGEN_ERROR",
                        format!("unsupported async result {}", inner),
                        *span,
                    )),
                }
            }
            Expr::RecordInit { name, fields, .. } => {
                let mut values = BTreeMap::new();
                for (field, expr) in fields {
                    let value = self.emit_expr(expr)?;
                    let value = if let Some(expected) = self
                        .module
                        .ir
                        .semantic
                        .records
                        .get(name)
                        .and_then(|record| record.fields.get(field))
                        .cloned()
                    {
                        self.coerce_value(value, &expected)?
                    } else {
                        value
                    };
                    values.insert(field.clone(), value);
                }
                Ok(LlValue::record(name, values))
            }
        }
    }

    fn emit_default_return(&mut self) {
        let ret_ty = llvm_type(&self.function.ret);
        match ret_ty.as_str() {
            "void" => self.out.push_str("  ret void\n"),
            "i64" => self.out.push_str("  ret i64 0\n"),
            "i1" => self.out.push_str("  ret i1 0\n"),
            "double" => self.out.push_str("  ret double 0.0\n"),
            "float" => self.out.push_str("  ret float 0.0\n"),
            "ptr" => self.out.push_str("  ret ptr null\n"),
            _ => self.out.push_str("  ret i32 0\n"),
        }
        self.terminated = true;
    }

    fn next_temp(&mut self) -> String {
        let value = format!("%t{}", self.temp);
        self.temp += 1;
        value
    }

    fn next_label(&mut self, prefix: &str) -> String {
        let value = format!("{}.{}", prefix, self.label);
        self.label += 1;
        value
    }

    fn emit_method_receiver(
        &mut self,
        callee: &Expr,
        method: &str,
        span: Span,
    ) -> AxResult<LlValue> {
        let Expr::Member { object, field, .. } = callee else {
            return Err(Diagnostic::error(
                "AX_CODEGEN_ERROR",
                format!("{} expects a method receiver", method),
                span,
            ));
        };
        if field != method {
            return Err(Diagnostic::error(
                "AX_CODEGEN_ERROR",
                format!("expected method `{}`", method),
                span,
            ));
        }
        self.emit_expr(object)
    }
}

fn expr_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Member { object, field, .. } => Some(format!("{}.{}", expr_name(object)?, field)),
        _ => None,
    }
}

fn llvm_type(ty: &TypeRef) -> String {
    match ty.name.as_str() {
        "void" => "void".to_string(),
        "bool" => "i1".to_string(),
        "i8" | "u8" => "i8".to_string(),
        "i16" | "u16" => "i16".to_string(),
        "i32" | "u32" => "i32".to_string(),
        "i64" | "u64" => "i64".to_string(),
        "f32" => "float".to_string(),
        "f64" => "double".to_string(),
        "str" => "ptr".to_string(),
        "ptr" | "TcpServer" | "TcpConn" | "Map" => "ptr".to_string(),
        _ => "i32".to_string(),
    }
}

fn integer_llvm_width(llvm_ty: &str) -> Option<u8> {
    match llvm_ty {
        "i1" => Some(1),
        "i8" => Some(8),
        "i16" => Some(16),
        "i32" => Some(32),
        "i64" => Some(64),
        _ => None,
    }
}

fn is_unsigned_ax_type(ax_ty: &str) -> bool {
    matches!(ax_ty, "u8" | "u16" | "u32" | "u64")
}

fn can_coerce_numeric(from_llvm_ty: &str, to_llvm_ty: &str) -> bool {
    (integer_llvm_width(from_llvm_ty).is_some() && integer_llvm_width(to_llvm_ty).is_some())
        || matches!(
            (from_llvm_ty, to_llvm_ty),
            ("double", "float") | ("float", "double")
        )
}

fn path_is_sep(ch: char) -> bool {
    ch == '/' || ch == '\\'
}

fn path_has_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_is_absolute_const(value: &str) -> bool {
    value.chars().next().is_some_and(path_is_sep)
        || (path_has_drive(value) && value.chars().nth(2).is_some_and(path_is_sep))
}

fn path_normalize_const(value: &str) -> String {
    if value.is_empty() {
        return ".".to_string();
    }
    let normalized = value.replace('\\', "/");
    let mut prefix = "";
    let mut rest = normalized.as_str();
    let mut absolute = false;
    if path_has_drive(rest) {
        prefix = &rest[..2];
        rest = &rest[2..];
        if rest.starts_with('/') {
            absolute = true;
            rest = rest.trim_start_matches('/');
        }
    } else if rest.starts_with('/') {
        absolute = true;
        rest = rest.trim_start_matches('/');
    }

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if let Some(last) = parts.last() {
                if *last != ".." {
                    parts.pop();
                    continue;
                }
            }
            if !absolute {
                parts.push(part);
            }
            continue;
        }
        parts.push(part);
    }

    let mut out = String::new();
    out.push_str(prefix);
    if absolute {
        out.push('/');
    }
    for (idx, part) in parts.iter().enumerate() {
        if (!out.is_empty() && !out.ends_with('/')) || idx > 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        if absolute {
            "/".to_string()
        } else {
            ".".to_string()
        }
    } else {
        out
    }
}

fn path_basename_const(value: &str) -> String {
    let mut normalized = path_normalize_const(value);
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn path_dirname_const(value: &str) -> String {
    let mut normalized = path_normalize_const(value);
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if let Some(index) = normalized.rfind('/') {
        if index == 0 {
            "/".to_string()
        } else {
            normalized[..index].to_string()
        }
    } else {
        ".".to_string()
    }
}

fn path_extname_const(value: &str) -> String {
    let base = path_basename_const(value);
    if let Some(index) = base.rfind('.') {
        if index > 0 {
            return base[index..].to_string();
        }
    }
    String::new()
}

fn path_stem_const(value: &str) -> String {
    let base = path_basename_const(value);
    if let Some(index) = base.rfind('.') {
        if index > 0 {
            return base[..index].to_string();
        }
    }
    base
}

fn fold_path_string_call(name: &str, value: &str) -> Option<String> {
    match name {
        "path.normalize" => Some(path_normalize_const(value)),
        "path.basename" => Some(path_basename_const(value)),
        "path.dirname" => Some(path_dirname_const(value)),
        "path.extname" => Some(path_extname_const(value)),
        "path.stem" => Some(path_stem_const(value)),
        _ => None,
    }
}

#[derive(Clone, Debug)]
enum ConstJson {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<ConstJson>),
    Object(Vec<(String, ConstJson)>),
}

struct ConstJsonParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> ConstJsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(mut self) -> Option<ConstJson> {
        let value = self.parse_value()?;
        self.skip_ws();
        (self.pos == self.input.len()).then_some(value)
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Option<ConstJson> {
        self.skip_ws();
        match self.peek()? {
            b'n' => self.consume_literal("null").then_some(ConstJson::Null),
            b't' => self
                .consume_literal("true")
                .then_some(ConstJson::Bool(true)),
            b'f' => self
                .consume_literal("false")
                .then_some(ConstJson::Bool(false)),
            b'"' => self.parse_string().map(ConstJson::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number().map(ConstJson::Number),
            _ => None,
        }
    }

    fn consume_literal(&mut self, literal: &str) -> bool {
        if self.input[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            true
        } else {
            false
        }
    }

    fn consume_byte(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if !self.consume_byte(b'"') {
            return None;
        }
        let mut out = String::new();
        while self.pos < self.input.len() {
            let ch = self.input[self.pos..].chars().next()?;
            self.pos += ch.len_utf8();
            match ch {
                '"' => return Some(out),
                '\\' => {
                    let escaped = self.input[self.pos..].chars().next()?;
                    self.pos += escaped.len_utf8();
                    match escaped {
                        '"' | '\\' | '/' => out.push(escaped),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => return None,
                        _ => return None,
                    }
                }
                ch if (ch as u32) < 0x20 => return None,
                _ => out.push(ch),
            }
        }
        None
    }

    fn parse_number(&mut self) -> Option<String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return None;
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        Some(self.input[start..self.pos].to_string())
    }

    fn parse_array(&mut self) -> Option<ConstJson> {
        if !self.consume_byte(b'[') {
            return None;
        }
        let mut items = Vec::new();
        self.skip_ws();
        if self.consume_byte(b']') {
            return Some(ConstJson::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            if self.consume_byte(b']') {
                return Some(ConstJson::Array(items));
            }
            if !self.consume_byte(b',') {
                return None;
            }
        }
    }

    fn parse_object(&mut self) -> Option<ConstJson> {
        if !self.consume_byte(b'{') {
            return None;
        }
        let mut fields = Vec::new();
        self.skip_ws();
        if self.consume_byte(b'}') {
            return Some(ConstJson::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if !self.consume_byte(b':') {
                return None;
            }
            let value = self.parse_value()?;
            fields.push((key, value));
            self.skip_ws();
            if self.consume_byte(b'}') {
                return Some(ConstJson::Object(fields));
            }
            if !self.consume_byte(b',') {
                return None;
            }
        }
    }
}

fn parse_const_json(value: &str) -> Option<ConstJson> {
    if value.contains("\\u") {
        return None;
    }
    ConstJsonParser::new(value).parse()
}

fn json_compact_const(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

fn json_quote_const(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn const_json_compact(value: &ConstJson) -> String {
    match value {
        ConstJson::Null => "null".to_string(),
        ConstJson::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        ConstJson::Number(value) => value.clone(),
        ConstJson::String(value) => json_quote_const(value),
        ConstJson::Array(items) => {
            let inner = items
                .iter()
                .map(const_json_compact)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]", inner)
        }
        ConstJson::Object(fields) => {
            let inner = fields
                .iter()
                .map(|(key, value)| {
                    format!("{}:{}", json_quote_const(key), const_json_compact(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", inner)
        }
    }
}

fn const_json_kind(value: &ConstJson) -> &'static str {
    match value {
        ConstJson::Null => "null",
        ConstJson::Bool(_) => "bool",
        ConstJson::Number(_) => "number",
        ConstJson::String(_) => "string",
        ConstJson::Array(_) => "array",
        ConstJson::Object(_) => "object",
    }
}

fn const_json_string_result(value: &ConstJson) -> String {
    match value {
        ConstJson::String(value) => value.clone(),
        _ => const_json_compact(value),
    }
}

fn const_json_get_key<'a>(value: &'a ConstJson, key: &str) -> Option<&'a ConstJson> {
    match value {
        ConstJson::Object(fields) => fields
            .iter()
            .find_map(|(field, value)| (field == key).then_some(value)),
        _ => None,
    }
}

fn const_json_query<'a>(value: &'a ConstJson, path: &str) -> Option<&'a ConstJson> {
    if path.is_empty() {
        return None;
    }
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = match current {
            ConstJson::Object(_) => const_json_get_key(current, segment)?,
            ConstJson::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
    }
    Some(current)
}

fn const_json_len(value: &ConstJson) -> i32 {
    match value {
        ConstJson::String(value) => value.len() as i32,
        ConstJson::Array(items) => items.len() as i32,
        ConstJson::Object(fields) => fields.len() as i32,
        _ => 0,
    }
}

fn const_json_i32(value: &ConstJson) -> i32 {
    match value {
        ConstJson::Number(value) => value.parse::<i32>().unwrap_or(0),
        _ => 0,
    }
}

fn const_json_bool(value: &ConstJson) -> bool {
    matches!(value, ConstJson::Bool(true))
}

fn const_json_array_contains(value: &ConstJson, needle: &str) -> bool {
    match value {
        ConstJson::Array(items) => items
            .iter()
            .any(|item| const_json_string_result(item) == needle),
        _ => false,
    }
}

fn const_json_keys(value: &ConstJson) -> String {
    match value {
        ConstJson::Object(fields) => fields
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn const_json_keys_json(value: &ConstJson) -> String {
    match value {
        ConstJson::Object(fields) => {
            let items = fields
                .iter()
                .map(|(key, _)| json_quote_const(key))
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]", items)
        }
        _ => "[]".to_string(),
    }
}

fn const_str_arg(args: &[LlValue], index: usize) -> Option<&str> {
    args.get(index)?.const_str.as_deref()
}

fn const_i32_arg(args: &[LlValue], index: usize) -> Option<i32> {
    let value = args.get(index)?;
    (value.llvm_ty == "i32").then(|| value.repr.parse::<i32>().ok())?
}

fn fold_json_string_call(name: &str, args: &[LlValue]) -> Option<String> {
    match name {
        "json.compact" => const_str_arg(args, 0).map(json_compact_const),
        "json.get" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(
                const_json_get_key(&json, key)
                    .map(const_json_string_result)
                    .unwrap_or_default(),
            )
        }
        "json.query" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(
                const_json_query(&json, path)
                    .map(const_json_string_result)
                    .unwrap_or_default(),
            )
        }
        "json.get_or" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            let fallback = const_str_arg(args, 2)?;
            Some(
                const_json_get_key(&json, key)
                    .map(const_json_string_result)
                    .unwrap_or_else(|| fallback.to_string()),
            )
        }
        "json.query_or" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            let fallback = const_str_arg(args, 2)?;
            Some(
                const_json_query(&json, path)
                    .map(const_json_string_result)
                    .unwrap_or_else(|| fallback.to_string()),
            )
        }
        "json.kind" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(
                const_json_get_key(&json, key)
                    .map(const_json_kind)
                    .unwrap_or("missing")
                    .to_string(),
            )
        }
        "json.query_kind" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(
                const_json_query(&json, path)
                    .map(const_json_kind)
                    .unwrap_or("missing")
                    .to_string(),
            )
        }
        "json.at" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            let index = const_i32_arg(args, 2)?;
            let value = const_json_get_key(&json, key)?;
            let ConstJson::Array(items) = value else {
                return Some(String::new());
            };
            Some(
                usize::try_from(index)
                    .ok()
                    .and_then(|index| items.get(index))
                    .map(const_json_string_result)
                    .unwrap_or_default(),
            )
        }
        "json.query_at" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            let index = const_i32_arg(args, 2)?;
            let value = const_json_query(&json, path)?;
            let ConstJson::Array(items) = value else {
                return Some(String::new());
            };
            Some(
                usize::try_from(index)
                    .ok()
                    .and_then(|index| items.get(index))
                    .map(const_json_string_result)
                    .unwrap_or_default(),
            )
        }
        "json.keys" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            Some(const_json_keys(&json))
        }
        "json.keys_json" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            Some(const_json_keys_json(&json))
        }
        "json.query_keys_json" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(
                const_json_query(&json, path)
                    .map(const_json_keys_json)
                    .unwrap_or_else(|| "[]".to_string()),
            )
        }
        _ => None,
    }
}

fn fold_json_bool_call(name: &str, args: &[LlValue]) -> Option<bool> {
    match name {
        "json.valid" => {
            let value = const_str_arg(args, 0)?;
            (!value.contains("\\u")).then(|| parse_const_json(value).is_some())
        }
        "json.has" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(const_json_get_key(&json, key).is_some())
        }
        "json.query_has" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(const_json_query(&json, path).is_some())
        }
        "json.bool" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(const_json_get_key(&json, key).is_some_and(const_json_bool))
        }
        "json.query_bool" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(const_json_query(&json, path).is_some_and(const_json_bool))
        }
        "json.contains" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            let needle = const_str_arg(args, 2)?;
            Some(
                const_json_get_key(&json, key)
                    .is_some_and(|value| const_json_array_contains(value, needle)),
            )
        }
        "json.query_contains" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            let needle = const_str_arg(args, 2)?;
            Some(
                const_json_query(&json, path)
                    .is_some_and(|value| const_json_array_contains(value, needle)),
            )
        }
        _ => None,
    }
}

fn fold_json_i32_call(name: &str, args: &[LlValue]) -> Option<i32> {
    match name {
        "json.int" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(const_json_get_key(&json, key).map_or(0, const_json_i32))
        }
        "json.query_int" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(const_json_query(&json, path).map_or(0, const_json_i32))
        }
        "json.len" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let key = const_str_arg(args, 1)?;
            Some(const_json_get_key(&json, key).map_or(0, const_json_len))
        }
        "json.query_len" => {
            let json = parse_const_json(const_str_arg(args, 0)?)?;
            let path = const_str_arg(args, 1)?;
            Some(const_json_query(&json, path).map_or(0, const_json_len))
        }
        _ => None,
    }
}

fn url_is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn url_encode_const(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if url_is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

fn url_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn url_decode_const(value: &str, plus_to_space: bool) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                url_hex_value(bytes[index + 1]),
                url_hex_value(bytes[index + 2]),
            ) {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(if plus_to_space && bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8(out).ok()
}

fn url_authority_start(value: &str) -> usize {
    if let Some(index) = value.find("://") {
        index + 3
    } else if value.starts_with("//") {
        2
    } else {
        0
    }
}

fn url_authority_end(value: &str, start: usize) -> usize {
    value[start..]
        .find(|ch| matches!(ch, '/' | '?' | '#'))
        .map(|offset| start + offset)
        .unwrap_or(value.len())
}

fn url_scheme_const(value: &str) -> String {
    value
        .find("://")
        .filter(|index| *index > 0)
        .map(|index| value[..index].to_string())
        .unwrap_or_default()
}

fn url_host_const(value: &str) -> String {
    let mut start = url_authority_start(value);
    let end = url_authority_end(value, start);
    if start == end {
        return String::new();
    }
    if let Some(offset) = value[start..end].rfind('@') {
        start += offset + 1;
    }
    if value.as_bytes().get(start) == Some(&b'[') {
        value[start..end]
            .find(']')
            .map(|offset| value[start..=start + offset].to_string())
            .unwrap_or_else(|| value[start..end].to_string())
    } else {
        let host_end = value[start..end]
            .find(':')
            .map(|offset| start + offset)
            .unwrap_or(end);
        value[start..host_end].to_string()
    }
}

fn url_path_const(value: &str) -> String {
    let start = url_authority_start(value);
    let rest = &value[start..];
    let path_offset = rest.find('/');
    let query_offset = rest.find('?');
    let fragment_offset = rest.find('#');
    let Some(path_offset) = path_offset else {
        return "/".to_string();
    };
    if query_offset.is_some_and(|query| query < path_offset)
        || fragment_offset.is_some_and(|fragment| fragment < path_offset)
    {
        return "/".to_string();
    }
    let path_start = start + path_offset;
    let path_end = value[path_start..]
        .find(|ch| matches!(ch, '?' | '#'))
        .map(|offset| path_start + offset)
        .unwrap_or(value.len());
    value[path_start..path_end].to_string()
}

fn url_query_find_const(value: &str, key: &str) -> Option<Option<String>> {
    let mut cursor = if let Some(index) = value.find('?') {
        index + 1
    } else if value.contains('=') {
        0
    } else {
        return Some(None);
    };
    if value.as_bytes().get(cursor) == Some(&b'?') {
        cursor += 1;
    }
    while cursor < value.len() && value.as_bytes()[cursor] != b'#' {
        let key_start = cursor;
        while cursor < value.len() && !matches!(value.as_bytes()[cursor], b'=' | b'&' | b'#') {
            cursor += 1;
        }
        let key_end = cursor;
        let mut value_start = cursor;
        let mut value_end = cursor;
        if value.as_bytes().get(cursor) == Some(&b'=') {
            cursor += 1;
            value_start = cursor;
            while cursor < value.len() && !matches!(value.as_bytes()[cursor], b'&' | b'#') {
                cursor += 1;
            }
            value_end = cursor;
        }
        let decoded_key = url_decode_const(&value[key_start..key_end], true)?;
        if decoded_key == key {
            return Some(Some(url_decode_const(
                &value[value_start..value_end],
                true,
            )?));
        }
        if value.as_bytes().get(cursor) == Some(&b'&') {
            cursor += 1;
        }
    }
    Some(None)
}

fn fold_url_string_call(name: &str, args: &[LlValue]) -> Option<String> {
    match name {
        "url.encode" => const_str_arg(args, 0).map(url_encode_const),
        "url.decode" => url_decode_const(const_str_arg(args, 0)?, true),
        "url.scheme" => const_str_arg(args, 0).map(url_scheme_const),
        "url.host" => const_str_arg(args, 0).map(url_host_const),
        "url.path" => const_str_arg(args, 0).map(url_path_const),
        "url.query_get" => {
            let value = const_str_arg(args, 0)?;
            let key = const_str_arg(args, 1)?;
            Some(url_query_find_const(value, key)?.unwrap_or_default())
        }
        "url.query_or" => {
            let value = const_str_arg(args, 0)?;
            let key = const_str_arg(args, 1)?;
            let fallback = const_str_arg(args, 2)?;
            Some(url_query_find_const(value, key)?.unwrap_or_else(|| fallback.to_string()))
        }
        _ => None,
    }
}

fn fold_url_bool_call(name: &str, args: &[LlValue]) -> Option<bool> {
    match name {
        "url.query_has" => {
            let value = const_str_arg(args, 0)?;
            let key = const_str_arg(args, 1)?;
            Some(url_query_find_const(value, key)?.is_some())
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstValue {
    I32(i32),
    Bool(bool),
}

enum ConstControl {
    Continue,
    Return(ConstValue),
}

struct ConstEvaluator<'a> {
    program: &'a Program,
    steps: usize,
    budget: usize,
}

impl<'a> ConstEvaluator<'a> {
    fn new(program: &'a Program) -> Self {
        Self {
            program,
            steps: 0,
            budget: 50_000_000,
        }
    }

    fn tick(&mut self) -> Option<()> {
        if self.steps >= self.budget {
            None
        } else {
            self.steps += 1;
            Some(())
        }
    }

    fn eval_function(&mut self, name: &str, args: &[i32]) -> Option<ConstValue> {
        self.tick()?;
        let function = self.program.items.iter().find_map(|item| match item {
            Item::Function(function) if function.name == name => Some(function),
            _ => None,
        })?;
        if function.is_async
            || !function.effects.is_empty()
            || function.ret.name != "i32"
            || function.params.len() != args.len()
            || function.params.iter().any(|param| param.ty.name != "i32")
        {
            return None;
        }
        let mut env = BTreeMap::new();
        for (param, value) in function.params.iter().zip(args.iter().copied()) {
            env.insert(param.name.clone(), ConstValue::I32(value));
        }
        match self.eval_block(&function.body, &mut env)? {
            ConstControl::Return(value) => Some(value),
            ConstControl::Continue => Some(ConstValue::I32(0)),
        }
    }

    fn eval_block(
        &mut self,
        block: &Block,
        env: &mut BTreeMap<String, ConstValue>,
    ) -> Option<ConstControl> {
        for stmt in &block.stmts {
            match self.eval_stmt(stmt, env)? {
                ConstControl::Continue => {}
                returned @ ConstControl::Return(_) => return Some(returned),
            }
        }
        Some(ConstControl::Continue)
    }

    fn eval_stmt(
        &mut self,
        stmt: &Stmt,
        env: &mut BTreeMap<String, ConstValue>,
    ) -> Option<ConstControl> {
        self.tick()?;
        match stmt {
            Stmt::Let { name, expr, .. } => {
                let value = self.eval_expr(expr, env)?;
                env.insert(name.clone(), value);
                Some(ConstControl::Continue)
            }
            Stmt::Assign { target, expr, .. } => {
                let Expr::Ident(name, _) = target else {
                    return None;
                };
                let value = self.eval_expr(expr, env)?;
                if !env.contains_key(name) {
                    return None;
                }
                env.insert(name.clone(), value);
                Some(ConstControl::Continue)
            }
            Stmt::Return { expr, .. } => {
                let value = if let Some(expr) = expr {
                    self.eval_expr(expr, env)?
                } else {
                    ConstValue::I32(0)
                };
                Some(ConstControl::Return(value))
            }
            Stmt::Expr { expr, .. } => {
                self.eval_expr(expr, env)?;
                Some(ConstControl::Continue)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                if self.eval_bool(cond, env)? {
                    self.eval_block(then_block, env)
                } else if let Some(else_block) = else_block {
                    self.eval_block(else_block, env)
                } else {
                    Some(ConstControl::Continue)
                }
            }
            Stmt::While { cond, body, .. } => {
                while self.eval_bool(cond, env)? {
                    self.tick()?;
                    match self.eval_block(body, env)? {
                        ConstControl::Continue => {}
                        returned @ ConstControl::Return(_) => return Some(returned),
                    }
                }
                Some(ConstControl::Continue)
            }
            Stmt::Assert { expr, .. } => {
                self.eval_bool(expr, env)?;
                Some(ConstControl::Continue)
            }
            Stmt::Loop { .. } => None,
        }
    }

    fn eval_expr(
        &mut self,
        expr: &Expr,
        env: &mut BTreeMap<String, ConstValue>,
    ) -> Option<ConstValue> {
        match expr {
            Expr::Int(value, _) => i32::try_from(*value).ok().map(ConstValue::I32),
            Expr::Bool(value, _) => Some(ConstValue::Bool(*value)),
            Expr::Ident(name, _) => env.get(name).copied(),
            Expr::Unary { op, expr, .. } => match (op, self.eval_expr(expr, env)?) {
                (UnaryOp::Neg, ConstValue::I32(value)) => {
                    Some(ConstValue::I32(value.wrapping_neg()))
                }
                (UnaryOp::Not, ConstValue::Bool(value)) => Some(ConstValue::Bool(!value)),
                _ => None,
            },
            Expr::Binary {
                op, left, right, ..
            } => self.eval_binary(*op, left, right, env),
            Expr::Call { callee, args, .. } => {
                let name = expr_name(callee)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.eval_i32(arg, env)?);
                }
                self.eval_function(&name, &values)
            }
            _ => None,
        }
    }

    fn eval_binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        env: &mut BTreeMap<String, ConstValue>,
    ) -> Option<ConstValue> {
        match op {
            BinaryOp::And => {
                let left = self.eval_bool(left, env)?;
                if !left {
                    return Some(ConstValue::Bool(false));
                }
                Some(ConstValue::Bool(self.eval_bool(right, env)?))
            }
            BinaryOp::Or => {
                let left = self.eval_bool(left, env)?;
                if left {
                    return Some(ConstValue::Bool(true));
                }
                Some(ConstValue::Bool(self.eval_bool(right, env)?))
            }
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                let left = self.eval_i32(left, env)?;
                let right = self.eval_i32(right, env)?;
                match op {
                    BinaryOp::Add => Some(ConstValue::I32(left.wrapping_add(right))),
                    BinaryOp::Sub => Some(ConstValue::I32(left.wrapping_sub(right))),
                    BinaryOp::Mul => Some(ConstValue::I32(left.wrapping_mul(right))),
                    BinaryOp::Div => (right != 0).then_some(ConstValue::I32(left / right)),
                    BinaryOp::Mod => (right != 0).then_some(ConstValue::I32(left % right)),
                    BinaryOp::Eq => Some(ConstValue::Bool(left == right)),
                    BinaryOp::Ne => Some(ConstValue::Bool(left != right)),
                    BinaryOp::Lt => Some(ConstValue::Bool(left < right)),
                    BinaryOp::Le => Some(ConstValue::Bool(left <= right)),
                    BinaryOp::Gt => Some(ConstValue::Bool(left > right)),
                    BinaryOp::Ge => Some(ConstValue::Bool(left >= right)),
                    _ => None,
                }
            }
        }
    }

    fn eval_i32(&mut self, expr: &Expr, env: &mut BTreeMap<String, ConstValue>) -> Option<i32> {
        match self.eval_expr(expr, env)? {
            ConstValue::I32(value) => Some(value),
            _ => None,
        }
    }

    fn eval_bool(&mut self, expr: &Expr, env: &mut BTreeMap<String, ConstValue>) -> Option<bool> {
        match self.eval_expr(expr, env)? {
            ConstValue::Bool(value) => Some(value),
            _ => None,
        }
    }
}

fn const_eval_user_call_i32(program: &Program, name: &str, args: &[LlValue]) -> Option<i32> {
    let mut values = Vec::new();
    for arg in args {
        if arg.llvm_ty != "i32" {
            return None;
        }
        values.push(arg.repr.parse::<i32>().ok()?);
    }
    match ConstEvaluator::new(program).eval_function(name, &values)? {
        ConstValue::I32(value) => Some(value),
        _ => None,
    }
}

fn sanitize_ident(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn async_entry_name(function_name: &str) -> String {
    format!("{}_ax_async_entry", sanitize_ident(function_name))
}

fn future_ax_ty(ret: &TypeRef) -> String {
    format!("Future<{}>", ret.display())
}

fn future_inner_ax_ty(value: &str) -> Option<String> {
    value
        .strip_prefix("Future<")?
        .strip_suffix('>')
        .map(str::to_string)
}

fn llvm_type_name(ax_ty: &str) -> String {
    match ax_ty {
        "void" => "void".to_string(),
        "bool" => "i1".to_string(),
        "i8" | "u8" => "i8".to_string(),
        "i16" | "u16" => "i16".to_string(),
        "i32" | "u32" => "i32".to_string(),
        "i64" | "u64" => "i64".to_string(),
        "f32" => "float".to_string(),
        "f64" => "double".to_string(),
        "str" | "ptr" => "ptr".to_string(),
        _ => "ptr".to_string(),
    }
}

fn pack_alias(pack_name: &str) -> &str {
    pack_name.rsplit('.').next().unwrap_or(pack_name)
}

fn linked_native_sources(semantic: &SemanticInfo) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for pack in &semantic.packs {
        for source in &pack.native_sources {
            let path = PathBuf::from(source);
            if seen.insert(path.clone()) {
                sources.push(path);
            }
        }
    }
    sources
}

fn add_runtime_sources(
    command: &mut Command,
    runtime_dir: &Path,
    program: &Program,
    semantic: &SemanticInfo,
) {
    command
        .arg(runtime_dir.join("core.c"))
        .arg(runtime_dir.join("crypto.c"))
        .arg(runtime_dir.join("io.c"));
    let needs_async = program_needs_async(program);
    let needs_tcp = program_needs_tcp(program) || semantic_calls_tcp(semantic);
    let needs_http = program_needs_http(program);
    let needs_http_client = semantic_calls_http_client(semantic);
    let needs_map = semantic_calls_map(semantic);
    if needs_async {
        command.arg(runtime_dir.join("async.c"));
    }
    if needs_tcp {
        command.arg(runtime_dir.join("net_tcp.c"));
    }
    if needs_http || needs_http_client {
        command.arg(runtime_dir.join("net_http.c"));
    }
    if needs_map {
        command.arg(runtime_dir.join("map.c"));
    }
    if !cfg!(target_os = "windows")
        && (needs_async || needs_tcp || needs_http || needs_http_client || needs_map)
    {
        command.arg("-pthread");
    }
}

fn semantic_uses_effect(semantic: &SemanticInfo, function: &str, effect: &str) -> bool {
    semantic.functions.get(function).is_some_and(|info| {
        info.effects.iter().any(|item| item == effect)
            || info.inferred_effects.iter().any(|item| item == effect)
    })
}

fn semantic_calls_http_client(semantic: &SemanticInfo) -> bool {
    semantic.functions.values().any(|function| {
        function.calls.iter().any(|call| {
            matches!(
                call.as_str(),
                "http.get" | "http.post" | "http.get_json" | "http.post_json"
            )
        })
    })
}

fn semantic_calls_tcp(semantic: &SemanticInfo) -> bool {
    semantic.functions.values().any(|function| {
        function.calls.iter().any(|call| {
            call == "tcp.listen"
                || call == "tcp.connect"
                || call == "tcp.serve_text"
                || call.ends_with(".accept")
                || call.ends_with(".read_text")
                || call.ends_with(".write_text")
                || call.ends_with(".request_text")
                || call.ends_with(".close")
        })
    })
}

fn semantic_calls_map(semantic: &SemanticInfo) -> bool {
    semantic.functions.values().any(|function| {
        function.calls.iter().any(|call| {
            call == "map.new"
                || call.ends_with(".set")
                || call.ends_with(".get")
                || call.ends_with(".has")
                || call.ends_with(".del")
                || call.ends_with(".len")
                || call.ends_with(".clear")
        })
    })
}

fn add_link_dead_strip_flags(command: &mut Command) {
    if cfg!(target_os = "macos") {
        command.arg("-Wl,-dead_strip");
    } else if !cfg!(target_os = "windows") {
        command.arg("-Wl,--gc-sections");
    }
}

fn program_needs_async(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(item, Item::Function(function) if function.is_async))
}

fn program_needs_http(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(item, Item::Server(_)))
}

fn program_needs_tcp(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(item, Item::Tcp(_)))
}

fn program_needs_http_tls(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(item, Item::Server(server) if server.tls))
}

fn program_needs_tcp_tls(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(item, Item::Tcp(tcp) if tcp.tls))
}

fn program_needs_tls(program: &Program) -> bool {
    program_needs_http_tls(program) || program_needs_tcp_tls(program)
}

fn llvm_escape_string(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'\\' => out.push_str("\\5C"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(*byte as char),
            other => out.push_str(&format!("\\{:02X}", other)),
        }
    }
    out
}

fn json_body(fields: &[(String, Expr)]) -> String {
    let mut out = String::from("{");
    for (idx, (key, value)) in fields.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(key);
        out.push_str("\":");
        out.push_str(&json_value(value));
    }
    out.push('}');
    out
}

fn json_value(expr: &Expr) -> String {
    match expr {
        Expr::Bool(value, _) => value.to_string(),
        Expr::Int(value, _) => value.to_string(),
        Expr::Float(value, _) => value.to_string(),
        Expr::Str(value, _) => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        Expr::Await { expr, .. } => json_value(expr),
        _ => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ax_core::SourceFile;
    use ax_parser::parse_source;
    use ax_semantic::check_program;

    #[test]
    fn emits_llvm_for_hello() {
        let source = SourceFile::new("hello.ax", "{;\"hello\"}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare void @ax_io_println"));
        assert!(llvm.contains("define i32 @main()"));
        assert!(!llvm.contains("call void @ax_cli_init"));
    }

    #[test]
    fn build_program_keeps_absolute_ax_out_output_nonempty() {
        if Command::new("clang").arg("--version").output().is_err() {
            return;
        }

        let source = SourceFile::new("build_output.ax", "{}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let cwd = std::env::current_dir().expect("cwd");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let base = format!("ax-build-output-{}-{}", std::process::id(), stamp);
        let output = cwd.join(".ax-out").join(&base);

        let artifact = build_program(Path::new("build_output.ax"), &program, &semantic, &output)
            .expect("build");
        assert_eq!(artifact.output_path, output);
        let len = std::fs::metadata(&artifact.output_path)
            .expect("output metadata")
            .len();
        assert!(len > 0, "absolute output should not be truncated");

        let _ = std::fs::remove_file(&artifact.output_path);
        let _ = std::fs::remove_file(cwd.join(".ax-out").join(format!("{}.ll", base)));
        let _ = std::fs::remove_file(cwd.join(".ax-out").join(format!("{}.o", base)));
    }

    #[test]
    fn emits_tcp_route_table() {
        let source = SourceFile::new("tcp.ax", "&&3000{\"ping\">\"pong\\n\"*>\"unknown\\n\"}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("%ax_tcp_route"));
        assert!(llvm.contains("@ax_tcp_server_start"));
        assert!(llvm.contains("pong\\0A"));
    }

    #[test]
    fn emits_tcp_connect_call() {
        let source = SourceFile::new(
            "tcp_connect.ax",
            "{$\"PING\\n\"$Qb(\"127.0.0.1\",6379)b.write_text(a)$b.read_text(1024)$b.request_text(a,1024)b.close()^c!+d!}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare ptr @ax_tcp_connect"));
        assert!(llvm.contains("call ptr @ax_tcp_connect"));
        assert!(llvm.contains("call void @ax_tcp_write_text"));
        assert!(llvm.contains("call ptr @ax_tcp_read_text"));
        assert!(llvm.contains("call ptr @ax_tcp_request_text"));
    }

    #[test]
    fn emits_tcp_listen_host_port_and_parse_i32() {
        let source = SourceFile::new("tcp_listen_host.ax", "{$Sr(\"6379\")$Qa(\"127.0.0.1\",a)}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare i32 @ax_str_parse_i32(ptr)"));
        assert!(llvm.contains("call i32 @ax_str_parse_i32"));
        assert!(llvm.contains("declare ptr @ax_tcp_listen_on(ptr, i32)"));
        assert!(llvm.contains("call ptr @ax_tcp_listen_on"));
    }

    #[test]
    fn emits_tls_server_entrypoints() {
        let source = SourceFile::new("tls.ax", "&!3443{G/ping>\"pong\"}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_http_tls_server_start"));
        assert!(llvm.contains("call i32 @ax_http_tls_server_start"));
    }

    #[test]
    fn emits_core_runtime_operations() {
        let source = SourceFile::new(
            "core.ax",
            "{#(\"/tmp/ax.txt\",\"http://127.0.0.1:3010/health\",\"/tmp/ax-state.json\",\"/tmp\",\"http://127.0.0.1:3010/echo\",\"/tmp/ax.env\",\"/tmp/ax-renamed.txt\",\"/tmp/ax-atomic.txt\",\"fallback\",\"/tmp/ax-copy.txt\",\"/tmp/ax-b64.bin\",\"/tmp/ax-trash\",\"items\",\"AX_TEST\",\"k\",\"/tmp/ax-dir\",\"printf ax\",\"limit\",\"x\",\"--input\",\"default\",\"status\",\"verify\",\"ok\",\"steps\",\"tools\",\"mode\")$Id;+p Fb(\"/tmp/ax-dir/nested/artifacts\")Fc(\"/tmp/ax-dir/generated/state.json\");a,D Fh(h,D);;c,\"{ \\\"ok\\\" : true }\"Fj(a,\"!\")$Fm(a)$Fn(\"/tmp/ax-missing.txt\",i)$Fo(a,8)$Fp(a,1,4)$Fq(a,4)$Fr(a,0,1)Fs(a,0,1)8(c)Fl(\"/tmp/ax-missing.json\",\"{}\")$M0(a)C0(k,V)5(a)a@Fw(d)Fx(a)Fy(a)X0(a)Fah Fai Faf(a,j)Fag(j,g)Fao(d)Fz(d)T0(d)Fap(d)U0(d)S0(d)P0(d,\"ax\")N0(d,\"*.txt\");+l;\"/tmp/ax-trash/file.txt\",G Fe(l)Fe(p)$U(G)Cb(K)Cc(G,W)$Cj(o,W)Ck(o,W)Cl(o,W,Y)$Cm(o,a)Cn(o,a)Co(o,a,Z)$Cp(o,a,1,4)Cq(o,a,1,4)Cr(o,a,1,4,_)$4(a)Ce(a)Cf(a,aa)$Cg(a,1,4)Ch(a,1,4)Ci(a,1,4,ab)$Cs(G)$Ct(ac)Cu(ad,G)$=ad:G Cv(16)Cx;f,\"AX_DOTENV_NAME=agent\\nAX_DOTENV_MODE=release\\n\"Ec(n,x)Ef(f)Eg(f)Ea(n)Ed(\"AX_MISSING\",i)Ee(\"AX_\")Eb(n)Xa(q)Xb(\"printf ax-process\",4)Xd(\"printf ax-json\",7)Xe(\"printf ax-log\",6)Xc(q)Aa Ab(0)Ac(t)Ad(t)Ae(\"--mode\",u)Af Ha(b)Hb(e,G)Hc(b,256)Hd(e,G,256)$7(\"{ \\\"ok\\\" : true, \\\"limit\\\" : 2, \\\"meta\\\" : { \\\"owner\\\" : \\\"agent\\\", \\\"enabled\\\" : true }, \\\"steps\\\" : [{ \\\"name\\\" : \\\"read\\\" }], \\\"items\\\" : [1, 2], \\\"tools\\\" : [\\\"read\\\", \\\"verify\\\"] }\")3(af)Jf(G)$Jd(\"owner\",G)$H(ag)$Je(Jf(G))Ji(af,r,\"3\")Jj(af,v,\"ready\")Jk(af,v)M(x,\"true\")Jm(af,x)Jn(af,\"missing\",i)af`\"meta.owner\"Jp(af,\"meta.missing\",i)A0(af,\"steps.0.name\")af#r af'\"meta.enabled\"E0(af,m)Jaf(af,r)Jae(af,x)S(af,z,w)Jw(af,z,w)F0(af,m)T(af,m)af\\y af[m,0]P(af,y,0)Jy(af)Jz(af)H0(af,\"meta\")Jah(G)G!G~s Sc(G,s)Sd(G,s)Se(G,s)Sf(G,s)Sg(G)Sh(G)Si(G)Sj(G,G)Sk(G,2)Sl(G,s,\"y\")Sm(G,0,1)Sn(G,s)So(G)Ss(G,0)str.token_upper(G,0)$Pb(d,\"ax.txt\")9(aj)Pc(aj)Pd(aj)Pe(aj)Pf(aj)Pg(aj)$\"https://agent.local/tools?q=Ax%20language&mode=fast\"Ui(ak)Uh(ak)Ug(ak)Uc(ak,\"q\")Ud(ak,B,u)Ue(ak,B)Uf(ak)$Ua(G)Ub(al)Fd(a)Fd(h)Fd(c)Fd(k)Fd(g)Fd(f)Ib(\"out:\");G Ic(\"diagnostic\")Ta Tb Tc(Ta)Td(0)$Ma(16)Mb(am)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_io_read_line"));
        assert!(llvm.contains("@ax_io_print"));
        assert!(llvm.contains("@ax_io_eprintln"));
        assert!(llvm.contains("@ax_fs_write_text"));
        assert!(llvm.contains("@ax_fs_write_text_atomic"));
        assert!(llvm.contains("@ax_fs_write_json_atomic"));
        assert!(llvm.contains("@ax_fs_write_base64"));
        assert!(llvm.contains("@ax_fs_append_text"));
        assert!(llvm.contains("@ax_fs_append_jsonl"));
        assert!(llvm.contains("@ax_fs_read_text"));
        assert!(llvm.contains("@ax_fs_read_text_or"));
        assert!(llvm.contains("@ax_fs_read_text_limit"));
        assert!(llvm.contains("@ax_fs_read_text_range"));
        assert!(llvm.contains("@ax_fs_read_text_tail"));
        assert!(llvm.contains("@ax_fs_read_lines"));
        assert!(llvm.contains("@ax_fs_read_lines_json"));
        assert!(llvm.contains("@ax_fs_read_jsonl"));
        assert!(llvm.contains("@ax_fs_read_json"));
        assert!(llvm.contains("@ax_fs_read_json_or"));
        assert!(llvm.contains("@ax_fs_read_base64"));
        assert!(llvm.contains("@ax_fs_exists"));
        assert!(llvm.contains("@ax_fs_remove"));
        assert!(llvm.contains("@ax_fs_mkdir"));
        assert!(llvm.contains("@ax_fs_mkdir_all"));
        assert!(llvm.contains("@ax_fs_ensure_parent"));
        assert!(llvm.contains("@ax_fs_list"));
        assert!(llvm.contains("@ax_fs_list_json"));
        assert!(llvm.contains("@ax_fs_list_stat_json"));
        assert!(llvm.contains("@ax_fs_walk"));
        assert!(llvm.contains("@ax_fs_walk_json"));
        assert!(llvm.contains("@ax_fs_walk_stat_json"));
        assert!(llvm.contains("@ax_fs_find"));
        assert!(llvm.contains("@ax_fs_glob"));
        assert!(llvm.contains("@ax_fs_copy"));
        assert!(llvm.contains("@ax_fs_size"));
        assert!(llvm.contains("@ax_fs_stat_json"));
        assert!(llvm.contains("@ax_fs_cwd"));
        assert!(llvm.contains("@ax_fs_temp_dir"));
        assert!(llvm.contains("@ax_fs_rename"));
        assert!(llvm.contains("@ax_fs_remove_dir"));
        assert!(llvm.contains("@ax_fs_is_file"));
        assert!(llvm.contains("@ax_fs_is_dir"));
        assert!(llvm.contains("@ax_fs_modified"));
        assert!(llvm.contains("@ax_crypto_sha256_hex"));
        assert!(llvm.contains("@ax_crypto_sha256_json"));
        assert!(llvm.contains("@ax_crypto_sha256_verify_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_json"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_verify_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_json"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_verify_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_range_hex"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_range_json"));
        assert!(llvm.contains("@ax_crypto_hmac_sha256_file_range_verify_hex"));
        assert!(llvm.contains("@ax_crypto_sha256_file_hex"));
        assert!(llvm.contains("@ax_crypto_sha256_file_json"));
        assert!(llvm.contains("@ax_crypto_sha256_file_verify_hex"));
        assert!(llvm.contains("@ax_crypto_sha256_file_range_hex"));
        assert!(llvm.contains("@ax_crypto_sha256_file_range_json"));
        assert!(llvm.contains("@ax_crypto_sha256_file_range_verify_hex"));
        assert!(llvm.contains("@ax_crypto_base64_encode"));
        assert!(llvm.contains("@ax_crypto_base64_decode"));
        assert!(llvm.contains("@ax_crypto_constant_time_eq"));
        assert!(llvm.contains("@ax_crypto_random_hex"));
        assert!(llvm.contains("@ax_crypto_random_base64url"));
        assert!(llvm.contains("@ax_crypto_uuid_v4"));
        assert!(llvm.contains("@ax_env_get"));
        assert!(llvm.contains("@ax_env_has"));
        assert!(llvm.contains("@ax_env_set"));
        assert!(llvm.contains("@ax_env_get_or"));
        assert!(llvm.contains("@ax_env_snapshot_json"));
        assert!(llvm.contains("@ax_env_load_dotenv"));
        assert!(llvm.contains("@ax_env_load_dotenv_json"));
        assert!(llvm.contains("@ax_process_exec"));
        assert!(llvm.contains("@ax_process_exec_limit"));
        assert!(llvm.contains("@ax_process_status"));
        assert!(llvm.contains("@ax_process_run_json"));
        assert!(llvm.contains("@ax_process_run_log_json"));
        assert!(llvm.contains("@ax_process_run_lines_json"));
        assert!(llvm.contains("@ax_process_run_log_lines_json"));
        assert!(llvm.contains("@ax_cli_init"));
        assert!(llvm.contains("@ax_cli_argc"));
        assert!(llvm.contains("@ax_cli_arg"));
        assert!(llvm.contains("@ax_cli_has"));
        assert!(llvm.contains("@ax_cli_value"));
        assert!(llvm.contains("@ax_cli_value_or"));
        assert!(llvm.contains("@ax_cli_args_json"));
        assert!(llvm.contains("@ax_cli_parse_json"));
        assert!(llvm.contains("@ax_http_get"));
        assert!(llvm.contains("@ax_http_post"));
        assert!(llvm.contains("@ax_http_get_json"));
        assert!(llvm.contains("@ax_http_post_json"));
        assert!(llvm.contains("@ax_json_escape"));
        assert!(llvm.contains("@ax_json_quote"));
        assert!(llvm.contains("@ax_json_pair"));
        assert!(llvm.contains("@ax_json_string_pair"));
        assert!(llvm.contains("@ax_json_set"));
        assert!(llvm.contains("@ax_json_string_set"));
        assert!(llvm.contains("@ax_json_remove"));
        assert!(llvm.contains("@ax_json_object"));
        assert!(llvm.contains("@ax_json_array"));
        assert!(llvm.contains("@ax_json_array_push"));
        assert!(llvm.contains("@ax_json_string_array_push"));
        assert!(llvm.contains("@ax_json_compact"));
        assert!(llvm.contains("@ax_json_valid"));
        assert!(llvm.contains("@ax_json_get"));
        assert!(llvm.contains("@ax_json_query"));
        assert!(llvm.contains("@ax_json_get_or"));
        assert!(llvm.contains("@ax_json_query_or"));
        assert!(llvm.contains("@ax_json_has"));
        assert!(llvm.contains("@ax_json_query_has"));
        assert!(llvm.contains("@ax_json_int"));
        assert!(llvm.contains("@ax_json_bool"));
        assert!(llvm.contains("@ax_json_query_int"));
        assert!(llvm.contains("@ax_json_query_bool"));
        assert!(llvm.contains("@ax_json_contains"));
        assert!(llvm.contains("@ax_json_query_contains"));
        assert!(llvm.contains("@ax_json_kind"));
        assert!(llvm.contains("@ax_json_query_kind"));
        assert!(llvm.contains("@ax_json_keys"));
        assert!(llvm.contains("@ax_json_keys_json"));
        assert!(llvm.contains("@ax_json_query_keys_json"));
        assert!(llvm.contains("@ax_json_len"));
        assert!(llvm.contains("@ax_json_query_len"));
        assert!(llvm.contains("@ax_json_at"));
        assert!(llvm.contains("@ax_json_query_at"));
        assert!(llvm.contains("@ax_str_len"));
        assert!(llvm.contains("@ax_str_eq"));
        assert!(llvm.contains("@ax_str_contains"));
        assert!(llvm.contains("@ax_str_index_of"));
        assert!(llvm.contains("@ax_str_count"));
        assert!(llvm.contains("@ax_str_starts_with"));
        assert!(llvm.contains("@ax_str_ends_with"));
        assert!(llvm.contains("@ax_str_trim"));
        assert!(llvm.contains("@ax_str_upper"));
        assert!(llvm.contains("@ax_str_lower"));
        assert!(llvm.contains("@ax_str_concat"));
        assert!(llvm.contains("@ax_str_repeat"));
        assert!(llvm.contains("@ax_str_replace"));
        assert!(llvm.contains("@ax_str_slice"));
        assert!(llvm.contains("@ax_str_split_json"));
        assert!(llvm.contains("@ax_str_lines_json"));
        assert!(llvm.contains("@ax_str_token"));
        assert!(llvm.contains("@ax_str_token_upper"));
        assert!(llvm.contains("@ax_str_line"));
        assert!(llvm.contains("@ax_path_normalize"));
        assert!(llvm.contains("@ax_path_join"));
        assert!(llvm.contains("@ax_path_basename"));
        assert!(llvm.contains("@ax_path_dirname"));
        assert!(llvm.contains("@ax_path_extname"));
        assert!(llvm.contains("@ax_path_stem"));
        assert!(llvm.contains("@ax_path_is_absolute"));
        assert!(llvm.contains("@ax_url_encode"));
        assert!(llvm.contains("@ax_url_decode"));
        assert!(llvm.contains("@ax_url_query_get"));
        assert!(llvm.contains("@ax_url_query_or"));
        assert!(llvm.contains("@ax_url_query_has"));
        assert!(llvm.contains("@ax_url_query_json"));
        assert!(llvm.contains("@ax_url_path"));
        assert!(llvm.contains("@ax_url_host"));
        assert!(llvm.contains("@ax_url_scheme"));
        assert!(llvm.contains("@ax_time_now"));
        assert!(llvm.contains("@ax_time_now_ms"));
        assert!(llvm.contains("@ax_time_iso_utc"));
        assert!(llvm.contains("@ax_time_sleep_ms"));
        assert!(llvm.contains("@ax_heap_alloc"));
        assert!(llvm.contains("@ax_heap_free"));
    }

    #[test]
    fn emits_fs_base64_file_operations() {
        let source = SourceFile::new(
            "fs_base64.ax",
            "{$\"/tmp/ax.bin\"$M0(a)$J0(a,1,8)$I0(a,8)C0(\"/tmp/ax.copy\",b)C0(\"/tmp/ax.chunk\",c)C0(\"/tmp/ax.tail\",d)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_fs_read_base64"));
        assert!(llvm.contains("@ax_fs_read_base64_range"));
        assert!(llvm.contains("@ax_fs_read_base64_tail"));
        assert!(llvm.contains("@ax_fs_write_base64"));
    }

    #[test]
    fn emits_fs_append_jsonl() {
        let source = SourceFile::new(
            "fs_append_jsonl.ax",
            "{$\"/tmp/events.jsonl\"Fi(a,\"{ \\\"ok\\\" : true }\")$Ft(a,0,1)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_fs_append_jsonl"));
        assert!(llvm.contains("@ax_fs_read_jsonl"));
    }

    #[test]
    fn emits_crypto_random_base64url() {
        let source = SourceFile::new("crypto_token.ax", "{$Cw(16)}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_crypto_random_base64url"));
    }

    #[test]
    fn emits_process_line_json_operations() {
        let source = SourceFile::new(
            "process_lines.ax",
            "{$Xf(\"echo ax\",16)$Xg(\"echo ax 1>&2\",16)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_process_run_lines_json"));
        assert!(llvm.contains("@ax_process_run_log_lines_json"));
    }

    #[test]
    fn emits_cli_parse_json() {
        let source = SourceFile::new("cli_parse.ax", "{$Ag^Aa}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_cli_parse_json"));
        assert!(llvm.contains("define i32 @main(i32 %argc, ptr %argv)"));
        assert!(llvm.contains("call void @ax_cli_init(i32 %argc, ptr %argv)"));
    }

    #[test]
    fn emits_primitive_width_coercions() {
        let source = SourceFile::new(
            "primitive_widths.ax",
            "%Packet{tag:u8,count:i64,ratio:f32} @widen(value:i64):i64{^value+1} @tag_value(value:u8):u8{^value} {$count:i64=41$byte:u8=42$ratio:f32=1.5$packet=Packet{tag:42,count:9,ratio:0.5}?widen(41):42&tag_value(42):byte&packet.count:9{^0}^1}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("sext i32 41 to i64"));
        assert!(llvm.contains("trunc i32 42 to i8"));
        assert!(llvm.contains("fptrunc double 1.5 to float"));
        assert!(llvm.contains("fptrunc double 0.5 to float"));
        assert!(llvm.contains("call i64 @widen(i64"));
        assert!(llvm.contains("call i8 @tag_value(i8"));
    }

    #[test]
    fn emits_json_array_push_operations() {
        let source = SourceFile::new(
            "json_array_push.ax",
            "{$\"items\"$Je(Jf(\"read\"))$Jg(b,\"true\")$Jh(c,\"done\")^{a:d}\\a}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("@ax_json_array_push"));
        assert!(llvm.contains("@ax_json_string_array_push"));
    }

    #[test]
    fn folds_constant_std_string_path_and_json_calls() {
        let source = SourceFile::new(
            "const_std.ax",
            "{$\"tools\"$\"task\"$\"{ \\\"task\\\" : \\\"summarize\\\", \\\"ok\\\" : true, \\\"agent\\\" : { \\\"limits\\\" : { \\\"tokens\\\" : 2048 } }, \\\"tools\\\" : [\\\"read\\\", \\\"verify\\\"] }\"$7(c)$\"https://agent.local/tools/search?q=Ax%20language&mode=fast\"$Ua(Ug(e))?3(d)&E0(d,b)&d'\"ok\"&S(d,a,\"verify\")&Jy(d)~a&Pg(\"/tmp/ax\")&Ue(e,\"mode\"){^Jm(d,b)!+d#\"agent.limits.tokens\"+T(d,a)+Uh(e)!+Ub(f)!+Uc(e,\"q\")!}^1}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(!llvm.contains("call ptr @ax_json_compact"));
        assert!(!llvm.contains("call i32 @ax_json_valid"));
        assert!(!llvm.contains("call i32 @ax_json_has"));
        assert!(!llvm.contains("call i32 @ax_json_query_bool"));
        assert!(!llvm.contains("call i32 @ax_json_contains"));
        assert!(!llvm.contains("call ptr @ax_json_keys"));
        assert!(!llvm.contains("call ptr @ax_json_get"));
        assert!(!llvm.contains("call i32 @ax_json_query_int"));
        assert!(!llvm.contains("call i32 @ax_json_len"));
        assert!(!llvm.contains("call i32 @ax_str_contains"));
        assert!(!llvm.contains("call i32 @ax_str_len"));
        assert!(!llvm.contains("call i32 @ax_path_is_absolute"));
        assert!(!llvm.contains("call ptr @ax_url_"));
        assert!(!llvm.contains("call i32 @ax_url_"));
    }

    #[test]
    fn folds_pure_i32_function_calls() {
        let source = SourceFile::new(
            "const_user.ax",
            "@a(c:#):#{?c<=1{^c}|{^a(c-1)+a(c-2)}} @b(c:#):#{$d=0$e=0 ~d<c{e=(e+d*31)%1000003 d=d+1}^e} {^a(10)+b(5)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("define i32 @main()"));
        assert!(llvm.contains("i32 55"));
        assert!(llvm.contains("310"));
        assert!(!llvm.contains("call i32 @fib(i32 10)"));
        assert!(!llvm.contains("call i32 @mix(i32 5)"));
    }

    #[test]
    fn emits_mutable_while_locals() {
        let source = SourceFile::new("loop.ax", "{$0$0 ~a<3{b=b+a a=a+1}^b}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("alloca i32"));
        assert!(llvm.contains("load i32"));
        assert!(llvm.contains("store i32"));
        assert!(llvm.contains("br label %while.cond"));
    }

    #[test]
    fn emits_async_spawn_and_await() {
        let source = SourceFile::new(
            "async.ax",
            "@@add(x:#,y:#):#{^x+y} {$value=@(add(20,22))^value}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("define ptr @add_ax_async_entry"));
        assert!(llvm.contains("@ax_async_context_new"));
        assert!(llvm.contains("@ax_async_context_get_i32"));
        assert!(llvm.contains("call ptr @ax_async_spawn_with_context(ptr @add_ax_async_entry"));
        assert!(llvm.contains("@ax_async_await_i32"));
    }

    #[test]
    fn emits_async_cancel() {
        let source = SourceFile::new("async_cancel.ax", "@@a(b:#,c:#):#{^b+c} {$b=a(20,22)Na(b)}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare void @ax_async_cancel(ptr)"));
        assert!(llvm.contains("call void @ax_async_cancel(ptr "));
    }

    #[test]
    fn emits_async_detach() {
        let source = SourceFile::new("async_detach.ax", "@@a():void{} {$b=a()Nb(b)}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare void @ax_async_detach(ptr)"));
        assert!(llvm.contains("call void @ax_async_detach(ptr "));
    }

    #[test]
    fn custom_backend_emits_arm64_assembly() {
        let source = SourceFile::new(
            "custom.ax",
            "@add(x:#,y:#):#{^x+y} {;\"custom\"^add(20,22)}",
        );
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let asm = CustomAsmModule::new(&program, &semantic)
            .generate()
            .expect("asm");
        assert!(asm.contains(".globl _main"));
        assert!(asm.contains("bl _ax_io_println"));
        assert!(asm.contains("bl _add"));
        assert!(asm.contains(".asciz \"custom\""));
    }

    #[test]
    fn emits_enum_variant_tags() {
        let source = SourceFile::new("enum.ax", "%%a{a,b} {$b=a.a?b:a.a{^0}|{^1}}");
        let program = parse_source(&source).expect("parse");
        let semantic = check_program(&program).expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("icmp eq i32"));
    }

    #[test]
    fn rejects_external_pack_call_without_native_sources() {
        let source = SourceFile::new("external.ax", "+acme.telemetry {telemetry.track()}");
        let program = parse_source(&source).expect("parse");
        let semantic = ax_semantic::check_program_with_packs(
            &program,
            vec![ax_semantic::PackSpec {
                name: "acme.telemetry".to_string(),
                syntax: Vec::new(),
                operations: vec!["telemetry.track".to_string()],
                effects: vec!["telemetry.write".to_string()],
                native_sources: Vec::new(),
            }],
        )
        .expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let err = LlvmModule::new(&ir)
            .generate()
            .expect_err("native source error");
        assert_eq!(err.code, "AX_CODEGEN_ERROR");
        assert!(err.message.contains("requires pack native sources"));
    }

    #[test]
    fn emits_external_pack_native_call() {
        let source = SourceFile::new("external.ax", "+acme.telemetry {telemetry.track()}");
        let program = parse_source(&source).expect("parse");
        let semantic = ax_semantic::check_program_with_packs(
            &program,
            vec![ax_semantic::PackSpec {
                name: "acme.telemetry".to_string(),
                syntax: Vec::new(),
                operations: vec!["telemetry.track".to_string()],
                effects: vec!["telemetry.write".to_string()],
                native_sources: vec!["native.c".to_string()],
            }],
        )
        .expect("semantic");
        let ir = IrProgram::lower(&program, &semantic);
        let llvm = LlvmModule::new(&ir).generate().expect("llvm");
        assert!(llvm.contains("declare void @ax_pack_acme_telemetry_track()"));
        assert!(llvm.contains("call void @ax_pack_acme_telemetry_track()"));
    }
}
