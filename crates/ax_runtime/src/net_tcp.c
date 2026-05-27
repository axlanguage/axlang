#include "ax_runtime.h"

#ifdef _WIN32

#include <stdio.h>
#include <stdlib.h>

int ax_tcp_server_start(int port, const ax_tcp_route* routes, int route_count) {
  (void)routes;
  (void)route_count;
  fprintf(stderr, "ax tcp listen :%d is not supported by the Windows runtime yet\n", port);
  return 1;
}

int ax_tcp_ping_server(int port) {
  return ax_tcp_server_start(port, 0, 0);
}

void* ax_tcp_listen(int port) {
  (void)port;
  fprintf(stderr, "tcp.listen is not supported by the Windows runtime yet\n");
  return 0;
}

void* ax_tcp_listen_on(const char* host, int port) {
  (void)host;
  (void)port;
  fprintf(stderr, "tcp.listen(host, port) is not supported by the Windows runtime yet\n");
  return 0;
}

void* ax_tcp_connect(const char* host, int port) {
  (void)host;
  (void)port;
  fprintf(stderr, "tcp.connect is not supported by the Windows runtime yet\n");
  return 0;
}

int ax_tcp_serve_text(void* server, void* state, ax_tcp_text_handler handler, int workers) {
  (void)server;
  (void)state;
  (void)handler;
  (void)workers;
  fprintf(stderr, "tcp.serve_text is not supported by the Windows runtime yet\n");
  return 1;
}

void* ax_tcp_accept(void* server) {
  (void)server;
  return 0;
}

char* ax_tcp_read_text(void* conn, int max_bytes) {
  (void)conn;
  (void)max_bytes;
  char* out = (char*)malloc(1);
  if (out != 0) {
    out[0] = 0;
  }
  return out;
}

void ax_tcp_write_text(void* conn, const char* text) {
  (void)conn;
  (void)text;
}

char* ax_tcp_request_text(void* conn, const char* text, int max_bytes) {
  (void)text;
  return ax_tcp_read_text(conn, max_bytes);
}

void ax_tcp_close(void* conn) {
  (void)conn;
}

#else

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

typedef struct {
  int fd;
} ax_tcp_server_handle;

typedef struct {
  int fd;
  char read_buffer[65536];
  size_t read_head;
  size_t read_len;
  char write_buffer[65536];
  size_t write_len;
  int close_after_write;
  short event_revents;
} ax_tcp_conn_handle;

#define AX_TCP_READ_CAP ((size_t)65536)
#define AX_TCP_READ_MASK (AX_TCP_READ_CAP - 1)
#define AX_TCP_WRITE_CAP ((size_t)65536)

static char* ax_tcp_empty_string(void) {
  char* out = (char*)malloc(1);
  if (out != 0) {
    out[0] = 0;
  }
  return out;
}

static void ax_tcp_configure_conn(int fd) {
  int yes = 1;
  setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &yes, sizeof(yes));
#ifdef SO_NOSIGPIPE
  setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &yes, sizeof(yes));
#endif
}

static void ax_tcp_conn_init(ax_tcp_conn_handle* conn, int fd) {
  conn->fd = fd;
  conn->read_head = 0;
  conn->read_len = 0;
  conn->write_len = 0;
  conn->close_after_write = 0;
  conn->event_revents = 0;
}

static void ax_tcp_set_nonblocking(int fd) {
  int flags = fcntl(fd, F_GETFL, 0);
  if (flags >= 0) {
    fcntl(fd, F_SETFL, flags | O_NONBLOCK);
  }
}

static int ax_tcp_write_all_fd(int fd, const char* data, size_t len) {
  const char* cursor = data;
  size_t remaining = len;
  while (remaining > 0) {
    ssize_t n = write(fd, cursor, remaining);
    if (n < 0 && errno == EINTR) {
      continue;
    }
    if (n <= 0) {
      return 0;
    }
    cursor += n;
    remaining -= (size_t)n;
  }
  return 1;
}

static void ax_tcp_flush_write(ax_tcp_conn_handle* conn) {
  if (conn == 0 || conn->fd < 0 || conn->write_len == 0) {
    return;
  }
  if (ax_tcp_write_all_fd(conn->fd, conn->write_buffer, conn->write_len)) {
    conn->write_len = 0;
  }
}

