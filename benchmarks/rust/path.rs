fn normalize_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_sep = false;
    for ch in value.replace('\\', "/").split('/') {
        if ch.is_empty() || ch == "." {
            last_sep = true;
            continue;
        }
        if !out.is_empty() && !last_sep {
            out.push('/');
        } else if !out.is_empty() {
            out.push('/');
        }
        out.push_str(ch);
        last_sep = false;
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out
    }
}

fn basename_path(value: &str) -> &str {
    value.rsplit('/').next().unwrap_or(value)
}

fn path_score(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    let text = "examples//agents/./string_agent.ax";
    while i < n {
        let normalized = normalize_path(text);
        acc += basename_path(&normalized).len() as i32;
        acc += if text.starts_with('/') || text.starts_with('\\') { 1 } else { 2 };
        i += 1;
    }
    acc % 251
}

fn main() {
    std::process::exit(path_score(1_000_000));
}
