use std::io::{Read, Write};
use std::net::TcpListener;

fn respond(mut stream: std::net::TcpStream, body: &str, content_type: &str) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        content_type,
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:3202").expect("listen");
    for stream in listener.incoming().flatten() {
        let mut buffer = [0_u8; 1024];
        let mut stream = stream;
        let read_len = stream.read(&mut buffer).unwrap_or(0);
        let request = std::str::from_utf8(&buffer[..read_len]).unwrap_or("");
        if request.starts_with("GET /health ") {
            respond(stream, "{\"ok\":true,\"service\":\"ax\"}", "application/json");
        } else if request.starts_with("GET /ping ") {
            respond(stream, "pong", "text/plain");
        } else {
            let body = "not found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    }
}