static int ax_tcp_append_bytes(char** buffer,
                               size_t* len,
                               size_t* cap,
                               const char* value,
                               size_t value_len,
                               size_t max_len) {
  if (value_len == 0) {
    return 1;
  }
  if (max_len > 0 && *len + value_len > max_len) {
    value_len = max_len > *len ? max_len - *len : 0;
  }
  if (value_len == 0) {
    return 0;
  }
  size_t needed = *len + value_len + 1;
  if (needed > *cap) {
    size_t next = *cap == 0 ? 128 : *cap;
    while (next < needed) {
      next <<= 1;
    }
    char* grown = (char*)realloc(*buffer, next);
    if (grown == 0) {
      free(*buffer);
      *buffer = 0;
      *len = 0;
      *cap = 0;
      return 0;
    }
    *buffer = grown;
    *cap = next;
  }
  memcpy(*buffer + *len, value, value_len);
  *len += value_len;
  (*buffer)[*len] = 0;
  return 1;
}

static size_t ax_tcp_min_size(size_t a, size_t b) {
  return a < b ? a : b;
}

static size_t ax_tcp_ring_tail(const ax_tcp_conn_handle* conn) {
  return (conn->read_head + conn->read_len) & AX_TCP_READ_MASK;
}

static size_t ax_tcp_ring_contiguous_used(const ax_tcp_conn_handle* conn) {
  size_t first = AX_TCP_READ_CAP - conn->read_head;
  return ax_tcp_min_size(conn->read_len, first);
}

static size_t ax_tcp_ring_contiguous_free(const ax_tcp_conn_handle* conn) {
  if (conn->read_len >= AX_TCP_READ_CAP) {
    return 0;
  }
  size_t tail = ax_tcp_ring_tail(conn);
  size_t free_len = AX_TCP_READ_CAP - conn->read_len;
  size_t until_wrap = AX_TCP_READ_CAP - tail;
  if (tail < conn->read_head) {
    until_wrap = conn->read_head - tail;
  }
  return ax_tcp_min_size(free_len, until_wrap);
}

static ssize_t ax_tcp_refill(ax_tcp_conn_handle* conn) {
  if (conn->read_len == 0) {
    conn->read_head = 0;
  }
  size_t space = ax_tcp_ring_contiguous_free(conn);
  if (space == 0) {
    return 0;
  }
  size_t tail = ax_tcp_ring_tail(conn);
  ssize_t n;
  for (;;) {
    n = read(conn->fd, conn->read_buffer + tail, space);
    if (n < 0 && errno == EINTR) {
      continue;
    }
    break;
  }
  if (n > 0) {
    conn->read_len += (size_t)n;
  }
  return n;
}

static size_t ax_tcp_find_lf(const ax_tcp_conn_handle* conn, size_t max_scan) {
  size_t scan = ax_tcp_min_size(conn->read_len, max_scan);
  if (scan == 0) {
    return (size_t)-1;
  }

  size_t first = ax_tcp_min_size(scan, AX_TCP_READ_CAP - conn->read_head);
  void* hit = memchr(conn->read_buffer + conn->read_head, '\n', first);
  if (hit != 0) {
    return (size_t)((char*)hit - (conn->read_buffer + conn->read_head));
  }

  size_t remaining = scan - first;
  if (remaining == 0) {
    return (size_t)-1;
  }
  hit = memchr(conn->read_buffer, '\n', remaining);
  if (hit != 0) {
    return first + (size_t)((char*)hit - conn->read_buffer);
  }
  return (size_t)-1;
}

static void ax_tcp_copy_from_ring(const ax_tcp_conn_handle* conn, char* out, size_t len) {
  size_t first = ax_tcp_min_size(len, AX_TCP_READ_CAP - conn->read_head);
  if (first > 0) {
    memcpy(out, conn->read_buffer + conn->read_head, first);
  }
  if (len > first) {
    memcpy(out + first, conn->read_buffer, len - first);
  }
}

static void ax_tcp_consume(ax_tcp_conn_handle* conn, size_t len) {
  size_t take = ax_tcp_min_size(len, conn->read_len);
  conn->read_head = (conn->read_head + take) & AX_TCP_READ_MASK;
  conn->read_len -= take;
}

static char ax_tcp_ring_peek(const ax_tcp_conn_handle* conn, size_t offset) {
  return conn->read_buffer[(conn->read_head + offset) & AX_TCP_READ_MASK];
}

static int ax_tcp_ring_line_equals(const ax_tcp_conn_handle* conn,
                                   size_t take,
                                   const char* word,
                                   size_t word_len) {
  if (take != word_len + 1 && take != word_len + 2) {
    return 0;
  }
  for (size_t i = 0; i < word_len; i++) {
    if (ax_tcp_ring_peek(conn, i) != word[i]) {
      return 0;
    }
  }
  if (take == word_len + 1) {
    return ax_tcp_ring_peek(conn, word_len) == '\n';
  }
  return ax_tcp_ring_peek(conn, word_len) == '\r' &&
         ax_tcp_ring_peek(conn, word_len + 1) == '\n';
}

