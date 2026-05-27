const PAYLOAD: &str = "{ \"task\" : \"summarize\", \"ok\" : true, \"tokens\" : 2048, \"agent\" : { \"name\" : \"codex\", \"limits\" : { \"tokens\" : 2048 } }, \"steps\" : [{ \"name\" : \"read\" }, { \"name\" : \"verify\" }], \"tools\" : [\"read\", \"write\", \"verify\"] }";

fn compact_json(value: &str) -> String {
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

fn value_len_after(value: &str, marker: &str) -> i32 {
    let Some(start) = value.find(marker).map(|index| index + marker.len()) else {
        return 0;
    };
    value[start..].find('"').unwrap_or(0) as i32
}

fn int_after(value: &str, marker: &str) -> i32 {
    let Some(start) = value.find(marker).map(|index| index + marker.len()) else {
        return 0;
    };
    let end = value[start..]
        .find(|ch: char| !ch.is_ascii_digit())
        .map(|offset| start + offset)
        .unwrap_or(value.len());
    value[start..end].parse().unwrap_or(0)
}

fn array_segment<'a>(value: &'a str, marker: &str) -> Option<&'a str> {
    let start = value.find(marker)? + marker.len();
    let end = value[start..].find(']')? + start;
    Some(&value[start..end])
}

fn array_len(value: &str, marker: &str) -> i32 {
    let Some(segment) = array_segment(value, marker) else {
        return 0;
    };
    if segment.is_empty() {
        0
    } else {
        segment.matches(',').count() as i32 + 1
    }
}

fn array_contains(value: &str, marker: &str, needle: &str) -> bool {
    array_segment(value, marker)
        .map(|segment| segment.contains(needle))
        .unwrap_or(false)
}

fn json_score(n: i32) -> i32 {
    let mut i = 0;
    let mut acc = 0;
    while i < n {
        let compact = compact_json(PAYLOAD);
        if compact.contains("\"task\":") && compact.contains("\"ok\":true") {
            let task = value_len_after(&compact, "\"task\":\"");
            let agent = value_len_after(&compact, "\"agent\":{\"name\":\"");
            let first_step = value_len_after(&compact, "\"steps\":[{\"name\":\"");
            let first_tool = value_len_after(&compact, "\"tools\":[\"");
            let tokens = int_after(&compact, "\"limits\":{\"tokens\":");
            let tool_count = array_len(&compact, "\"tools\":[");
            if tokens == 2048
                && array_contains(&compact, "\"tools\":[", "\"verify\"")
                && compact.contains("\"steps\"")
                && compact.contains("\"limits\":{\"tokens\":2048}")
            {
                acc += task;
                acc += agent;
                acc += first_step;
                acc += "string".len() as i32;
                acc += "number".len() as i32;
                acc += first_tool;
                acc += tool_count;
                acc += tokens % 97;
            }
        }
        i += 1;
    }
    acc % 251
}

fn main() {
    std::process::exit(json_score(10_000));
}
