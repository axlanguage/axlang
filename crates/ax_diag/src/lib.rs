use ax_core::{SourceFile, Span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: Span,
    pub severity: Severity,
    pub help: Option<String>,
}

pub type AxResult<T> = Result<T, Diagnostic>;

impl Diagnostic {
    pub fn error(code: impl Into<String>, message: impl Into<String>, span: Span) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span,
            severity: Severity::Error,
            help: None,
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn render(&self, source: &SourceFile) -> String {
        let mut out = format!(
            "{}[{}]: {}\n --> {}:{}:{}\n",
            self.severity.as_str(),
            self.code,
            self.message,
            source.path.display(),
            self.span.line,
            self.span.column
        );
        if let Some(line) = source.line_text(self.span.line) {
            out.push_str("  |\n");
            out.push_str(&format!("{:>2} | {}\n", self.span.line, line));
            let caret_width = self.span.end.saturating_sub(self.span.start).max(1);
            let padding = " ".repeat(self.span.column.saturating_sub(1));
            out.push_str(&format!(
                "  | {}{}\n",
                padding,
                "^".repeat(caret_width.min(48))
            ));
        }
        if let Some(help) = &self.help {
            out.push_str(&format!("help: {}\n", help));
        }
        out
    }

    pub fn to_json(&self, source: &SourceFile) -> String {
        format!(
            "{{\"severity\":\"{}\",\"code\":\"{}\",\"message\":\"{}\",\"path\":\"{}\",\"line\":{},\"column\":{},\"help\":{}}}",
            self.severity.as_str(),
            escape_json(&self.code),
            escape_json(&self.message),
            escape_json(&source.path.display().to_string()),
            self.span.line,
            self.span.column,
            self.help
                .as_ref()
                .map(|h| format!("\"{}\"", escape_json(h)))
                .unwrap_or_else(|| "null".to_string())
        )
    }
}

fn escape_json(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            c => vec![c],
        })
        .collect()
}
