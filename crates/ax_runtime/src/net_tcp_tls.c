#include "ax_runtime.h"

#ifdef _WIN32

#include <stdio.h>

int ax_tcp_tls_server_start(int port, const ax_tcp_route* routes, int route_count) {
  (void)routes;
  (void)route_count;
  fprintf(stderr, "ax tcp tls listen :%d is not supported by the Windows runtime yet\n", port);
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
#include <sys/socket.h>
#include <unistd.h>

typedef struct {
  int client;
  SSL_CTX* ssl_ctx;
  const ax_tcp_route* routes;
  int route_count;
} ax_tcp_tls_client;

static int ax_tcp_tls_create_listener(int port) {
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

static char* ax_tcp_tls_trim(char* value) {
  while (*value == ' ' || *value == '\t' || *value == '\r' || *value == '\n') {
    value++;
  }
  size_t len = strlen(value);
  while (len > 0) {
    char ch = value[len - 1];
    if (ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n') {
      value[len - 1] = 0;
      len--;
    } else {
      break;
    }
  }
  return value;
}

static const ax_tcp_route* ax_tcp_tls_find_route(
  const ax_tcp_route* routes,
  int route_count,
  const char* msg
) {
  const ax_tcp_route* wildcard = 0;
  for (int i = 0; i < route_count; i++) {
    if (routes[i].is_wildcard) {
      if (wildcard == 0) {
        wildcard = &routes[i];
      }
      continue;
    }
    if (strcmp(routes[i].pattern, msg) == 0) {
      return &routes[i];
    }
  }
  return wildcard;
}

static void ax_tcp_tls_write_all(SSL* ssl, const char* data, size_t len) {
  while (len > 0) {
    int n = SSL_write(ssl, data, (int)len);
    if (n <= 0) {
      return;
    }
    data += n;
    len -= (size_t)n;
  }
}

static void ax_tcp_tls_handle_client(int client, SSL_CTX* ssl_ctx, const ax_tcp_route* routes, int route_count) {
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

  char buffer[1024];
  int n = SSL_read(ssl, buffer, sizeof(buffer) - 1);
  if (n >= 0) {
    buffer[n] = 0;
    char* msg = ax_tcp_tls_trim(buffer);
    const ax_tcp_route* route = ax_tcp_tls_find_route(routes, route_count, msg);
    const char* response = route == 0 ? "" : route->response;
    ax_tcp_tls_write_all(ssl, response, strlen(response));
  }

  SSL_shutdown(ssl);
  SSL_free(ssl);
  close(client);
}

static void* ax_tcp_tls_client_thread(void* data) {
  ax_tcp_tls_client* client = (ax_tcp_tls_client*)data;
  ax_tcp_tls_handle_client(client->client, client->ssl_ctx, client->routes, client->route_count);
  free(client);
  return 0;
}

static SSL_CTX* ax_tcp_tls_create_context(void) {
  const char* cert = getenv("AX_TCP_TLS_CERT");
  const char* key = getenv("AX_TCP_TLS_KEY");
  if (cert == 0 || key == 0 || cert[0] == 0 || key[0] == 0) {
    fprintf(stderr, "AX_TCP_TLS_CERT and AX_TCP_TLS_KEY are required for TLS servers\n");
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

int ax_tcp_tls_server_start(int port, const ax_tcp_route* routes, int route_count) {
  signal(SIGPIPE, SIG_IGN);
  SSL_library_init();
  SSL_load_error_strings();

  SSL_CTX* ssl_ctx = ax_tcp_tls_create_context();
  if (ssl_ctx == 0) {
    return 1;
  }

  int server = ax_tcp_tls_create_listener(port);
  if (server < 0) {
    SSL_CTX_free(ssl_ctx);
    return 1;
  }
  printf("ax tcp tls listen :%d\n", port);
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

    ax_tcp_tls_client* ctx = (ax_tcp_tls_client*)malloc(sizeof(ax_tcp_tls_client));
    if (ctx != 0) {
      ctx->client = client;
      ctx->ssl_ctx = ssl_ctx;
      ctx->routes = routes;
      ctx->route_count = route_count;
      pthread_t thread;
      if (pthread_create(&thread, 0, ax_tcp_tls_client_thread, ctx) == 0) {
        pthread_detach(thread);
        continue;
      }
      free(ctx);
    }

    ax_tcp_tls_handle_client(client, ssl_ctx, routes, route_count);
  }
}

#endif