static char* ax_tcp_static_inline_command(const ax_tcp_conn_handle* conn,
                                          size_t take,
                                          size_t* out_len) {
  size_t first = AX_TCP_READ_CAP - conn->read_head;
  if (take == 5 && first >= 5 && memcmp(conn->read_buffer + conn->read_head, "PING\n", 5) == 0) {
    *out_len = 4;
    return "PING";
  }
  if (take == 6 && first >= 6 &&
      memcmp(conn->read_buffer + conn->read_head, "PING\r\n", 6) == 0) {
    *out_len = 4;
    return "PING";
  }
  if (take == 6 && first >= 6 &&
      memcmp(conn->read_buffer + conn->read_head, "+PONG\n", 6) == 0) {
    *out_len = 5;
    return "+PONG";
  }
  if (take == 7 && first >= 7 &&
      memcmp(conn->read_buffer + conn->read_head, "+PONG\r\n", 7) == 0) {
    *out_len = 5;
    return "+PONG";
  }
  if (take == 4 && first >= 4 && memcmp(conn->read_buffer + conn->read_head, "+OK\n", 4) == 0) {
    *out_len = 3;
    return "+OK";
  }
  if (take == 5 && first >= 5 &&
      memcmp(conn->read_buffer + conn->read_head, "+OK\r\n", 5) == 0) {
    *out_len = 3;
    return "+OK";
  }
  if (ax_tcp_ring_line_equals(conn, take, "PING", 4)) {
    *out_len = 4;
    return "PING";
  }
  if (ax_tcp_ring_line_equals(conn, take, "QUIT", 4)) {
    *out_len = 4;
    return "QUIT";
  }
  if (ax_tcp_ring_line_equals(conn, take, "EXIT", 4)) {
    *out_len = 4;
    return "EXIT";
  }
  return 0;
}

static int ax_tcp_append_ring_bytes(char** buffer,
                                    size_t* len,
                                    size_t* cap,
                                    ax_tcp_conn_handle* conn,
                                    size_t take,
                                    size_t max_len) {
  size_t remaining = ax_tcp_min_size(take, conn->read_len);
  while (remaining > 0) {
    size_t chunk = ax_tcp_min_size(remaining, ax_tcp_ring_contiguous_used(conn));
    if (!ax_tcp_append_bytes(buffer, len, cap, conn->read_buffer + conn->read_head, chunk, max_len)) {
      return 0;
    }
    ax_tcp_consume(conn, chunk);
    remaining -= chunk;
  }
  return 1;
}

static int ax_tcp_read_exact_to_buffer(ax_tcp_conn_handle* conn,
                                       char** buffer,
                                       size_t* len,
                                       size_t* cap,
                                       size_t bytes_left,
                                       size_t max_len) {
  while (bytes_left > 0 && *len < max_len) {
    if (conn->read_len == 0) {
      if (ax_tcp_refill(conn) <= 0) {
        return 0;
      }
    }
    size_t limit = max_len - *len;
    size_t take = ax_tcp_min_size(bytes_left, ax_tcp_min_size(conn->read_len, limit));
    if (take == 0) {
      return 0;
    }
    if (!ax_tcp_append_ring_bytes(buffer, len, cap, conn, take, max_len)) {
      return 0;
    }
    bytes_left -= take;
  }
  return bytes_left == 0;
}

