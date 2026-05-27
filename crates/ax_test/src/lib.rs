use ax_ast::*;
use ax_core::SourceFile;
use ax_diag::{AxResult, Diagnostic};
use ax_parser::parse_source;
use ax_semantic::{check_program_with_options, CheckOptions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct TestReport {
    pub passed: usize,
    pub failed: usize,
    pub files: usize,
}

pub fn run_tests(root: &Path) -> Result<TestReport, String> {
    let mut files = Vec::new();
    collect_ax_files(&root.join("tests"), &mut files).map_err(|err| err.to_string())?;
    let mut report = TestReport::default();
    for path in files {
        let source = SourceFile::from_path(&path).map_err(|err| err.to_string())?;
        let program = parse_source(&source).map_err(|diag| diag.render(&source))?;
        check_program_with_options(
            &program,
            CheckOptions {
                require_main: false,
                packs: Vec::new(),
            },
        )
        .map_err(|diag| diag.render(&source))?;
        let file_report = run_program_tests(&program)?;
        report.passed += file_report.passed;
        report.failed += file_report.failed;
        report.files += 1;
    }
    Ok(report)
}

fn collect_ax_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_ax_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "ax") {
            out.push(path);
        }
    }
    Ok(())
}

fn run_program_tests(program: &Program) -> Result<TestReport, String> {
    let mut functions = BTreeMap::new();
    let mut tests = Vec::new();
    for item in &program.items {
        match item {
            Item::Function(function) => {
                functions.insert(function.name.clone(), function);
            }
            Item::Test(test) => tests.push(test),
            _ => {}
        }
    }
    let mut report = TestReport::default();
    for test in tests {
        match run_test(test, &functions) {
            Ok(()) => report.passed += 1,
            Err(err) => {
                report.failed += 1;
                eprintln!("test {:?} failed: {}", test.name, err.message);
            }
        }
    }
    Ok(report)
}

fn run_test(test: &TestBlock, functions: &BTreeMap<String, &Function>) -> AxResult<()> {
    let mut env = BTreeMap::new();
    eval_block(&test.body, &mut env, functions)?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
    Void,
}

enum EvalFlow {
    Continue,
    Return(Value),
}

fn eval_block(
    block: &Block,
    env: &mut BTreeMap<String, Value>,
    functions: &BTreeMap<String, &Function>,
) -> AxResult<EvalFlow> {
    for stmt in &block.stmts {
        match eval_stmt(stmt, env, functions)? {
            EvalFlow::Continue => {}
            flow @ EvalFlow::Return(_) => return Ok(flow),
        }
    }
    Ok(EvalFlow::Continue)
}

fn eval_stmt(
    stmt: &Stmt,
    env: &mut BTreeMap<String, Value>,
    functions: &BTreeMap<String, &Function>,
) -> AxResult<EvalFlow> {
    match stmt {
        Stmt::Let { name, expr, .. } => {
            let value = eval_expr(expr, env, functions)?;
            env.insert(name.clone(), value);
            Ok(EvalFlow::Continue)
        }
        Stmt::Assign { target, expr, span } => {
            let Expr::Ident(name, _) = target else {
                return Err(Diagnostic::error(
                    "AX_TEST_UNSUPPORTED",
                    "test runner supports assignment to local identifiers only",
                    *span,
                ));
            };
            if !env.contains_key(name) {
                return Err(Diagnostic::error(
                    "AX_UNKNOWN_SYMBOL",
                    format!("unknown test symbol `{}`", name),
                    *span,
                ));
            }
            let value = eval_expr(expr, env, functions)?;
            env.insert(name.clone(), value);
            Ok(EvalFlow::Continue)
        }
        Stmt::Return { expr, .. } => {
            let value = if let Some(expr) = expr {
                eval_expr(expr, env, functions)?
            } else {
                Value::Void
            };
            Ok(EvalFlow::Return(value))
        }
        Stmt::Expr { expr, .. } => {
            eval_expr(expr, env, functions)?;
            Ok(EvalFlow::Continue)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
            span,
        } => match eval_expr(cond, env, functions)? {
            Value::Bool(true) => eval_block(then_block, env, functions),
            Value::Bool(false) => {
                if let Some(else_block) = else_block {
                    eval_block(else_block, env, functions)
                } else {
                    Ok(EvalFlow::Continue)
                }
            }
            _ => Err(Diagnostic::error(
                "AX_TEST_UNSUPPORTED",
                "test runner conditions must evaluate to bool",
                *span,
            )),
        },
        Stmt::Loop { body, span } => eval_loop(body, env, functions, *span, None),
        Stmt::While { cond, body, span } => eval_loop(body, env, functions, *span, Some(cond)),
        Stmt::Assert { expr, span } => {
            let value = eval_expr(expr, env, functions)?;
            if value != Value::Bool(true) {
                return Err(Diagnostic::error(
                    "AX_TEST_FAILED",
                    "assertion failed",
                    *span,
                ));
            }
            Ok(EvalFlow::Continue)
        }
    }
}

fn eval_loop(
    body: &Block,
    env: &mut BTreeMap<String, Value>,
    functions: &BTreeMap<String, &Function>,
    span: ax_core::Span,
    cond: Option<&Expr>,
) -> AxResult<EvalFlow> {
    for _ in 0..100_000 {
        if let Some(cond) = cond {
            match eval_expr(cond, env, functions)? {
                Value::Bool(true) => {}
                Value::Bool(false) => return Ok(EvalFlow::Continue),
                _ => {
                    return Err(Diagnostic::error(
                        "AX_TEST_UNSUPPORTED",
                        "test runner loop conditions must evaluate to bool",
                        span,
                    ))
                }
            }
        }
        match eval_block(body, env, functions)? {
            EvalFlow::Continue => {}
            flow @ EvalFlow::Return(_) => return Ok(flow),
        }
    }
    Err(Diagnostic::error(
        "AX_TEST_LIMIT",
        "test loop exceeded 100000 iterations",
        span,
    ))
}

