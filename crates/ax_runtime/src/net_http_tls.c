#include "ax_runtime.h"

#ifdef _WIN32

#include <stdio.h>

int ax_http_tls_server_start(int port, const ax_http_route* routes, int route_count) {
  (void)routes;
  (void)route_count;
  fprintf(stderr, "ax http tls listen :%d is not supported by the Windows runtime yet\n", port);
  return 1;
}

#else

#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <openssl/err.h>
#include <openssl/ssl.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <unistd.h>

#define AX_HTTPS_BUFFER_SIZE 16384

typedef struct {
  int client;
  SSL_CTX* ssl_ctx;
  const ax_http_route* routes;
  int route_count;
} ax_https_client;

static int ax_tls_create_listener(int port) {
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) {
    perror("socket");
    return -1;
  }

  int yes = 1;
  setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));

  struct sockaddr_in addr;
  memset(&addr, 0, sizeof(addr));
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_ANY);
  addr.sin_port = htons((uint16_t)port);

  if (bind(fd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
    perror("bind");
    close(fd);
    return -1;
  }

  if (listen(fd, 64) < 0) {
    perror("listen");
    close(fd);
    return -1;
  }

  return fd;
}

static char* ax_https_find_header_end(char* buffer, size_t len) {
  if (len < 4) {
    return 0;
  }
  for (size_t i = 0; i + 3 < len; i++) {
    if (buffer[i] == '\r' && buffer[i + 1] == '\n' && buffer[i + 2] == '\r' && buffer[i + 3] == '\n') {
      return buffer + i + 4;
    }
  }
  return 0;
}

static void ax_https_strip_query(char* path) {
  char* query = strchr(path, '?');
  if (query != 0) {
    *query = 0;
  }
}

static const ax_http_route* ax_https_find_route(
  const ax_http_route* routes,
  int route_count,
  const char* method,
  const char* path
) {
  for (int i = 0; i < route_count; i++) {
    if (strcmp(routes[i].method, method) == 0 && strcmp(routes[i].path, path) == 0) {
      return &routes[i];
    }
  }
  for (int i = 0; i < route_count; i++) {
    size_t route_len = strlen(routes[i].path);
    if (
      route_len > 0 &&
      routes[i].path[route_len - 1] == '*' &&
      strcmp(routes[i].method, method) == 0 &&
      strncmp(routes[i].path, path, route_len - 1) == 0
    ) {
      return &routes[i];
    }
  }
  return 0;
}

static int ax_https_path_exists(const ax_http_route* routes, int route_count, const char* path) {
  for (int i = 0; i < route_count; i++) {
    size_t route_len = strlen(routes[i].path);
    if (strcmp(routes[i].path, path) == 0) {
      return 1;
    }
    if (
      route_len > 0 &&
      routes[i].path[route_len - 1] == '*' &&
      strncmp(routes[i].path, path, route_len - 1) == 0
    ) {
      return 1;
    }
  }
  return 0;
}

static void ax_https_send_all(SSL* ssl, const char* data, size_t len) {
  while (len > 0) {
    int n = SSL_write(ssl, data, (int)len);
    if (n <= 0) {
      return;
    }
    data += n;
    len -= (size_t)n;
  }
}

static void ax_https_send_response(
  SSL* ssl,
  int status,
  const char* status_text,
  const char* content_type,
  const char* body
) {
  size_t body_len = strlen(body);
  char header[512];
  int header_len = snprintf(
    header,
    sizeof(header),
    "HTTP/1.1 %d %s\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
    status,
    status_text,
    content_type,
    body_len
  );
  if (header_len <= 0) {
    return;
  }
  ax_https_send_all(ssl, header, (size_t)header_len);
  ax_https_send_all(ssl, body, body_len);
}

static size_t ax_https_content_length(const char* headers) {
  const char* cursor = headers;
  while (*cursor != 0) {
    const char* line_end = strstr(cursor, "\r\n");
    size_t line_len = line_end == 0 ? strlen(cursor) : (size_t)(line_end - cursor);
    if (line_len >= 15 && strncasecmp(cursor, "Content-Length:", 15) == 0) {
      const char* value = cursor + 15;
      while (*value == ' ' || *value == '\t') {
        value++;
      }
      return (size_t)strtoull(value, 0, 10);
    }
    if (line_end == 0) {
      break;
    }
    cursor = line_end + 2;
  }
  return 0;
}

static void ax_https_stream_body(
  SSL* ssl,
  const char* initial,
  size_t initial_len,
  size_t content_length
) {
  char header[256];
  int header_len = snprintf(
    header,
    sizeof(header),
    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
    content_length
  );
  if (header_len > 0) {
    ax_https_send_all(ssl, header, (size_t)header_len);
  }
  size_t first = initial_len < content_length ? initial_len : content_length;
  if (first > 0) {
    ax_https_send_all(ssl, initial, first);
  }
  size_t remaining = content_length - first;
  char chunk[4096];
  while (remaining > 0) {
    size_t want = remaining < sizeof(chunk) ? remaining : sizeof(chunk);
    int n = SSL_read(ssl, chunk, (int)want);
    if (n <= 0) {
      return;
    }
    ax_https_send_all(ssl, chunk, (size_t)n);
    remaining -= (size_t)n;
  }
}