static char* ax_tcp_read_line_raw(ax_tcp_conn_handle* conn,
                                  int max_bytes,
                                  int include_newline,
                                  size_t* out_len,
                                  int* closed) {
  size_t max_len = max_bytes <= 0 ? 65536u : (size_t)max_bytes;
  char* out = 0;
  size_t len = 0;
  size_t cap = 0;
  *closed = 0;

  while (len < max_len) {
    if (conn->read_len == 0) {
      ax_tcp_flush_write(conn);
      if (ax_tcp_refill(conn) <= 0) {
        *closed = len == 0;
        break;
      }
    }

    size_t remaining_limit = max_len - len;
    size_t lf = ax_tcp_find_lf(conn, remaining_limit);
    size_t take = lf == (size_t)-1 ? ax_tcp_min_size(conn->read_len, remaining_limit) : lf + 1;

    if (len == 0 && take > 0 && (lf != (size_t)-1 || take == remaining_limit)) {
      if (include_newline && lf != (size_t)-1) {
        size_t static_len = 0;
        char* static_line = ax_tcp_static_inline_command(conn, take, &static_len);
        if (static_line != 0) {
          ax_tcp_consume(conn, take);
          len = static_len;
          out = static_line;
          break;
        }
      }
      out = (char*)malloc(take + 1);
      if (out == 0) {
        *closed = 1;
        return ax_tcp_empty_string();
      }
      ax_tcp_copy_from_ring(conn, out, take);
      ax_tcp_consume(conn, take);
      len = take;
      out[len] = 0;
      break;
    }

    if (take == 0 || !ax_tcp_append_ring_bytes(&out, &len, &cap, conn, take, max_len)) {
      *closed = len == 0;
      break;
    }
    if (lf != (size_t)-1) {
      break;
    }
  }

  if (out == 0) {
    out = ax_tcp_empty_string();
    len = 0;
  }
  if (!include_newline) {
    while (len > 0 && (out[len - 1] == '\n' || out[len - 1] == '\r')) {
      out[--len] = 0;
    }
  }
  if (out_len != 0) {
    *out_len = len;
  }
  return out;
}

static long ax_tcp_parse_resp_count(const char* line) {
  if (line == 0 || line[0] != '*') {
    return -1;
  }
  char* end = 0;
  long count = strtol(line + 1, &end, 10);
  return end == line + 1 || count < 0 ? -1 : count;
}

static long ax_tcp_parse_bulk_len(const char* line) {
  if (line == 0 || line[0] != '$') {
    return -1;
  }
  char* end = 0;
  long len = strtol(line + 1, &end, 10);
  return end == line + 1 || len < 0 ? -1 : len;
}

static int ax_create_listener_on(const char* host, int port) {
  char service[16];
  snprintf(service, sizeof(service), "%d", port);

  struct addrinfo hints;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_flags = AI_PASSIVE;

  const char* bind_host = host != 0 && host[0] != 0 ? host : 0;
  struct addrinfo* results = 0;
  int status = getaddrinfo(bind_host, service, &hints, &results);
  if (status != 0) {
    fprintf(stderr, "getaddrinfo: %s\n", gai_strerror(status));
    return -1;
  }

  int fd = -1;
  for (struct addrinfo* item = results; item != 0; item = item->ai_next) {
    fd = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
    if (fd < 0) {
      continue;
    }

    int yes = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));
#ifdef SO_REUSEPORT
    setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &yes, sizeof(yes));
#endif

    if (bind(fd, item->ai_addr, item->ai_addrlen) == 0 && listen(fd, 16384) == 0) {
      break;
    }
    close(fd);
    fd = -1;
  }
  freeaddrinfo(results);

  if (fd < 0) {
    perror("bind/listen");
  }
  return fd;
}

static int ax_create_listener(int port) {
  return ax_create_listener_on(0, port);
}

