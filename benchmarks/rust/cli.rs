fn has_arg(args: &[String], name: &str) -> bool {
    args.iter()
        .skip(1)
        .any(|arg| arg == name || arg.starts_with(&format!("{name}=")))
}

fn value_arg(args: &[String], name: &str) -> String {
    let equals = format!("{name}=");
    let mut iter = args.iter().skip(1).peekable();
    while let Some(arg) = iter.next() {
        if arg.starts_with(&equals) {
            return arg[equals.len()..].to_string();
        }
        if arg == name {
            return iter.peek().map(|value| (*value).to_string()).unwrap_or_default();
        }
    }
    String::new()
}

fn cli_score(args: &[String], n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    while i < n {
        if has_arg(args, "--input") {
            acc += value_arg(args, "--input").len() as i32;
        }
        if has_arg(args, "--mode") {
            acc += value_arg(args, "--mode").len() as i32;
        }
        i += 1;
    }
    acc % 251
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    std::process::exit(cli_score(&args, 100_000));
}
