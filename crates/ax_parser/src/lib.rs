use ax_ast::*;
use ax_core::{SourceFile, Span};
use ax_diag::{AxResult, Diagnostic};
use ax_lexer::{lex, Token, TokenKind};
use std::collections::BTreeSet;

pub fn parse_source(source: &SourceFile) -> AxResult<Program> {
    Parser::new(source).parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    allow_record_init: bool,
    implicit_name_scopes: Vec<MinifiedNameGenerator>,
}

impl Parser {
    fn new(source: &SourceFile) -> Self {
        Self {
            tokens: lex(source),
            index: 0,
            allow_record_init: true,
            implicit_name_scopes: Vec::new(),
        }
    }

    fn parse_program(&mut self) -> AxResult<Program> {
        let mut program = Program::default();
        while !self.is_eof() {
            if self.is_symbol("+") {
                program.uses.push(self.parse_use()?);
            } else {
                program.items.push(self.parse_item()?);
            }
        }
        Ok(program)
    }

    fn parse_use(&mut self) -> AxResult<UseDecl> {
        let start = self.expect_symbol("+")?;
        let (path, end) = self.parse_dotted_path()?;
        Ok(UseDecl {
            path,
            span: start.merge(end),
        })
    }

    fn parse_item(&mut self) -> AxResult<Item> {
        if self.is_symbol("@") {
            self.parse_symbol_function().map(Item::Function)
        } else if self.is_symbol("{") {
            self.parse_symbol_main_block().map(Item::Function)
        } else if self.is_symbol("&") {
            self.parse_symbol_server().map(Item::Server)
        } else if self.is_symbol("&&") {
            self.parse_symbol_tcp().map(Item::Tcp)
        } else if self.is_symbol("%") {
            self.parse_symbol_type_item()
        } else if self.is_symbol("?") {
            self.parse_symbol_test().map(Item::Test)
        } else {
            Err(self.error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected item, found {}", self.describe_current()),
            ))
        }
    }

    fn parse_symbol_main_block(&mut self) -> AxResult<Function> {
        let start = self.current().span;
        let body = self.parse_function_block(&[])?;
        Ok(Function {
            name: "main".to_string(),
            is_async: false,
            params: Vec::new(),
            ret: TypeRef::simple("i32"),
            effects: Vec::new(),
            body,
            span: start,
        })
    }

    fn parse_symbol_function(&mut self) -> AxResult<Function> {
        let start = self.expect_symbol("@")?;
        let is_async = self.eat_symbol("@");
        if !is_async && self.is_symbol("{") {
            let body = self.parse_function_block(&[])?;
            return Ok(Function {
                name: "main".to_string(),
                is_async: false,
                params: Vec::new(),
                ret: TypeRef::simple("i32"),
                effects: Vec::new(),
                body,
                span: start,
            });
        }
        let (name, name_span) = self.expect_ident_like()?;
        self.expect_symbol("(")?;
        let mut params = Vec::new();
        if !self.eat_symbol(")") {
            loop {
                let (param_name, param_span) = self.expect_ident_like()?;
                self.expect_symbol(":")?;
                let ty = self.parse_type_ref()?;
                params.push(Param {
                    name: param_name,
                    ty,
                    span: param_span,
                });
                if self.eat_symbol(")") {
                    break;
                }
                self.expect_symbol(",")?;
            }
        }
        self.expect_symbol(":")?;
        let ret = self.parse_type_ref()?;
        let effects = if self.eat_symbol("!") {
            self.parse_effect_list()?
        } else {
            Vec::new()
        };
        let param_names = params
            .iter()
            .map(|param| param.name.clone())
            .collect::<Vec<_>>();
        let body = self.parse_function_block(&param_names)?;
        Ok(Function {
            name,
            is_async,
            params,
            ret,
            effects,
            body,
            span: start.merge(name_span),
        })
    }

    fn parse_symbol_type_item(&mut self) -> AxResult<Item> {
        let start = self.expect_symbol("%")?;
        if self.eat_symbol("%") {
            let (name, name_span) = self.expect_ident_like()?;
            let variants = self.parse_variant_block()?;
            Ok(Item::Enum(EnumDecl {
                name,
                variants,
                span: start.merge(name_span),
            }))
        } else if self.eat_symbol("!") {
            let (name, name_span) = self.expect_ident_like()?;
            let variants = self.parse_variant_block()?;
            Ok(Item::Error(ErrorDecl {
                name,
                variants,
                span: start.merge(name_span),
            }))
        } else {
            let (name, name_span) = self.expect_ident_like()?;
            self.expect_symbol("{")?;
            let mut fields = Vec::new();
            while !self.eat_symbol("}") {
                if self.is_eof() {
                    return Err(self.error("AX_PARSE_UNEXPECTED_TOKEN", "unterminated type block"));
                }
                let (field_name, field_span) = self.expect_ident_like()?;
                self.expect_symbol(":")?;
                let ty = self.parse_type_ref()?;
                fields.push(Field {
                    name: field_name,
                    ty,
                    span: field_span,
                });
                self.eat_symbol(",");
            }
            Ok(Item::Type(TypeDecl {
                name,
                fields,
                span: start.merge(name_span),
            }))
        }
    }

    fn parse_variant_block(&mut self) -> AxResult<Vec<String>> {
        self.expect_symbol("{")?;
        let mut variants = Vec::new();
        while !self.eat_symbol("}") {
            if self.is_eof() {
                return Err(self.error("AX_PARSE_UNEXPECTED_TOKEN", "unterminated variant block"));
            }
            let (variant, _) = self.expect_ident_like()?;
            variants.push(variant);
            self.eat_symbol(",");
        }
        Ok(variants)
    }

    fn parse_symbol_server(&mut self) -> AxResult<ServerBlock> {
        let start = self.expect_symbol("&")?;
        let tls = self.eat_symbol("!");
        self.parse_server_tail(start, tls)
    }

    fn parse_server_tail(&mut self, start: Span, tls: bool) -> AxResult<ServerBlock> {
        let (port, port_span) = self.expect_int()?;
        let port = parse_port(port, port_span)?;
        self.expect_symbol("{")?;
        let mut routes = Vec::new();
        while !self.eat_symbol("}") {
            if self.is_eof() {
                return Err(self.error("AX_PARSE_UNEXPECTED_TOKEN", "unterminated server block"));
            }
            routes.push(self.parse_http_route()?);
        }
        Ok(ServerBlock {
            port,
            tls,
            routes,
            span: start,
        })
    }

    fn parse_http_route(&mut self) -> AxResult<HttpRoute> {
        let (method_text, method_span) = self.expect_ident_like()?;
        let method = match method_text.as_str() {
            "G" => HttpMethod::Get,
            "P" => HttpMethod::Post,
            "U" => HttpMethod::Put,
            "A" => HttpMethod::Patch,
            "D" => HttpMethod::Delete,
            _ => {
                return Err(Diagnostic::error(
                    "AX_INVALID_ROUTE",
                    format!("unknown HTTP method `{}`", method_text),
                    method_span,
                ))
            }
        };
        let (path, path_span) = self.parse_path()?;
        self.expect_symbol(">")?;
        let response = self.parse_http_response()?;
        Ok(HttpRoute {
            method,
            path,
            response,
            span: method_span.merge(path_span),
        })
    }

    fn parse_http_response(&mut self) -> AxResult<HttpResponse> {
        if matches!(self.current().kind, TokenKind::Str(_)) {
            let (value, _) = self.expect_string()?;
            Ok(HttpResponse::Text(value))
        } else if self.eat_symbol("#") {
            self.parse_http_json_response()
        } else if self.eat_symbol("~") {
            Ok(HttpResponse::StreamBody)
        } else {
            Err(self.error(
                "AX_INVALID_ROUTE",
                "expected min route response string, `#`, or `~`",
            ))
        }
    }

    fn parse_http_json_response(&mut self) -> AxResult<HttpResponse> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        while !self.eat_symbol("}") {
            let (key, _) = self.expect_ident_like()?;
            self.expect_symbol(":")?;
            let value = self.parse_expr()?;
            fields.push((key, value));
            self.eat_symbol(",");
        }
        Ok(HttpResponse::Json(fields))
    }

    fn parse_symbol_tcp(&mut self) -> AxResult<TcpBlock> {
        let start = self.expect_symbol("&&")?;
        let tls = self.eat_symbol("!");
        self.parse_tcp_tail(start, tls)
    }

    fn parse_tcp_tail(&mut self, start: Span, tls: bool) -> AxResult<TcpBlock> {
        let (port, port_span) = self.expect_int()?;
        let port = parse_port(port, port_span)?;
        self.expect_symbol("{")?;
        let mut routes = Vec::new();
        while !self.eat_symbol("}") {
            if self.is_eof() {
                return Err(self.error("AX_PARSE_UNEXPECTED_TOKEN", "unterminated tcp block"));
            }
            let pattern_span = self.current().span;
            let pattern = if self.eat_symbol("*") {
                TcpPattern::Wildcard
            } else {
                TcpPattern::Exact(self.expect_string()?.0)
            };
            self.expect_symbol(">")?;
            let (response, response_span) = self.expect_string()?;
            routes.push(TcpRoute {
                pattern,
                response,
                span: pattern_span.merge(response_span),
            });
        }
        Ok(TcpBlock {
            port,
            tls,
            routes,
            span: start,
        })
    }

    fn parse_symbol_test(&mut self) -> AxResult<TestBlock> {
        let start = self.expect_symbol("?")?;
        let (name, name_span) = self.expect_string()?;
        let body = self.parse_block()?;
        Ok(TestBlock {
            name,
            body,
            span: start.merge(name_span),
        })
    }

    fn parse_block(&mut self) -> AxResult<Block> {
        let start = self.expect_symbol("{")?;
        let mut stmts = Vec::new();
        while !self.eat_symbol("}") {
            if self.is_eof() {
                return Err(self.error("AX_PARSE_UNEXPECTED_TOKEN", "unterminated block"));
            }
            if self.is_symbol("#") {
                stmts.extend(self.parse_string_pool()?);
            } else {
                stmts.push(self.parse_stmt()?);
            }
        }
        Ok(Block { stmts, span: start })
    }

    fn parse_function_block(&mut self, param_names: &[String]) -> AxResult<Block> {
        self.push_implicit_name_scope(param_names);
        let body = self.parse_block();
        self.implicit_name_scopes.pop();
        body
    }

    fn parse_isolated_block(&mut self) -> AxResult<Block> {
        let saved_scopes = self.implicit_name_scopes.clone();
        let body = self.parse_block();
        self.implicit_name_scopes = saved_scopes;
        body
    }

    fn parse_string_pool(&mut self) -> AxResult<Vec<Stmt>> {
        let start = self.expect_symbol("#")?;
        self.expect_symbol("(")?;
        let mut stmts = Vec::new();
        if self.eat_symbol(")") {
            return Ok(stmts);
        }
        let mut implicit_names = MinifiedNameGenerator::new(implicit_string_pool_reserved());
        loop {
            let (name, name_span) = if self.is_string() {
                let name = self
                    .next_implicit_name()
                    .unwrap_or_else(|| implicit_names.next());
                (name, self.current().span)
            } else {
                self.expect_ident_like()?
            };
            let (value, value_span) = self.expect_string()?;
            self.reserve_implicit_name(&name);
            stmts.push(Stmt::Let {
                name,
                ty: None,
                expr: Expr::Str(value, value_span),
                span: start.merge(name_span),
            });
            if self.eat_symbol(")") {
                break;
            }
            self.expect_symbol(",")?;
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> AxResult<Stmt> {
        if self.is_symbol("$") {
            let start = self.expect_symbol("$")?;
            let (name, name_span, ty) = if self.eat_symbol("=") || !self.is_named_min_let_start() {
                let name = self.next_implicit_name().ok_or_else(|| {
                    self.error(
                        "AX_PARSE_UNEXPECTED_TOKEN",
                        "implicit let name outside function",
                    )
                })?;
                (name, start, None)
            } else {
                let (name, name_span) = self.expect_ident_like()?;
                self.reserve_implicit_name(&name);
                let ty = if self.eat_symbol(":") {
                    Some(self.parse_type_ref()?)
                } else {
                    None
                };
                self.expect_symbol("=")?;
                (name, name_span, ty)
            };
            let expr = self.parse_expr()?;
            Ok(Stmt::Let {
                name,
                ty,
                expr,
                span: start.merge(name_span),
            })
        } else if self.is_symbol("^") {
            let span = self.expect_symbol("^")?;
            let expr = if self.is_symbol("}") {
                None
            } else {
                Some(self.parse_expr()?)
            };
            Ok(Stmt::Return { expr, span })
        } else if self.is_symbol("?") {
            let span = self.expect_symbol("?")?;
            let cond = self.parse_condition_expr()?;
            let then_block = self.parse_isolated_block()?;
            let else_block = if self.eat_symbol("|") {
                Some(self.parse_isolated_block()?)
            } else {
                None
            };
            Ok(Stmt::If {
                cond,
                then_block,
                else_block,
                span,
            })
        } else if self.is_symbol("~") {
            let span = self.expect_symbol("~")?;
            if self.is_symbol("{") {
                let body = self.parse_block()?;
                Ok(Stmt::Loop { body, span })
            } else {
                let cond = self.parse_condition_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::While { cond, body, span })
            }
        } else if self.is_symbol(":") {
            let span = self.expect_symbol(":")?;
            let expr = self.parse_expr()?;
            Ok(Stmt::Assert { expr, span })
        } else if self.eat_symbol(";") {
            let start = self.previous_span();
            if self.eat_symbol(";") {
                let path = self.parse_expr()?;
                self.expect_symbol(",")?;
                let value = self.parse_expr()?;
                let span = start.merge(value.span());
                return Ok(Stmt::Expr {
                    expr: Expr::Call {
                        callee: Box::new(expr_from_dotted_path("fs.write_json_atomic", start)),
                        args: vec![path, value],
                        span,
                    },
                    span,
                });
            }
            if self.eat_symbol("+") {
                let path = self.parse_expr()?;
                let span = start.merge(path.span());
                return Ok(Stmt::Expr {
                    expr: Expr::Call {
                        callee: Box::new(expr_from_dotted_path("fs.mkdir", start)),
                        args: vec![path],
                        span,
                    },
                    span,
                });
            }
            let arg = self.parse_expr()?;
            let span = start.merge(arg.span());
            if self.eat_symbol(",") {
                let value = self.parse_expr()?;
                let span = start.merge(value.span());
                return Ok(Stmt::Expr {
                    expr: Expr::Call {
                        callee: Box::new(expr_from_dotted_path("fs.write_text", start)),
                        args: vec![arg, value],
                        span,
                    },
                    span,
                });
            }
            Ok(Stmt::Expr {
                expr: Expr::Call {
                    callee: Box::new(expr_from_dotted_path("io.println", start)),
                    args: vec![arg],
                    span,
                },
                span,
            })
        } else {
            let start_span = self.current().span;
            let target_or_expr = self.parse_expr()?;
            if self.eat_symbol("=") {
                let expr = self.parse_expr()?;
                Ok(Stmt::Assign {
                    target: target_or_expr,
                    expr,
                    span: start_span,
                })
            } else {
                let span = target_or_expr.span();
                Ok(Stmt::Expr {
                    expr: target_or_expr,
                    span,
                })
            }
        }
    }

    fn parse_expr(&mut self) -> AxResult<Expr> {
        self.parse_binary(0)
    }

    fn parse_condition_expr(&mut self) -> AxResult<Expr> {
        let allow_record_init = self.allow_record_init;
        self.allow_record_init = false;
        let expr = self.parse_expr();
        self.allow_record_init = allow_record_init;
        expr
    }

    fn parse_binary(&mut self, min_prec: u8) -> AxResult<Expr> {
        let mut left = self.parse_unary()?;
        while let Some((op, prec)) = self.peek_binary_op() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let right = self.parse_binary(prec + 1)?;
            let span = left.span().merge(right.span());
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> AxResult<Expr> {
        if self.eat_symbol("!") {
            let op_span = self.previous_span();
            if let TokenKind::Int(value) = &self.current().kind {
                if value == "0" || value == "1" {
                    let value = value == "1";
                    let value_span = self.bump().span;
                    return Ok(Expr::Bool(value, op_span.merge(value_span)));
                }
            }
            let expr = self.parse_unary()?;
            let span = op_span.merge(expr.span());
            Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            })
        } else if self.eat_symbol("-") {
            let op_span = self.previous_span();
            let expr = self.parse_unary()?;
            let span = op_span.merge(expr.span());
            Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            })
        } else if self.eat_symbol("@") {
            let await_span = self.previous_span();
            let expr = self.parse_unary()?;
            let span = await_span.merge(expr.span());
            Ok(Expr::Await {
                expr: Box::new(expr),
                span,
            })
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> AxResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.eat_symbol(".") {
                let (field, field_span) = self.expect_ident_like()?;
                let span = expr.span().merge(field_span);
                expr = Expr::Member {
                    object: Box::new(expr),
                    field,
                    span,
                };
            } else if self.eat_symbol("(") {
                let mut args = Vec::new();
                if !self.eat_symbol(")") {
                    loop {
                        args.push(self.parse_expr()?);
                        if self.eat_symbol(")") {
                            break;
                        }
                        self.expect_symbol(",")?;
                    }
                }
                let span = expr.span();
                let callee = normalize_call_callee(expr);
                expr = Expr::Call {
                    callee: Box::new(callee),
                    args,
                    span,
                };
            } else if self.eat_symbol("[") {
                let key = self.parse_expr()?;
                self.expect_symbol(",")?;
                let index = self.parse_expr()?;
                let end = self.expect_symbol("]")?;
                let span = expr.span().merge(end);
                expr = Expr::Call {
                    callee: Box::new(expr_from_dotted_path("json.at", span)),
                    args: vec![expr, key, index],
                    span,
                };
            } else if self.is_json_query_postfix() {
                let op = self.bump().clone();
                let Some(callee) = json_query_postfix_callee(&op) else {
                    unreachable!("checked json query postfix");
                };
                let key = self.parse_unary()?;
                let span = expr.span().merge(key.span());
                expr = Expr::Call {
                    callee: Box::new(expr_from_dotted_path(callee, op.span)),
                    args: vec![expr, key],
                    span,
                };
            } else if self.is_adjacent_symbol_to_previous("~") {
                self.bump();
                let needle = self.parse_unary()?;
                let span = expr.span().merge(needle.span());
                expr = Expr::Call {
                    callee: Box::new(expr_from_dotted_path("str.contains", span)),
                    args: vec![expr, needle],
                    span,
                };
            } else if self.is_adjacent_symbol_to_previous("!") {
                let op_span = self.bump().span;
                let span = expr.span().merge(op_span);
                expr = Expr::Call {
                    callee: Box::new(expr_from_dotted_path("str.len", span)),
                    args: vec![expr],
                    span,
                };
            } else if self.is_adjacent_symbol_to_previous("@") {
                let op_span = self.bump().span;
                let span = expr.span().merge(op_span);
                expr = Expr::Call {
                    callee: Box::new(expr_from_dotted_path("fs.is_file", span)),
                    args: vec![expr],
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> AxResult<Expr> {
        let token = self.bump().clone();
        match token.kind {
            TokenKind::Int(value) => Ok(Expr::Int(value.parse().unwrap_or(0), token.span)),
            TokenKind::Float(value) => Ok(Expr::Float(value.parse().unwrap_or(0.0), token.span)),
            TokenKind::Str(value) => Ok(Expr::Str(value, token.span)),
            TokenKind::Ident(value) => {
                if self.allow_record_init && self.eat_symbol("{") {
                    let mut fields = Vec::new();
                    while !self.eat_symbol("}") {
                        let (field, _) = self.expect_ident_like()?;
                        self.expect_symbol(":")?;
                        let expr = self.parse_expr()?;
                        fields.push((field, expr));
                        self.eat_symbol(",");
                    }
                    Ok(Expr::RecordInit {
                        name: value,
                        fields,
                        span: token.span,
                    })
                } else if !self.is_symbol("(") {
                    if let Some(expanded) = expanded_std_call_alias(&value) {
                        Ok(Expr::Call {
                            callee: Box::new(expr_from_dotted_path(&expanded, token.span)),
                            args: Vec::new(),
                            span: token.span,
                        })
                    } else {
                        Ok(Expr::Ident(value, token.span))
                    }
                } else {
                    Ok(Expr::Ident(value, token.span))
                }
            }
            TokenKind::Symbol(value) if value == "(" => {
                let expr = self.parse_expr()?;
                self.expect_symbol(")")?;
                Ok(expr)
            }
            TokenKind::Symbol(value) if value == "{" => self.parse_json_object_expr(token.span),
            _ => Err(Diagnostic::error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected expression, found {:?}", token.kind),
                token.span,
            )),
        }
    }

    fn parse_json_object_expr(&mut self, start: Span) -> AxResult<Expr> {
        let key = self.parse_unary()?;
        let pair_callee = if self.eat_symbol(":") {
            "json.pair"
        } else {
            self.expect_symbol("=")?;
            "json.string_pair"
        };
        let value = self.parse_expr()?;
        let end = self.expect_symbol("}")?;
        let pair_span = start.merge(value.span());
        let pair = Expr::Call {
            callee: Box::new(expr_from_dotted_path(pair_callee, start)),
            args: vec![key, value],
            span: pair_span,
        };
        Ok(Expr::Call {
            callee: Box::new(expr_from_dotted_path("json.object", start)),
            args: vec![pair],
            span: start.merge(end),
        })
    }

    fn parse_type_ref(&mut self) -> AxResult<TypeRef> {
        if self.eat_symbol("[") {
            let inner = self.parse_type_ref()?;
            self.expect_symbol("]")?;
            let mut ty = TypeRef::simple("array");
            ty.array = true;
            ty.args.push(inner);
            ty.optional = self.eat_symbol("?");
            return Ok(ty);
        }
        let coded = if self.eat_symbol("#") {
            Some("i32")
        } else if self.eat_symbol("$") {
            Some("str")
        } else {
            None
        };
        if let Some(name) = coded {
            let mut ty = TypeRef::simple(name);
            ty.optional = self.eat_symbol("?");
            return Ok(ty);
        }
        let (name, _) = self.expect_ident_like()?;
        let mut ty = TypeRef::simple(name);
        if self.eat_symbol("<") {
            loop {
                ty.args.push(self.parse_type_ref()?);
                if self.eat_symbol(">") {
                    break;
                }
                self.expect_symbol(",")?;
            }
        }
        ty.optional = self.eat_symbol("?");
        Ok(ty)
    }

    fn parse_effect_list(&mut self) -> AxResult<Vec<String>> {
        let mut effects = Vec::new();
        loop {
            let (path, _) = self.parse_dotted_path()?;
            effects.push(path);
            if !self.eat_symbol(",") {
                break;
            }
        }
        Ok(effects)
    }

    fn parse_dotted_path(&mut self) -> AxResult<(String, Span)> {
        let (mut path, start) = self.expect_ident_like()?;
        let mut span = start;
        while self.eat_symbol(".") {
            let (part, part_span) = self.expect_ident_like()?;
            path.push('.');
            path.push_str(&part);
            span = span.merge(part_span);
        }
        Ok((path, span))
    }

    fn parse_path(&mut self) -> AxResult<(String, Span)> {
        let start = self.expect_symbol("/")?;
        let mut path = String::from("/");
        let mut span = start;
        loop {
            match &self.current().kind {
                TokenKind::Ident(value)
                | TokenKind::Keyword(value)
                | TokenKind::Int(value)
                | TokenKind::Float(value) => {
                    path.push_str(value);
                    span = span.merge(self.current().span);
                    self.bump();
                }
                TokenKind::Symbol(value) if value == "*" => {
                    path.push('*');
                    span = span.merge(self.current().span);
                    self.bump();
                    break;
                }
                _ => break,
            }
            if self.eat_symbol("/") {
                path.push('/');
                span = span.merge(self.previous_span());
            } else {
                break;
            }
        }
        Ok((path, span))
    }

    fn peek_binary_op(&self) -> Option<(BinaryOp, u8)> {
        let symbol = match &self.current().kind {
            TokenKind::Symbol(symbol) => symbol.as_str(),
            _ => return None,
        };
        let (op, prec) = match symbol {
            "|" => (BinaryOp::Or, 1),
            "&" => (BinaryOp::And, 2),
            ":" => (BinaryOp::Eq, 3),
            "!:" => (BinaryOp::Ne, 3),
            "<" => (BinaryOp::Lt, 4),
            "<=" => (BinaryOp::Le, 4),
            ">" => (BinaryOp::Gt, 4),
            ">=" => (BinaryOp::Ge, 4),
            "+" => (BinaryOp::Add, 5),
            "-" => (BinaryOp::Sub, 5),
            "*" => (BinaryOp::Mul, 6),
            "/" => (BinaryOp::Div, 6),
            "%" => (BinaryOp::Mod, 6),
            _ => return None,
        };
        Some((op, prec))
    }

    fn expect_symbol(&mut self, symbol: &str) -> AxResult<Span> {
        if self.eat_symbol(symbol) {
            Ok(self.previous_span())
        } else {
            Err(self.error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected `{}`", symbol),
            ))
        }
    }

    fn eat_symbol(&mut self, symbol: &str) -> bool {
        if self.is_symbol(symbol) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn is_symbol(&self, symbol: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Symbol(value) if value == symbol)
    }

    fn is_adjacent_symbol_to_previous(&self, symbol: &str) -> bool {
        self.is_symbol(symbol)
            && self
                .tokens
                .get(self.index.saturating_sub(1))
                .is_some_and(|previous| previous.span.end == self.current().span.start)
    }

    fn is_string(&self) -> bool {
        matches!(self.current().kind, TokenKind::Str(_))
    }

    fn is_json_query_postfix(&self) -> bool {
        json_query_postfix_callee(self.current()).is_some()
    }

    fn is_named_min_let_start(&self) -> bool {
        if !matches!(self.current().kind, TokenKind::Ident(_)) {
            return false;
        }
        self.next_is_symbol("=") || self.next_is_symbol(":")
    }

    fn next_is_symbol(&self, symbol: &str) -> bool {
        matches!(
            self.tokens.get(self.index + 1).map(|token| &token.kind),
            Some(TokenKind::Symbol(value)) if value == symbol
        )
    }

    fn push_implicit_name_scope(&mut self, param_names: &[String]) {
        let mut names = MinifiedNameGenerator::new(implicit_string_pool_reserved());
        for param_name in param_names {
            names.reserve(param_name);
        }
        self.implicit_name_scopes.push(names);
    }

    fn next_implicit_name(&mut self) -> Option<String> {
        self.implicit_name_scopes
            .last_mut()
            .map(MinifiedNameGenerator::next)
    }

    fn reserve_implicit_name(&mut self, name: &str) {
        if let Some(names) = self.implicit_name_scopes.last_mut() {
            names.reserve(name);
        }
    }

    fn expect_ident_like(&mut self) -> AxResult<(String, Span)> {
        let token = self.bump().clone();
        match token.kind {
            TokenKind::Ident(value) => Ok((value, token.span)),
            _ => Err(Diagnostic::error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected identifier, found {:?}", token.kind),
                token.span,
            )),
        }
    }

    fn expect_int(&mut self) -> AxResult<(String, Span)> {
        let token = self.bump().clone();
        match token.kind {
            TokenKind::Int(value) => Ok((value, token.span)),
            _ => Err(Diagnostic::error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected integer, found {:?}", token.kind),
                token.span,
            )),
        }
    }

    fn expect_string(&mut self) -> AxResult<(String, Span)> {
        let token = self.bump().clone();
        match token.kind {
            TokenKind::Str(value) => Ok((value, token.span)),
            _ => Err(Diagnostic::error(
                "AX_PARSE_UNEXPECTED_TOKEN",
                format!("expected string, found {:?}", token.kind),
                token.span,
            )),
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn bump(&mut self) -> &Token {
        let idx = self.index;
        self.index += 1;
        &self.tokens[idx]
    }

    fn previous_span(&self) -> Span {
        self.tokens[self.index.saturating_sub(1)].span
    }

    fn is_eof(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    fn describe_current(&self) -> String {
        match &self.current().kind {
            TokenKind::Ident(value)
            | TokenKind::Int(value)
            | TokenKind::Float(value)
            | TokenKind::Str(value)
            | TokenKind::Keyword(value)
            | TokenKind::Symbol(value) => format!("`{}`", value),
            TokenKind::Eof => "end of file".to_string(),
        }
    }

    fn error(&self, code: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(code, message, self.current().span)
    }
}

fn parse_port(value: String, span: Span) -> AxResult<u16> {
    value.parse::<u16>().map_err(|_| {
        Diagnostic::error(
            "AX_INVALID_ROUTE",
            format!("port `{}` is outside the valid u16 range", value),
            span,
        )
    })
}

fn json_query_postfix_callee(token: &Token) -> Option<&'static str> {
    let TokenKind::Symbol(symbol) = &token.kind else {
        return None;
    };
    match symbol.as_str() {
        "`" => Some("json.query"),
        "'" => Some("json.query_bool"),
        "#" => Some("json.query_int"),
        "\\" => Some("json.query_len"),
        _ => None,
    }
}

fn normalize_call_callee(callee: Expr) -> Expr {
    if let Expr::Int(value, span) = &callee {
        if let Some(expanded) = expanded_std_call_alias(&value.to_string()) {
            return expr_from_dotted_path(&expanded, *span);
        }
    }
    let Some(path) = expr_path(&callee) else {
        return callee;
    };
    let Some(expanded) = expanded_std_call_alias(&path) else {
        return callee;
    };
    expr_from_dotted_path(&expanded, callee.span())
}

fn expr_path(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name, _) => Some(name.clone()),
        Expr::Member { object, field, .. } => Some(format!("{}.{}", expr_path(object)?, field)),
        _ => None,
    }
}