static char* ax_trim(char* value) {
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

static const ax_tcp_route* ax_find_route(
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

typedef struct {
  int client;
  const ax_tcp_route* routes;
  int route_count;
} ax_tcp_client;

static void ax_tcp_handle_client(int client, const ax_tcp_route* routes, int route_count) {
  char buffer[1024];
  ssize_t n = read(client, buffer, sizeof(buffer) - 1);
  if (n >= 0) {
    buffer[n] = 0;
    char* msg = ax_trim(buffer);
    const ax_tcp_route* route = ax_find_route(routes, route_count, msg);
    const char* response = route == 0 ? "" : route->response;
    write(client, response, strlen(response));
  }
  close(client);
}

static void* ax_tcp_client_thread(void* data) {
  ax_tcp_client* client = (ax_tcp_client*)data;
  ax_tcp_handle_client(client->client, client->routes, client->route_count);
  free(client);
  return 0;
}

int ax_tcp_server_start(int port, const ax_tcp_route* routes, int route_count) {
  signal(SIGPIPE, SIG_IGN);
  int server = ax_create_listener(port);
  if (server < 0) {
    return 1;
  }
  printf("ax tcp listen :%d\n", port);
  fflush(stdout);

  for (;;) {
    int client = accept(server, 0, 0);
    if (client < 0) {
      if (errno == EINTR) {
        continue;
      }
      perror("accept");
      close(server);
      return 1;
    }

    ax_tcp_client* ctx = (ax_tcp_client*)malloc(sizeof(ax_tcp_client));
    if (ctx != 0) {
      ctx->client = client;
      ctx->routes = routes;
      ctx->route_count = route_count;
      ax_tcp_configure_conn(client);
      pthread_t thread;
      if (pthread_create(&thread, 0, ax_tcp_client_thread, ctx) == 0) {
        pthread_detach(thread);
        continue;
      }
      free(ctx);
    }

    ax_tcp_handle_client(client, routes, route_count);
  }
}

int ax_tcp_ping_server(int port) {
  const ax_tcp_route routes[] = {
    { "ping", 0, "pong\n" },
    { "", 1, "unknown\n" },
  };
  return ax_tcp_server_start(port, routes, 2);
}

void* ax_tcp_listen(int port) {
  signal(SIGPIPE, SIG_IGN);
  int fd = ax_create_listener(port);
  if (fd < 0) {
    return 0;
  }
  ax_tcp_server_handle* server = (ax_tcp_server_handle*)malloc(sizeof(ax_tcp_server_handle));
  if (server == 0) {
    close(fd);
    return 0;
  }
  server->fd = fd;
  return server;
}

void* ax_tcp_listen_on(const char* host, int port) {
  signal(SIGPIPE, SIG_IGN);
  int fd = ax_create_listener_on(host, port);
  if (fd < 0) {
    return 0;
  }
  ax_tcp_server_handle* server = (ax_tcp_server_handle*)malloc(sizeof(ax_tcp_server_handle));
  if (server == 0) {
    close(fd);
    return 0;
  }
  server->fd = fd;
  return server;
}

void* ax_tcp_connect(const char* host, int port) {
  signal(SIGPIPE, SIG_IGN);
  if (host == 0) {
    return 0;
  }

  char service[16];
  snprintf(service, sizeof(service), "%d", port);

  struct addrinfo hints;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;

  struct addrinfo* results = 0;
  int status = getaddrinfo(host, service, &hints, &results);
  if (status != 0) {
    fprintf(stderr, "getaddrinfo: %s\n", gai_strerror(status));
    return 0;
  }

  int fd = -1;
  for (struct addrinfo* item = results; item != 0; item = item->ai_next) {
    fd = socket(item->ai_family, item->ai_socktype, item->ai_protocol);
    if (fd < 0) {
      continue;
    }
    if (connect(fd, item->ai_addr, item->ai_addrlen) == 0) {
      ax_tcp_configure_conn(fd);
      break;
    }
    close(fd);
    fd = -1;
  }
  freeaddrinfo(results);

  if (fd < 0) {
    perror("connect");
    return 0;
  }

  ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)malloc(sizeof(ax_tcp_conn_handle));
  if (conn == 0) {
    close(fd);
    return 0;
  }
  ax_tcp_conn_init(conn, fd);
  return conn;
}

typedef struct {
  ax_tcp_server_handle* server;
  void* state;
  ax_tcp_text_handler handler;
} ax_tcp_text_server_ctx;

typedef struct {
  ax_tcp_conn_handle** clients;
  struct pollfd* fds;
  size_t len;
  size_t cap;
} ax_tcp_event_loop;

static int ax_tcp_event_loop_reserve(ax_tcp_event_loop* loop, size_t needed) {
  if (needed <= loop->cap) {
    return 1;
  }
  size_t next = loop->cap == 0 ? 256u : loop->cap;
  while (next < needed) {
    next <<= 1;
  }
  ax_tcp_conn_handle** clients =
    (ax_tcp_conn_handle**)realloc(loop->clients, next * sizeof(ax_tcp_conn_handle*));
  if (clients == 0) {
    return 0;
  }
  struct pollfd* fds = (struct pollfd*)realloc(loop->fds, (next + 1u) * sizeof(struct pollfd));
  if (fds == 0) {
    return 0;
  }
  loop->clients = clients;
  loop->fds = fds;
  loop->cap = next;
  return 1;
}

static int ax_tcp_event_add_client(ax_tcp_event_loop* loop, int fd) {
  if (!ax_tcp_event_loop_reserve(loop, loop->len + 1u)) {
    close(fd);
    return 0;
  }
  ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)malloc(sizeof(ax_tcp_conn_handle));
  if (conn == 0) {
    close(fd);
    return 0;
  }
  ax_tcp_configure_conn(fd);
  ax_tcp_set_nonblocking(fd);
  ax_tcp_conn_init(conn, fd);
  loop->clients[loop->len++] = conn;
  return 1;
}

static void ax_tcp_event_close_at(ax_tcp_event_loop* loop, size_t index) {
  if (index >= loop->len) {
    return;
  }
  ax_tcp_conn_handle* conn = loop->clients[index];
  if (conn != 0) {
    if (conn->fd >= 0) {
      close(conn->fd);
    }
    free(conn);
  }
  loop->len--;
  if (index != loop->len) {
    loop->clients[index] = loop->clients[loop->len];
  }
}

