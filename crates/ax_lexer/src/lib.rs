use ax_core::{SourceFile, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Int(String),
    Float(String),
    Str(String),
    Keyword(String),
    Symbol(String),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn lex(source: &SourceFile) -> Vec<Token> {
    Lexer::new(source).lex_all()
}

struct Lexer<'a> {
    source: &'a SourceFile,
    chars: Vec<(usize, char)>,
    index: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a SourceFile) -> Self {
        Self {
            source,
            chars: source.text.char_indices().collect(),
            index: 0,
        }
    }

    fn lex_all(mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while let Some((start, ch)) = self.peek() {
            if ch.is_whitespace() {
                self.bump();
                continue;
            }
            if ch == '/' && self.peek_n(1).is_some_and(|(_, c)| c == '/') {
                self.bump();
                self.bump();
                while let Some((_, c)) = self.peek() {
                    self.bump();
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            let token = if is_ident_start(ch) {
                self.lex_ident_or_keyword(start)
            } else if ch.is_ascii_digit() {
                self.lex_number(start)
            } else if ch == '"' {
                self.lex_string(start)
            } else {
                self.lex_symbol(start)
            };
            tokens.push(token);
        }
        let eof_span = self
            .source
            .span_at(self.source.text.len(), self.source.text.len());
        tokens.push(Token {
            kind: TokenKind::Eof,
            span: eof_span,
        });
        tokens
    }

    fn lex_ident_or_keyword(&mut self, start: usize) -> Token {
        let mut value = String::new();
        while let Some((_, ch)) = self.peek() {
            if is_ident_continue(ch) {
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        let kind = if is_keyword(&value) {
            TokenKind::Keyword(value)
        } else {
            TokenKind::Ident(value)
        };
        let end = self.offset();
        Token {
            kind,
            span: self.source.span_at(start, end),
        }
    }

    fn lex_number(&mut self, start: usize) -> Token {
        let mut value = String::new();
        let mut is_float = false;
        while let Some((_, ch)) = self.peek() {
            if ch.is_ascii_digit() {
                value.push(ch);
                self.bump();
            } else if ch == '.'
                && !is_float
                && self.peek_n(1).is_some_and(|(_, c)| c.is_ascii_digit())
            {
                is_float = true;
                value.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        let end = self.offset();
        Token {
            kind: if is_float {
                TokenKind::Float(value)
            } else {
                TokenKind::Int(value)
            },
            span: self.source.span_at(start, end),
        }
    }

    fn lex_string(&mut self, start: usize) -> Token {
        self.bump();
        let mut value = String::new();
        while let Some((_, ch)) = self.peek() {
            self.bump();
            match ch {
                '"' => break,
                '\\' => {
                    if let Some((_, escaped)) = self.peek() {
                        self.bump();
                        value.push(match escaped {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            '"' => '"',
                            '\\' => '\\',
                            other => other,
                        });
                    }
                }
                other => value.push(other),
            }
        }
        let end = self.offset();
        Token {
            kind: TokenKind::Str(value),
            span: self.source.span_at(start, end),
        }
    }

    fn lex_symbol(&mut self, start: usize) -> Token {
        let (_, ch) = self.bump().expect("peeked before lex_symbol");
        let mut value = ch.to_string();
        if let Some((_, next)) = self.peek() {
            let pair = format!("{}{}", ch, next);
            if matches!(pair.as_str(), "!:" | "<=" | ">=" | "&&") {
                value = pair;
                self.bump();
            }
        }
        let end = self.offset();
        Token {
            kind: TokenKind::Symbol(value),
            span: self.source.span_at(start, end),
        }
    }

    fn peek(&self) -> Option<(usize, char)> {
        self.chars.get(self.index).copied()
    }

    fn peek_n(&self, n: usize) -> Option<(usize, char)> {
        self.chars.get(self.index + n).copied()
    }

    fn bump(&mut self) -> Option<(usize, char)> {
        let item = self.peek()?;
        self.index += 1;
        Some(item)
    }

    fn offset(&self) -> usize {
        self.peek()
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| self.source.text.len())
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "use"
            | "fn"
            | "async"
            | "await"
            | "type"
            | "enum"
            | "error"
            | "let"
            | "return"
            | "if"
            | "else"
            | "loop"
            | "while"
            | "test"
            | "true"
            | "false"
            | "server"
            | "tcp"
            | "assert"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_http_route() {
        let source = SourceFile::new("test.ax", "&3000{G/ping>\"pong\"}");
        let tokens = lex(&source);
        assert!(tokens
            .iter()
            .any(|t| t.kind == TokenKind::Symbol("&".into())));
        assert!(tokens
            .iter()
            .any(|t| t.kind == TokenKind::Symbol(">".into())));
        assert!(tokens
            .iter()
            .any(|t| t.kind == TokenKind::Str("pong".into())));
    }
}