fn eval_expr(
    expr: &Expr,
    env: &mut BTreeMap<String, Value>,
    functions: &BTreeMap<String, &Function>,
) -> AxResult<Value> {
    match expr {
        Expr::Int(value, _) => Ok(Value::Int(*value)),
        Expr::Bool(value, _) => Ok(Value::Bool(*value)),
        Expr::Str(value, _) => Ok(Value::Str(value.clone())),
        Expr::Ident(name, span) => env.get(name).cloned().ok_or_else(|| {
            Diagnostic::error(
                "AX_UNKNOWN_SYMBOL",
                format!("unknown test symbol `{}`", name),
                *span,
            )
        }),
        Expr::Binary {
            op,
            left,
            right,
            span,
            ..
        } => {
            let left = eval_expr(left, env, functions)?;
            let right = eval_expr(right, env, functions)?;
            eval_binary(*op, left, right, *span)
        }
        Expr::Unary { op, expr, span } => {
            let value = eval_expr(expr, env, functions)?;
            eval_unary(*op, value, *span)
        }
        Expr::Await { expr, .. } => eval_expr(expr, env, functions),
        Expr::Call { callee, args, span } => {
            let Expr::Ident(name, _) = callee.as_ref() else {
                return Err(Diagnostic::error(
                    "AX_TEST_UNSUPPORTED",
                    "test runner supports direct pure function calls only",
                    *span,
                ));
            };
            let function = functions.get(name).ok_or_else(|| {
                Diagnostic::error(
                    "AX_UNKNOWN_SYMBOL",
                    format!("unknown function `{}`", name),
                    *span,
                )
            })?;
            if function.params.len() != args.len() {
                return Err(Diagnostic::error(
                    "AX_ARITY_MISMATCH",
                    format!(
                        "function {} expects {} argument(s), got {}",
                        name,
                        function.params.len(),
                        args.len()
                    ),
                    *span,
                ));
            }
            let mut call_env = BTreeMap::new();
            for (param, arg) in function.params.iter().zip(args) {
                call_env.insert(param.name.clone(), eval_expr(arg, env, functions)?);
            }
            eval_function(function, &mut call_env, functions)
        }
        _ => Err(Diagnostic::error(
            "AX_TEST_UNSUPPORTED",
            "expression is not supported by compiler-side v1 test runner",
            expr.span(),
        )),
    }
}

fn eval_function(
    function: &Function,
    env: &mut BTreeMap<String, Value>,
    functions: &BTreeMap<String, &Function>,
) -> AxResult<Value> {
    match eval_block(&function.body, env, functions)? {
        EvalFlow::Continue => Ok(Value::Void),
        EvalFlow::Return(value) => Ok(value),
    }
}

fn eval_binary(op: BinaryOp, left: Value, right: Value, span: ax_core::Span) -> AxResult<Value> {
    match (op, left, right) {
        (BinaryOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (BinaryOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
        (BinaryOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (BinaryOp::Div, Value::Int(_), Value::Int(0)) => Err(Diagnostic::error(
            "AX_TEST_FAILED",
            "division by zero in test expression",
            span,
        )),
        (BinaryOp::Div, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
        (BinaryOp::Mod, Value::Int(_), Value::Int(0)) => Err(Diagnostic::error(
            "AX_TEST_FAILED",
            "modulo by zero in test expression",
            span,
        )),
        (BinaryOp::Mod, Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
        (BinaryOp::Eq, a, b) => Ok(Value::Bool(a == b)),
        (BinaryOp::Ne, a, b) => Ok(Value::Bool(a != b)),
        (BinaryOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (BinaryOp::Le, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (BinaryOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (BinaryOp::Ge, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
        (BinaryOp::And, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a && b)),
        (BinaryOp::Or, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a || b)),
        _ => Err(Diagnostic::error(
            "AX_TEST_UNSUPPORTED",
            "unsupported test expression operator or operand types",
            span,
        )),
    }
}

fn eval_unary(op: UnaryOp, value: Value, span: ax_core::Span) -> AxResult<Value> {
    match (op, value) {
        (UnaryOp::Not, Value::Bool(value)) => Ok(Value::Bool(!value)),
        (UnaryOp::Neg, Value::Int(value)) => Ok(Value::Int(-value)),
        _ => Err(Diagnostic::error(
            "AX_TEST_UNSUPPORTED",
            "unsupported unary test expression",
            span,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn run_tests_rejects_semantically_invalid_files() {
        let root = temp_root("ax-test-semantic");
        let tests_dir = root.join("tests");
        std::fs::create_dir_all(&tests_dir).expect("create tests dir");
        std::fs::write(
            tests_dir.join("bad.ax"),
            "@bad():#{missing=1^0} ?\"bad\"{:!1}\n",
        )
        .expect("write test file");

        let err = run_tests(&root).expect_err("semantic error");
        assert!(err.contains("AX_UNKNOWN_SYMBOL"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn run_tests_accepts_pure_control_flow() {
        let root = temp_root("ax-test-control");
        let tests_dir = root.join("tests");
        std::fs::create_dir_all(&tests_dir).expect("create tests dir");
        std::fs::write(
            tests_dir.join("control.ax"),
            "@fib(n:#):#{$i=0$a=0$b=1 ~i<n{$next=a+b a=b b=next i=i+1}^a} ?\"fib\"{:fib(7):13}\n",
        )
        .expect("write test file");

        let report = run_tests(&root).expect("test report");
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.files, 1);

        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), nanos))
    }
}