static int ax_tcp_event_flush(ax_tcp_conn_handle* conn) {
  while (conn->write_len > 0) {
    ssize_t n = write(conn->fd, conn->write_buffer, conn->write_len);
    if (n < 0 && errno == EINTR) {
      continue;
    }
    if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      return 1;
    }
    if (n <= 0) {
      return 0;
    }
    size_t sent = (size_t)n;
    if (sent >= conn->write_len) {
      conn->write_len = 0;
      return 1;
    }
    memmove(conn->write_buffer, conn->write_buffer + sent, conn->write_len - sent);
    conn->write_len -= sent;
  }
  return 1;
}

static int ax_tcp_event_queue_write(ax_tcp_conn_handle* conn, const char* text) {
  if (text == 0) {
    return 1;
  }
  const char* cursor = text;
  size_t remaining = strlen(text);
  while (remaining > 0) {
    size_t space = AX_TCP_WRITE_CAP - conn->write_len;
    if (space == 0) {
      if (!ax_tcp_event_flush(conn)) {
        return 0;
      }
      space = AX_TCP_WRITE_CAP - conn->write_len;
      if (space == 0) {
        return 1;
      }
    }
    size_t take = remaining < space ? remaining : space;
    memcpy(conn->write_buffer + conn->write_len, cursor, take);
    conn->write_len += take;
    cursor += take;
    remaining -= take;
  }
  return ax_tcp_event_flush(conn);
}

static int ax_tcp_event_should_close(const char* line) {
  if (line == 0) {
    return 0;
  }
  while (*line == ' ' || *line == '\t') {
    line++;
  }
  char token[5];
  size_t len = 0;
  while (len < sizeof(token) - 1u && line[len] != 0 && line[len] != ' ' && line[len] != '\t') {
    char ch = line[len];
    token[len] = (char)(ch >= 'a' && ch <= 'z' ? ch - ('a' - 'A') : ch);
    len++;
  }
  token[len] = 0;
  return strcmp(token, "QUIT") == 0 || strcmp(token, "EXIT") == 0;
}

static char* ax_tcp_event_take_line(ax_tcp_conn_handle* conn) {
  size_t lf = ax_tcp_find_lf(conn, conn->read_len);
  if (lf == (size_t)-1) {
    return 0;
  }
  size_t take = lf + 1u;
  size_t static_len = 0;
  char* static_line = ax_tcp_static_inline_command(conn, take, &static_len);
  if (static_line != 0) {
    ax_tcp_consume(conn, take);
    return static_line;
  }

  size_t line_len = take;
  if (line_len > 0 && ax_tcp_ring_peek(conn, line_len - 1u) == '\n') {
    line_len--;
  }
  if (line_len > 0 && ax_tcp_ring_peek(conn, line_len - 1u) == '\r') {
    line_len--;
  }
  char* out = (char*)malloc(line_len + 1u);
  if (out == 0) {
    ax_tcp_consume(conn, take);
    return ax_tcp_empty_string();
  }
  ax_tcp_copy_from_ring(conn, out, line_len);
  out[line_len] = 0;
  ax_tcp_consume(conn, take);
  return out;
}

static int ax_tcp_event_drain_lines(ax_tcp_conn_handle* conn,
                                    void* state,
                                    ax_tcp_text_handler handler) {
  for (;;) {
    char* line = ax_tcp_event_take_line(conn);
    if (line == 0) {
      break;
    }
    int close_after = ax_tcp_event_should_close(line);
    char* response = handler == 0 ? ax_tcp_empty_string() : handler(state, line);
    if (!ax_tcp_event_queue_write(conn, response)) {
      return 0;
    }
    if (close_after) {
      conn->close_after_write = 1;
      break;
    }
  }
  return 1;
}

static int ax_tcp_event_read_ready(ax_tcp_conn_handle* conn,
                                   void* state,
                                   ax_tcp_text_handler handler) {
  for (;;) {
    ssize_t n = ax_tcp_refill(conn);
    if (n > 0) {
      if (!ax_tcp_event_drain_lines(conn, state, handler) || conn->close_after_write) {
        return 1;
      }
      continue;
    }
    if (n == 0) {
      return 0;
    }
    if (errno == EAGAIN || errno == EWOULDBLOCK) {
      break;
    }
    return 0;
  }
  return ax_tcp_event_drain_lines(conn, state, handler);
}

