fn text_score(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    let text = std::env::var("AX_TEXT").unwrap_or_else(|_| "agent-native-compiler-runtime-pack".to_string());
    while i < n {
        if text.contains("runtime") {
            acc += text.len() as i32;
        }
        if text.starts_with("agent") {
            acc += 1;
        }
        if text.ends_with("pack") {
            acc += 2;
        }
        i += 1;
    }
    acc % 251
}

fn main() {
    std::process::exit(text_score(5_000_000));
}
