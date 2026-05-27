#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static void send_all(int fd, const char* data, size_t len) {
  while (len > 0) {
    ssize_t sent = send(fd, data, len, 0);
    if (sent <= 0) return;
    data += sent;
    len -= (size_t)sent;
  }
}

static void respond(int fd, const char* body, const char* content_type) {
  char header[256];
  int header_len = snprintf(
      header,
      sizeof(header),
      "HTTP/1.1 200 OK\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
      content_type,
      strlen(body));
  send_all(fd, header, (size_t)header_len);
  send_all(fd, body, strlen(body));
}

int main(void) {
  int server = socket(AF_INET, SOCK_STREAM, 0);
  int yes = 1;
  setsockopt(server, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));
  struct sockaddr_in addr;
  memset(&addr, 0, sizeof(addr));
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = htons(3201);
  if (bind(server, (struct sockaddr*)&addr, sizeof(addr)) != 0) return 1;
  if (listen(server, 128) != 0) return 1;
  for (;;) {
    int client = accept(server, 0, 0);
    if (client < 0) continue;
    char buffer[1024];
    ssize_t read_len = recv(client, buffer, sizeof(buffer) - 1, 0);
    if (read_len > 0) {
      buffer[read_len] = 0;
      if (strncmp(buffer, "GET /health ", 12) == 0) {
        respond(client, "{\"ok\":true,\"service\":\"ax\"}", "application/json");
      } else if (strncmp(buffer, "GET /ping ", 10) == 0) {
        respond(client, "pong", "text/plain");
      } else {
        const char* body = "not found";
        char header[256];
        int header_len = snprintf(
            header,
            sizeof(header),
            "HTTP/1.1 404 Not Found\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
            strlen(body));
        send_all(client, header, (size_t)header_len);
        send_all(client, body, strlen(body));
      }
    }
    close(client);
  }
}