static void ax_tcp_event_accept_ready(ax_tcp_event_loop* loop, int server_fd) {
  for (;;) {
    int client = accept(server_fd, 0, 0);
    if (client >= 0) {
      ax_tcp_event_add_client(loop, client);
      continue;
    }
    if (errno == EINTR) {
      continue;
    }
    if (errno == EAGAIN || errno == EWOULDBLOCK) {
      return;
    }
    return;
  }
}

static void* ax_tcp_serve_text_worker(void* data) {
  ax_tcp_text_server_ctx* ctx = (ax_tcp_text_server_ctx*)data;
  ax_tcp_event_loop loop;
  memset(&loop, 0, sizeof(loop));
  for (;;) {
    ax_tcp_event_loop_reserve(&loop, loop.len + 1u);
    loop.fds[0].fd = ctx->server->fd;
    loop.fds[0].events = POLLIN;
    loop.fds[0].revents = 0;
    for (size_t i = 0; i < loop.len; i++) {
      ax_tcp_conn_handle* conn = loop.clients[i];
      loop.fds[i + 1u].fd = conn->fd;
      loop.fds[i + 1u].events = POLLIN | (conn->write_len > 0 ? POLLOUT : 0);
      loop.fds[i + 1u].revents = 0;
      conn->event_revents = 0;
    }

    int ready = poll(loop.fds, (nfds_t)(loop.len + 1u), 1000);
    if (ready < 0 && errno == EINTR) {
      continue;
    }
    if (ready < 0) {
      continue;
    }
    if ((loop.fds[0].revents & POLLIN) != 0) {
      ax_tcp_event_accept_ready(&loop, ctx->server->fd);
    }
    for (size_t i = 0; i < loop.len; i++) {
      loop.clients[i]->event_revents = loop.fds[i + 1u].revents;
    }

    size_t i = 0;
    while (i < loop.len) {
      ax_tcp_conn_handle* conn = loop.clients[i];
      short revents = conn->event_revents;
      int keep = 1;
      if ((revents & (POLLERR | POLLNVAL)) != 0) {
        keep = 0;
      }
      if (keep && (revents & POLLIN) != 0) {
        keep = ax_tcp_event_read_ready(conn, ctx->state, ctx->handler);
      }
      if (keep && (revents & POLLHUP) != 0 && (revents & POLLIN) == 0) {
        ssize_t n = ax_tcp_refill(conn);
        if (n == 0) {
          keep = 0;
        } else if (n > 0) {
          keep = ax_tcp_event_drain_lines(conn, ctx->state, ctx->handler);
        } else if (!(errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) {
          keep = 0;
        }
      }
      if (keep && (revents & POLLOUT) != 0) {
        keep = ax_tcp_event_flush(conn);
      }
      if (keep && conn->close_after_write && conn->write_len == 0) {
        keep = 0;
      }
      if (!keep) {
        ax_tcp_event_close_at(&loop, i);
        continue;
      }
      i++;
    }
  }
  return 0;
}

int ax_tcp_serve_text(void* server_ptr, void* state, ax_tcp_text_handler handler, int workers) {
  ax_tcp_server_handle* server = (ax_tcp_server_handle*)server_ptr;
  if (server == 0 || server->fd < 0 || handler == 0) {
    return 1;
  }
  if (workers < 1) {
    workers = 1;
  }
  if (workers > 64) {
    workers = 64;
  }
  ax_tcp_set_nonblocking(server->fd);

  for (int i = 1; i < workers; i++) {
    ax_tcp_text_server_ctx* worker =
      (ax_tcp_text_server_ctx*)malloc(sizeof(ax_tcp_text_server_ctx));
    if (worker == 0) {
      continue;
    }
    worker->server = server;
    worker->state = state;
    worker->handler = handler;
    pthread_t thread;
    if (pthread_create(&thread, 0, ax_tcp_serve_text_worker, worker) == 0) {
      pthread_detach(thread);
    } else {
      free(worker);
    }
  }

  ax_tcp_text_server_ctx main_ctx;
  main_ctx.server = server;
  main_ctx.state = state;
  main_ctx.handler = handler;
  ax_tcp_serve_text_worker(&main_ctx);
  return 0;
}

void* ax_tcp_accept(void* server_ptr) {
  ax_tcp_server_handle* server = (ax_tcp_server_handle*)server_ptr;
  if (server == 0 || server->fd < 0) {
    return 0;
  }
  for (;;) {
    int client = accept(server->fd, 0, 0);
    if (client >= 0) {
      ax_tcp_configure_conn(client);
      ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)malloc(sizeof(ax_tcp_conn_handle));
      if (conn == 0) {
        close(client);
        return 0;
      }
      ax_tcp_conn_init(conn, client);
      return conn;
    }
    if (errno == EINTR) {
      continue;
    }
    perror("accept");
    return 0;
  }
}

