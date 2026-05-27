const http = require("node:http");

const server = http.createServer((req, res) => {
  if (req.url === "/health") {
    const body = '{"ok":true,"service":"ax"}';
    res.writeHead(200, {
      "Content-Type": "application/json",
      "Content-Length": Buffer.byteLength(body),
      "Connection": "close",
    });
    res.end(body);
  } else if (req.url === "/ping") {
    res.writeHead(200, {
      "Content-Type": "text/plain",
      "Content-Length": 4,
      "Connection": "close",
    });
    res.end("pong");
  } else {
    res.writeHead(404, { "Content-Length": 9, "Connection": "close" });
    res.end("not found");
  }
});

server.listen(3204, "127.0.0.1");
