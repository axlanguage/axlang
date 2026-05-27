fn mix(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    while i < n {
        acc = (acc + (i * 31)) % 1_000_003;
        i += 1;
    }
    acc
}

fn main() {
    std::process::exit(mix(10_000_000));
}