static char* ax_tcp_read_protocol_text(void* conn_ptr, int max_bytes) {
  ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)conn_ptr;
  if (conn == 0 || conn->fd < 0 || max_bytes <= 0) {
    return ax_tcp_empty_string();
  }

  size_t line_len = 0;
  int closed = 0;
  char* line = ax_tcp_read_line_raw(conn, max_bytes, 1, &line_len, &closed);
  if (closed && line_len == 0) {
    return line;
  }

  long array_count = ax_tcp_parse_resp_count(line);
  long bulk_len = ax_tcp_parse_bulk_len(line);
  if (bulk_len >= 0) {
    size_t max_len = (size_t)max_bytes;
    char* frame = 0;
    size_t frame_len = 0;
    size_t frame_cap = 0;
    if (!ax_tcp_append_bytes(&frame, &frame_len, &frame_cap, line, line_len, max_len)) {
      free(line);
      return ax_tcp_empty_string();
    }
    free(line);

    ax_tcp_read_exact_to_buffer(conn, &frame, &frame_len, &frame_cap, (size_t)bulk_len + 2, max_len);
    return frame == 0 ? ax_tcp_empty_string() : frame;
  }

  if (array_count < 0) {
    while (line_len > 0 && (line[line_len - 1] == '\n' || line[line_len - 1] == '\r')) {
      line[--line_len] = 0;
    }
    return line;
  }

  size_t max_len = (size_t)max_bytes;
  char* frame = 0;
  size_t frame_len = 0;
  size_t frame_cap = 0;
  if (!ax_tcp_append_bytes(&frame, &frame_len, &frame_cap, line, line_len, max_len)) {
    free(line);
    return ax_tcp_empty_string();
  }
  free(line);

  for (long i = 0; i < array_count && frame_len < max_len; i++) {
    size_t header_len = 0;
    int header_closed = 0;
    char* header = ax_tcp_read_line_raw(
      conn,
      (int)(max_len - frame_len),
      1,
      &header_len,
      &header_closed
    );
    if (header_closed && header_len == 0) {
      free(header);
      break;
    }
    if (!ax_tcp_append_bytes(&frame, &frame_len, &frame_cap, header, header_len, max_len)) {
      free(header);
      break;
    }
    long bulk_len = ax_tcp_parse_bulk_len(header);
    free(header);
    if (bulk_len < 0) {
      continue;
    }
    ax_tcp_read_exact_to_buffer(conn, &frame, &frame_len, &frame_cap, (size_t)bulk_len + 2, max_len);
  }

  if (frame == 0) {
    return ax_tcp_empty_string();
  }
  return frame;
}

char* ax_tcp_read_text(void* conn_ptr, int max_bytes) {
  return ax_tcp_read_protocol_text(conn_ptr, max_bytes);
}

void ax_tcp_write_text(void* conn_ptr, const char* text) {
  ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)conn_ptr;
  if (conn == 0 || conn->fd < 0 || text == 0) {
    return;
  }
  const char* cursor = text;
  size_t remaining = strlen(text);
  if (conn->read_len == 0 && conn->write_len == 0) {
    ax_tcp_write_all_fd(conn->fd, cursor, remaining);
    return;
  }
  while (remaining > 0) {
    size_t space = AX_TCP_WRITE_CAP - conn->write_len;
    if (space == 0) {
      ax_tcp_flush_write(conn);
      space = AX_TCP_WRITE_CAP - conn->write_len;
      if (space == 0) {
        return;
      }
    }
    size_t take = remaining < space ? remaining : space;
    memcpy(conn->write_buffer + conn->write_len, cursor, take);
    conn->write_len += take;
    cursor += take;
    remaining -= take;
    if (conn->write_len == AX_TCP_WRITE_CAP) {
      ax_tcp_flush_write(conn);
    }
  }
  if (conn->read_len == 0) {
    ax_tcp_flush_write(conn);
  }
}

char* ax_tcp_request_text(void* conn_ptr, const char* text, int max_bytes) {
  ax_tcp_write_text(conn_ptr, text);
  return ax_tcp_read_protocol_text(conn_ptr, max_bytes);
}

void ax_tcp_close(void* conn_ptr) {
  ax_tcp_conn_handle* conn = (ax_tcp_conn_handle*)conn_ptr;
  if (conn == 0) {
    return;
  }
  if (conn->fd >= 0) {
    ax_tcp_flush_write(conn);
    close(conn->fd);
    conn->fd = -1;
  }
  free(conn);
}

#endif