static void ax_https_handle_client(int client, SSL_CTX* ssl_ctx, const ax_http_route* routes, int route_count) {
  SSL* ssl = SSL_new(ssl_ctx);
  if (ssl == 0) {
    close(client);
    return;
  }
  SSL_set_fd(ssl, client);
  if (SSL_accept(ssl) <= 0) {
    SSL_free(ssl);
    close(client);
    return;
  }

  char buffer[AX_HTTPS_BUFFER_SIZE + 1];
  size_t buffered = 0;
  char* header_end = 0;
  while (header_end == 0 && buffered < AX_HTTPS_BUFFER_SIZE) {
    int n = SSL_read(ssl, buffer + buffered, (int)(AX_HTTPS_BUFFER_SIZE - buffered));
    if (n <= 0) {
      SSL_shutdown(ssl);
      SSL_free(ssl);
      close(client);
      return;
    }
    buffered += (size_t)n;
    header_end = ax_https_find_header_end(buffer, buffered);
  }

  if (header_end == 0) {
    ax_https_send_response(ssl, 400, "Bad Request", "text/plain", "Bad Request");
    SSL_shutdown(ssl);
    SSL_free(ssl);
    close(client);
    return;
  }

  size_t header_len = (size_t)(header_end - buffer);
  char headers[AX_HTTPS_BUFFER_SIZE + 1];
  memcpy(headers, buffer, header_len);
  headers[header_len] = 0;

  char method[16] = {0};
  char path[512] = {0};
  char version[16] = {0};
  if (sscanf(headers, "%15s %511s %15s", method, path, version) != 3) {
    ax_https_send_response(ssl, 400, "Bad Request", "text/plain", "Bad Request");
  } else {
    (void)version;
    ax_https_strip_query(path);
    const ax_http_route* route = ax_https_find_route(routes, route_count, method, path);
    if (route == 0) {
      if (ax_https_path_exists(routes, route_count, path)) {
        ax_https_send_response(ssl, 405, "Method Not Allowed", "text/plain", "Method Not Allowed");
      } else {
        ax_https_send_response(ssl, 404, "Not Found", "text/plain", "Not Found");
      }
    } else if (route->response_kind == 2) {
      size_t initial_body = buffered > header_len ? buffered - header_len : 0;
      ax_https_stream_body(ssl, buffer + header_len, initial_body, ax_https_content_length(headers));
    } else {
      const char* content_type = route->response_kind == 1 ? "application/json" : "text/plain";
      ax_https_send_response(ssl, 200, "OK", content_type, route->body);
    }
  }

  SSL_shutdown(ssl);
  SSL_free(ssl);
  close(client);
}

static void* ax_https_client_thread(void* data) {
  ax_https_client* client = (ax_https_client*)data;
  ax_https_handle_client(client->client, client->ssl_ctx, client->routes, client->route_count);
  free(client);
  return 0;
}

static SSL_CTX* ax_https_create_context(void) {
  const char* cert = getenv("AX_HTTP_TLS_CERT");
  const char* key = getenv("AX_HTTP_TLS_KEY");
  if (cert == 0 || key == 0 || cert[0] == 0 || key[0] == 0) {
    fprintf(stderr, "AX_HTTP_TLS_CERT and AX_HTTP_TLS_KEY are required for TLS servers\n");
    return 0;
  }

  SSL_CTX* ctx = SSL_CTX_new(TLS_server_method());
  if (ctx == 0) {
    ERR_print_errors_fp(stderr);
    return 0;
  }
  if (SSL_CTX_use_certificate_file(ctx, cert, SSL_FILETYPE_PEM) <= 0 ||
      SSL_CTX_use_PrivateKey_file(ctx, key, SSL_FILETYPE_PEM) <= 0 ||
      SSL_CTX_check_private_key(ctx) <= 0) {
    ERR_print_errors_fp(stderr);
    SSL_CTX_free(ctx);
    return 0;
  }
  return ctx;
}

int ax_http_tls_server_start(int port, const ax_http_route* routes, int route_count) {
  signal(SIGPIPE, SIG_IGN);
  SSL_library_init();
  SSL_load_error_strings();

  SSL_CTX* ssl_ctx = ax_https_create_context();
  if (ssl_ctx == 0) {
    return 1;
  }

  int server = ax_tls_create_listener(port);
  if (server < 0) {
    SSL_CTX_free(ssl_ctx);
    return 1;
  }
  printf("ax https listen :%d\n", port);
  fflush(stdout);

  for (;;) {
    int client = accept(server, 0, 0);
    if (client < 0) {
      if (errno == EINTR) {
        continue;
      }
      perror("accept");
      close(server);
      SSL_CTX_free(ssl_ctx);
      return 1;
    }

    ax_https_client* ctx = (ax_https_client*)malloc(sizeof(ax_https_client));
    if (ctx != 0) {
      ctx->client = client;
      ctx->ssl_ctx = ssl_ctx;
      ctx->routes = routes;
      ctx->route_count = route_count;
      pthread_t thread;
      if (pthread_create(&thread, 0, ax_https_client_thread, ctx) == 0) {
        pthread_detach(thread);
        continue;
      }
      free(ctx);
    }

    ax_https_handle_client(client, ssl_ctx, routes, route_count);
  }
}

#endif