fn expr_from_dotted_path(path: &str, span: Span) -> Expr {
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or(path);
    let mut expr = Expr::Ident(first.to_string(), span);
    for part in parts {
        expr = Expr::Member {
            object: Box::new(expr),
            field: part.to_string(),
            span,
        };
    }
    expr
}

#[derive(Clone)]
struct MinifiedNameGenerator {
    next: usize,
    reserved: BTreeSet<String>,
}

impl MinifiedNameGenerator {
    fn new(reserved: BTreeSet<String>) -> Self {
        Self { next: 0, reserved }
    }

    fn reserve(&mut self, name: &str) {
        self.reserved.insert(name.to_string());
    }

    fn next(&mut self) -> String {
        loop {
            let name = encode_minified_name(self.next);
            self.next += 1;
            if !self.reserved.contains(&name) {
                self.reserved.insert(name.clone());
                return name;
            }
        }
    }
}

fn encode_minified_name(mut value: usize) -> String {
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

fn implicit_string_pool_reserved() -> BTreeSet<String> {
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
        "main",
    ];
    words
        .into_iter()
        .map(str::to_string)
        .chain(minified_std_call_aliases())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_server() {
        let source = SourceFile::new("test.ax", "&3000{G/ping>\"pong\"}");
        let program = parse_source(&source).expect("parse");
        assert!(matches!(program.items[0], Item::Server(_)));
    }

    #[test]
    fn parses_tls_servers() {
        let source = SourceFile::new(
            "test.ax",
            "&!3443{G/ping>\"pong\"} &&!3444{\"ping\">\"pong\"}",
        );
        let program = parse_source(&source).expect("parse");
        assert!(matches!(&program.items[0], Item::Server(server) if server.tls));
        assert!(matches!(&program.items[1], Item::Tcp(tcp) if tcp.tls));
    }

    #[test]
    fn parses_http_stream_body_response() {
        let source = SourceFile::new("test.ax", "&3000{P/echo>~}");
        let program = parse_source(&source).expect("parse");
        assert!(
            matches!(&program.items[0], Item::Server(server) if matches!(server.routes[0].response, HttpResponse::StreamBody))
        );
    }

    #[test]
    fn parses_http_wildcard_route() {
        let source = SourceFile::new("test.ax", "&3000{G/assets/*>\"asset\"}");
        let program = parse_source(&source).expect("parse");
        assert!(
            matches!(&program.items[0], Item::Server(server) if server.routes[0].path == "/assets/*")
        );
    }

    #[test]
    fn parses_effectful_function() {
        let source = SourceFile::new("test.ax", "@main():#!io.stdout{C(\"hello\")^0}");
        let program = parse_source(&source).expect("parse");
        assert!(matches!(program.items[0], Item::Function(_)));
    }

    #[test]
    fn parses_async_await() {
        let source = SourceFile::new("test.ax", "@@a():#{^42} {$b=@(a())^b}");
        let program = parse_source(&source).expect("parse");
        assert!(matches!(&program.items[0], Item::Function(function) if function.is_async));
        assert!(matches!(&program.items[1], Item::Function(_)));
    }

    #[test]
    fn parses_unparenthesized_block_conditions() {
        let source = SourceFile::new("test.ax", "{$0~a<3{a=a+1}?a:3{^0}|{^1}}");
        let program = parse_source(&source).expect("parse");
        assert!(matches!(&program.items[0], Item::Function(_)));
    }

    #[test]
    fn rejects_expanded_syntax() {
        for source in [
            "use std.io",
            "fn main() -> i32 { return 0 }",
            "@a()->#{^0}",
            "server :3000 { GET /ping => text \"pong\" }",
            "tcp :3000 { \"ping\" => \"pong\" }",
            "{ let a = 1 return a }",
            "{ if true { return 0 } else { return 1 } }",
            "test \"old\" { assert true }",
        ] {
            let source = SourceFile::new("old.ax", source);
            assert!(parse_source(&source).is_err(), "accepted {source:?}");
        }
    }

    #[test]
    fn parses_ai_min_symbol_aliases() {
        let source = SourceFile::new(
            "test.ax",
            "%a{a:#,b:$} %%b{a,b} %!c{a,b} @@d(e:#):#{^e} @main():#{$f=a{a:1,b:\"x\"}$g=b.a ?(g:b.a&g!:b.b|!0){Ia(\"x\") ^@(d(f.a))}|{~(f.a<3){f.a=f.a+1}~{^1}}} &!3443{G/ping>\"pong\"P/echo>~G/state>#{ok:!1,no:!0}} &&!3444{\"ping\">\"pong\"*>\"fallback\"}",
        );
        let program = parse_source(&source).expect("parse");
        assert!(matches!(&program.items[0], Item::Type(_)));
        assert!(matches!(&program.items[1], Item::Enum(_)));
        assert!(matches!(&program.items[2], Item::Error(_)));
        assert!(matches!(&program.items[3], Item::Function(function) if function.is_async));
        assert!(matches!(&program.items[4], Item::Function(_)));
        assert!(
            matches!(&program.items[5], Item::Server(server) if server.tls && matches!(server.routes[0].method, HttpMethod::Get) && matches!(server.routes[1].response, HttpResponse::StreamBody))
        );
        assert!(matches!(&program.items[6], Item::Tcp(tcp) if tcp.tls));
        let Item::Function(main) = &program.items[4] else {
            panic!("expected main");
        };
        let Stmt::If { then_block, .. } = &main.body.stmts[2] else {
            panic!("expected if");
        };
        let Stmt::Expr {
            expr: Expr::Call { callee, .. },
            ..
        } = &then_block.stmts[0]
        else {
            panic!("expected call");
        };
        assert_eq!(expr_path(callee).as_deref(), Some("io.println"));
    }

    #[test]
    fn parses_ai_min_string_pool() {
        let source = SourceFile::new("test.ax", "@{#(a\"x\",b\"y\");a;b^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        assert_eq!(main.name, "main");
        assert_eq!(main.ret.name, "i32");

        assert!(matches!(
            &main.body.stmts[0],
            Stmt::Let {
                name,
                expr: Expr::Str(value, _),
                ..
            } if name == "a" && value == "x"
        ));
        assert!(matches!(
            &main.body.stmts[1],
            Stmt::Let {
                name,
                expr: Expr::Str(value, _),
                ..
            } if name == "b" && value == "y"
        ));
        let Stmt::Expr {
            expr: Expr::Call { callee, .. },
            ..
        } = &main.body.stmts[2]
        else {
            panic!("expected println");
        };
        assert_eq!(expr_path(callee).as_deref(), Some("io.println"));
    }

    #[test]
    fn parses_ai_min_bare_main_block() {
        let source = SourceFile::new("test.ax", "{;\"hello\"}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert_eq!(main.name, "main");
        assert_eq!(main.ret.name, "i32");
    }

    #[test]
    fn parses_ai_min_implicit_string_pool() {
        let source = SourceFile::new("test.ax", "@{#(\"x\",\"y\",\"z\");a;b;c^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert!(matches!(
            &main.body.stmts[0],
            Stmt::Let {
                name,
                expr: Expr::Str(value, _),
                ..
            } if name == "a" && value == "x"
        ));
        assert!(matches!(
            &main.body.stmts[2],
            Stmt::Let {
                name,
                expr: Expr::Str(value, _),
                ..
            } if name == "c" && value == "z"
        ));
    }

    #[test]
    fn parses_ai_min_implicit_lets() {
        let source = SourceFile::new("test.ax", "@{#(\"x\",\"y\")$1$2^c+d}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert!(matches!(
            &main.body.stmts[2],
            Stmt::Let {
                name,
                expr: Expr::Int(value, _),
                ..
            } if name == "c" && *value == 1
        ));
        assert!(matches!(
            &main.body.stmts[3],
            Stmt::Let {
                name,
                expr: Expr::Int(value, _),
                ..
            } if name == "d" && *value == 2
        ));
    }

    #[test]
    fn parses_ai_min_implicit_lets_in_isolated_if_block() {
        let source = SourceFile::new("test.ax", "@{$1?a:1{$2^b}$3^b}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        let Stmt::If { then_block, .. } = &main.body.stmts[1] else {
            panic!("expected if");
        };
        assert!(matches!(
            &then_block.stmts[0],
            Stmt::Let {
                name,
                expr: Expr::Int(value, _),
                ..
            } if name == "b" && *value == 2
        ));
        assert!(matches!(
            &main.body.stmts[2],
            Stmt::Let {
                name,
                expr: Expr::Int(value, _),
                ..
            } if name == "b" && *value == 3
        ));
    }

    #[test]
    fn parses_ai_min_semicolon_std_statements() {
        let source = SourceFile::new("test.ax", "@{$a=\"p\"$b=\"v\";a,b;;a,b;+a^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        let callees = main
            .body
            .stmts
            .iter()
            .skip(2)
            .take(3)
            .map(|stmt| {
                let Stmt::Expr {
                    expr: Expr::Call { callee, .. },
                    ..
                } = stmt
                else {
                    panic!("expected call statement");
                };
                expr_path(callee).expect("callee path")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            callees,
            ["fs.write_text", "fs.write_json_atomic", "fs.mkdir"]
        );
    }

    #[test]
    fn parses_ai_min_json_object_expr() {
        let source = SourceFile::new(
            "test.ax",
            "@{$a={\"items\":\"true\"}$b={\"name\"=\"agent\"}^0}",
        );
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected object let");
        };
        assert_eq!(expr_path(callee).as_deref(), Some("json.object"));
        let Expr::Call { callee, .. } = &args[0] else {
            panic!("expected pair");
        };
        assert_eq!(expr_path(callee).as_deref(), Some("json.pair"));

        let Stmt::Let {
            expr: Expr::Call { args, .. },
            ..
        } = &main.body.stmts[1]
        else {
            panic!("expected string object let");
        };
        let Expr::Call { callee, .. } = &args[0] else {
            panic!("expected string pair");
        };
        assert_eq!(expr_path(callee).as_deref(), Some("json.string_pair"));
    }

    #[test]
    fn parses_ai_min_json_at_postfix() {
        let source = SourceFile::new("test.ax", "@{$a=b[c,0]^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected json at");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("json.at"));
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn parses_numeric_std_call_alias() {
        let source = SourceFile::new("test.ax", "@{$a=0(b,c)^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected call");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("json.query_len"));
    }

    #[test]
    fn parses_zero_arg_std_call_alias_without_parens() {
        let source = SourceFile::new("test.ax", "@{$Ta^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected call");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("time.now"));
        assert!(args.is_empty());
    }

    #[test]
    fn parses_ai_min_json_query_postfixes() {
        let source = SourceFile::new("test.ax", "@{$a=b`c$b=d'e$c=f#g$d=h\\i^0}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        let callees = main
            .body
            .stmts
            .iter()
            .take(4)
            .map(|stmt| {
                let Stmt::Let {
                    expr: Expr::Call { callee, .. },
                    ..
                } = stmt
                else {
                    panic!("expected call let");
                };
                expr_path(callee).expect("callee path")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            callees,
            [
                "json.query",
                "json.query_bool",
                "json.query_int",
                "json.query_len"
            ]
        );
    }

    #[test]
    fn parses_ai_min_str_contains_postfix() {
        let source = SourceFile::new("test.ax", "{$a=b~c}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected call");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("str.contains"));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn parses_ai_min_str_len_postfix() {
        let source = SourceFile::new("test.ax", "{$a=b!}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected call");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("str.len"));
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn parses_ai_min_fs_is_file_postfix() {
        let source = SourceFile::new("test.ax", "{$a=b@}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };
        let Stmt::Let {
            expr: Expr::Call { callee, args, .. },
            ..
        } = &main.body.stmts[0]
        else {
            panic!("expected call");
        };

        assert_eq!(expr_path(callee).as_deref(), Some("fs.is_file"));
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn leaves_spaced_tilde_for_while_statement() {
        let source = SourceFile::new("test.ax", "{$a=0 ~a<2{a=a+1}^a}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert!(matches!(main.body.stmts[0], Stmt::Let { .. }));
        assert!(matches!(main.body.stmts[1], Stmt::While { .. }));
    }

    #[test]
    fn leaves_question_for_if_statement_after_expression() {
        let source = SourceFile::new("test.ax", "{$a=0?a{^0}^1}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert!(matches!(main.body.stmts[0], Stmt::Let { .. }));
        assert!(matches!(main.body.stmts[1], Stmt::If { .. }));
    }

    #[test]
    fn leaves_spaced_at_for_await_expression() {
        let source = SourceFile::new("test.ax", "{$a=0 @(b)^a}");
        let program = parse_source(&source).expect("parse");
        let Item::Function(main) = &program.items[0] else {
            panic!("expected main");
        };

        assert!(matches!(main.body.stmts[0], Stmt::Let { .. }));
        assert!(matches!(
            main.body.stmts[1],
            Stmt::Expr {
                expr: Expr::Await { .. },
                ..
            }
        ));
    }
}
