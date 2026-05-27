fn encode_url(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

fn decode_url(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn authority_start(value: &str) -> &str {
    value.split_once("://").map(|(_, rest)| rest).unwrap_or(value)
}

fn host_url(value: &str) -> &str {
    let rest = authority_start(value);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_end = authority.find(':').unwrap_or(authority.len());
    &authority[..host_end]
}

fn path_url(value: &str) -> &str {
    let rest = authority_start(value);
    let Some(start) = rest.find('/') else {
        return "/";
    };
    let end = rest[start..]
        .find(['?', '#'])
        .map(|offset| start + offset)
        .unwrap_or(rest.len());
    &rest[start..end]
}

fn query_get(value: &str, key: &str) -> String {
    let query = value.split_once('?').map(|(_, rest)| rest).unwrap_or(value);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if decode_url(name) == key {
            return decode_url(value);
        }
    }
    String::new()
}

fn url_score(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    let target = "https://agent.local/tools/search?q=Ax%20language&mode=fast";
    while i < n {
        let host = host_url(target);
        let path = path_url(target);
        let query = query_get(target, "q");
        let encoded = encode_url(path);
        let decoded = decode_url(&encoded);
        acc += host.len() as i32;
        acc += decoded.len() as i32;
        acc += query.len() as i32;
        i += 1;
    }
    acc % 251
}

fn main() {
    std::process::exit(url_score(100_000));
}
