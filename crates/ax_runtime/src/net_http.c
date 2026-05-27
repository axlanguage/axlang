#include "ax_runtime.h"

#ifdef _WIN32

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char* ax_http_strdup(const char* value) {
  const char* source = value == 0 ? "" : value;
  size_t len = strlen(source);
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return 0;
  }
  memcpy(out, source, len + 1);
  return out;
}

int ax_http_server_start(int port, const ax_http_route* routes, int route_count) {
  (void)routes;
  (void)route_count;
  fprintf(stderr, "ax http listen :%d is not supported by the Windows runtime yet\n", port);
  return 1;
}

char* ax_http_get(const char* url) {
  (void)url;
  return ax_http_strdup("");
}

char* ax_http_post(const char* url, const char* body) {
  (void)url;
  (void)body;
  return ax_http_strdup("");
}

char* ax_http_get_json(const char* url, int max_bytes) {
  (void)url;
  (void)max_bytes;
  return ax_http_strdup("{\"ok\":false,\"status\":0,\"body\":\"\",\"truncated\":false}");
}

char* ax_http_post_json(const char* url, const char* body, int max_bytes) {
  (void)url;
  (void)body;
  (void)max_bytes;
  return ax_http_strdup("{\"ok\":false,\"status\":0,\"body\":\"\",\"truncated\":false}");
}

#else

#include <arpa/inet.h>
#include <errno.h>
#include <netdb.h>
#include <netinet/in.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <unistd.h>

#define AX_HTTP_BUFFER_SIZE 16384
#define AX_HTTP_RESPONSE_CACHE_CAP 64

static void ax_send_all(int fd, const char* data, size_t len);
static int ax_http_response_cache_enabled = 1;

typedef struct {
  const ax_http_route* route;
  char* response;
  size_t len;
} ax_http_response_cache_entry;

static ax_http_response_cache_entry ax_http_response_cache[AX_HTTP_RESPONSE_CACHE_CAP];
static int ax_http_response_cache_count = 0;

static char* ax_http_strdup(const char* value) {
  const char* source = value == 0 ? "" : value;
  size_t len = strlen(source);
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return 0;
  }
  memcpy(out, source, len + 1);
  return out;
}

static char* ax_http_slice_copy(const char* value, size_t len) {
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return ax_http_strdup("");
  }
  memcpy(out, value, len);
  out[len] = 0;
  return out;
}

static int ax_http_append_bytes(char** buffer, size_t* len, size_t* cap, const char* value, size_t value_len) {
  size_t needed = *len + value_len + 1;
  if (needed > *cap) {
    size_t next = *cap == 0 ? 8192 : *cap;
    while (next < needed) {
      next *= 2;
    }
    char* resized = (char*)realloc(*buffer, next);
    if (resized == 0) {
      free(*buffer);
      *buffer = 0;
      *len = 0;
      *cap = 0;
      return 0;
    }
    *buffer = resized;
    *cap = next;
  }
  memcpy(*buffer + *len, value, value_len);
  *len += value_len;
  (*buffer)[*len] = 0;
  return 1;
}

static int ax_http_parse_url(const char* url, char* host, size_t host_cap, int* port, char* path, size_t path_cap) {
  if (url == 0 || strncmp(url, "http://", 7) != 0) {
    return 0;
  }
  const char* cursor = url + 7;
  const char* slash = strchr(cursor, '/');
  const char* authority_end = slash == 0 ? cursor + strlen(cursor) : slash;
  const char* colon = 0;
  for (const char* p = cursor; p < authority_end; p++) {
    if (*p == ':') {
      colon = p;
      break;
    }
  }

  size_t host_len = (size_t)((colon == 0 ? authority_end : colon) - cursor);
  if (host_len == 0 || host_len >= host_cap) {
    return 0;
  }
  memcpy(host, cursor, host_len);
  host[host_len] = 0;
  *port = 80;
  if (colon != 0) {
    *port = atoi(colon + 1);
    if (*port <= 0 || *port > 65535) {
      return 0;
    }
  }
  const char* request_path = slash == 0 ? "/" : slash;
  if (strlen(request_path) >= path_cap) {
    return 0;
  }
  strcpy(path, request_path);
  return 1;
}

static int ax_http_connect(const char* host, int port) {
  char port_text[16];
  snprintf(port_text, sizeof(port_text), "%d", port);
  struct addrinfo hints;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;

  struct addrinfo* result = 0;
  if (getaddrinfo(host, port_text, &hints, &result) != 0) {
    return -1;
  }
  int fd = -1;
  for (struct addrinfo* ai = result; ai != 0; ai = ai->ai_next) {
    fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
    if (fd < 0) {
      continue;
    }
    if (connect(fd, ai->ai_addr, ai->ai_addrlen) == 0) {
      break;
    }
    close(fd);
    fd = -1;
  }
  freeaddrinfo(result);
  return fd;
}

static char* ax_http_response_body(char* response, size_t len) {
  if (response == 0) {
    return ax_http_strdup("");
  }
  for (size_t i = 0; i + 3 < len; i++) {
    if (response[i] == '\r' && response[i + 1] == '\n' && response[i + 2] == '\r' && response[i + 3] == '\n') {
      return ax_http_slice_copy(response + i + 4, len - i - 4);
    }
  }
  return ax_http_strdup("");
}

static int ax_http_response_status(const char* response, size_t len) {
  if (response == 0 || len < 12 || strncmp(response, "HTTP/", 5) != 0) {
    return 0;
  }
  const char* space = memchr(response, ' ', len);
  if (space == 0 || space + 1 >= response + len) {
    return 0;
  }
  return atoi(space + 1);
}

static char* ax_http_response_body_limited(char* response, size_t len, int max_body_bytes, int* truncated) {
  if (truncated != 0) {
    *truncated = 0;
  }
  if (response == 0) {
    return ax_http_strdup("");
  }
  for (size_t i = 0; i + 3 < len; i++) {
    if (response[i] == '\r' && response[i + 1] == '\n' && response[i + 2] == '\r' && response[i + 3] == '\n') {
      const char* body = response + i + 4;
      size_t body_len = len - i - 4;
      size_t copy_len = body_len;
      if (max_body_bytes >= 0 && (size_t)max_body_bytes < copy_len) {
        copy_len = (size_t)max_body_bytes;
        if (truncated != 0) {
          *truncated = 1;
        }
      }
      return ax_http_slice_copy(body, copy_len);
    }
  }
  return ax_http_strdup("");
}

static char* ax_http_request_capture(
  const char* method,
  const char* url,
  const char* body,
  int max_body_bytes,
  int* out_status,
  int* out_truncated
) {
  if (out_status != 0) {
    *out_status = 0;
  }
  if (out_truncated != 0) {
    *out_truncated = 0;
  }
  char host[256];
  char path[1024];
  int port = 80;
  if (!ax_http_parse_url(url, host, sizeof(host), &port, path, sizeof(path))) {
    return ax_http_strdup("");
  }
  int fd = ax_http_connect(host, port);
  if (fd < 0) {
    return ax_http_strdup("");
  }

  const char* payload = body == 0 ? "" : body;
  char header[2048];
  int header_len = 0;
  if (strcmp(method, "POST") == 0) {
    header_len = snprintf(
      header,
      sizeof(header),
      "POST %s HTTP/1.1\r\nHost: %s\r\nUser-Agent: Ax/1.0\r\nContent-Type: text/plain\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
      path,
      host,
      strlen(payload)
    );
  } else {
    header_len = snprintf(
      header,
      sizeof(header),
      "GET %s HTTP/1.1\r\nHost: %s\r\nUser-Agent: Ax/1.0\r\nConnection: close\r\n\r\n",
      path,
      host
    );
  }
  if (header_len <= 0 || header_len >= (int)sizeof(header)) {
    close(fd);
    return ax_http_strdup("");
  }
  ax_send_all(fd, header, (size_t)header_len);
  if (strcmp(method, "POST") == 0 && *payload != 0) {
    ax_send_all(fd, payload, strlen(payload));
  }

  char* response = 0;
  size_t response_len = 0;
  size_t response_cap = 0;
  char chunk[4096];
  for (;;) {
    ssize_t n = read(fd, chunk, sizeof(chunk));
    if (n <= 0) {
      break;
    }
    if (!ax_http_append_bytes(&response, &response_len, &response_cap, chunk, (size_t)n)) {
      close(fd);
      return ax_http_strdup("");
    }
  }
  close(fd);
  if (out_status != 0) {
    *out_status = ax_http_response_status(response, response_len);
  }
  char* result = max_body_bytes < 0
    ? ax_http_response_body(response, response_len)
    : ax_http_response_body_limited(response, response_len, max_body_bytes, out_truncated);
  free(response);
  return result;
}

static char* ax_http_request(const char* method, const char* url, const char* body) {
  return ax_http_request_capture(method, url, body, -1, 0, 0);
}

static char* ax_http_result_json(int status, const char* body, int truncated) {
  char status_json[32];
  snprintf(status_json, sizeof(status_json), "%d", status);

  char* ok_pair = ax_json_pair("ok", status >= 200 && status < 300 ? "true" : "false");
  char* status_pair = ax_json_pair("status", status_json);
  char* body_pair = ax_json_string_pair("body", body);
  char* truncated_pair = ax_json_pair("truncated", truncated ? "true" : "false");
  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  ax_http_append_bytes(&fields, &len, &cap, ok_pair, strlen(ok_pair));
  ax_http_append_bytes(&fields, &len, &cap, ",", 1);
  ax_http_append_bytes(&fields, &len, &cap, status_pair, strlen(status_pair));
  ax_http_append_bytes(&fields, &len, &cap, ",", 1);
  ax_http_append_bytes(&fields, &len, &cap, body_pair, strlen(body_pair));
  ax_http_append_bytes(&fields, &len, &cap, ",", 1);
  ax_http_append_bytes(&fields, &len, &cap, truncated_pair, strlen(truncated_pair));

  char* result = fields == 0 ? ax_http_strdup("{}") : ax_json_object(fields);
  free(ok_pair);
  free(status_pair);
  free(body_pair);
  free(truncated_pair);
  free(fields);
  return result;
}

char* ax_http_get(const char* url) {
  return ax_http_request("GET", url, "");
}

char* ax_http_post(const char* url, const char* body) {
  return ax_http_request("POST", url, body);
}

char* ax_http_get_json(const char* url, int max_bytes) {
  if (max_bytes < 0) {
    max_bytes = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  int status = 0;
  int truncated = 0;
  char* body = ax_http_request_capture("GET", url, "", max_bytes, &status, &truncated);
  char* result = ax_http_result_json(status, body, truncated);
  free(body);
  return result;
}

char* ax_http_post_json(const char* url, const char* body, int max_bytes) {
  if (max_bytes < 0) {
    max_bytes = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  int status = 0;
  int truncated = 0;
  char* response_body = ax_http_request_capture("POST", url, body, max_bytes, &status, &truncated);
  char* result = ax_http_result_json(status, response_body, truncated);
  free(response_body);
  return result;
}

static int ax_create_listener(int port) {
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

static void ax_send_all(int fd, const char* data, size_t len) {
  while (len > 0) {
    ssize_t n = send(fd, data, len, 0);
    if (n <= 0) {
      return;
    }
    data += n;
    len -= (size_t)n;
  }
}

static void ax_send_response(
  int fd,
  int status,
  const char* status_text,
  const char* content_type,
  const char* body,
  int keep_alive
) {
  size_t body_len = strlen(body);
  char header[512];
  int header_len = snprintf(
    header,
    sizeof(header),
    "HTTP/1.1 %d %s\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: %s\r\n\r\n",
    status,
    status_text,
    content_type,
    body_len,
    keep_alive ? "keep-alive" : "close"
  );
  if (header_len <= 0) {
    return;
  }
  char response[1024];
  if ((size_t)header_len + body_len <= sizeof(response)) {
    memcpy(response, header, (size_t)header_len);
    memcpy(response + header_len, body, body_len);
    ax_send_all(fd, response, (size_t)header_len + body_len);
    return;
  }
  ax_send_all(fd, header, (size_t)header_len);
  ax_send_all(fd, body, body_len);
}

static const char* ax_route_content_type(const ax_http_route* route) {
  return route->response_kind == 1 ? "application/json" : "text/plain";
}

static void ax_send_cached_route_response(int fd, const ax_http_route* route) {
  const char* content_type = ax_route_content_type(route);
  if (!ax_http_response_cache_enabled) {
    ax_send_response(fd, 200, "OK", content_type, route->body, 0);
    return;
  }
  for (int i = 0; i < ax_http_response_cache_count; i++) {
    if (ax_http_response_cache[i].route == route) {
      ax_send_all(fd, ax_http_response_cache[i].response, ax_http_response_cache[i].len);
      return;
    }
  }
  if (ax_http_response_cache_count >= AX_HTTP_RESPONSE_CACHE_CAP) {
    ax_send_response(fd, 200, "OK", content_type, route->body, 0);
    return;
  }

  const char* body = route->body == 0 ? "" : route->body;
  size_t body_len = strlen(body);
  char header[512];
  int header_len = snprintf(
    header,
    sizeof(header),
    "HTTP/1.1 200 OK\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
    content_type,
    body_len
  );
  if (header_len <= 0) {
    return;
  }
  size_t response_len = (size_t)header_len + body_len;
  char* response = (char*)malloc(response_len);
  if (response == 0) {
    ax_send_response(fd, 200, "OK", content_type, route->body, 0);
    return;
  }
  memcpy(response, header, (size_t)header_len);
  memcpy(response + header_len, body, body_len);
  int slot = ax_http_response_cache_count++;
  ax_http_response_cache[slot].route = route;
  ax_http_response_cache[slot].response = response;
  ax_http_response_cache[slot].len = response_len;
  ax_send_all(fd, response, response_len);
}

static void ax_send_stream_response_header(int fd, size_t content_length) {
  char header[256];
  int header_len = snprintf(
    header,
    sizeof(header),
    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n",
    content_length
  );
  if (header_len > 0) {
    ax_send_all(fd, header, (size_t)header_len);
  }
}

static void ax_send_chunked_stream_response_header(int fd) {
  const char* header =
    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
  ax_send_all(fd, header, strlen(header));
}

static void ax_stream_content_length_body(
  int fd,
  const char* initial,
  size_t initial_len,
  size_t content_length
) {
  ax_send_stream_response_header(fd, content_length);
  size_t first = initial_len < content_length ? initial_len : content_length;
  if (first > 0) {
    ax_send_all(fd, initial, first);
  }
  size_t remaining = content_length - first;
  char chunk[4096];
  while (remaining > 0) {
    size_t want = remaining < sizeof(chunk) ? remaining : sizeof(chunk);
    ssize_t n = read(fd, chunk, want);
    if (n <= 0) {
      return;
    }
    ax_send_all(fd, chunk, (size_t)n);
    remaining -= (size_t)n;
  }
}

static char* ax_find_header_end(char* buffer, size_t len) {
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

static int ax_read_more(int fd, char* buffer, size_t* len) {
  if (*len >= AX_HTTP_BUFFER_SIZE) {
    return -2;
  }
  ssize_t n = read(fd, buffer + *len, AX_HTTP_BUFFER_SIZE - *len);
  if (n <= 0) {
    return (int)n;
  }
  *len += (size_t)n;
  return 1;
}

static size_t ax_content_length(const char* headers) {
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

static int ax_parse_chunk_size(const char* start, const char* end, size_t* out_size) {
  size_t value = 0;
  const char* cursor = start;
  while (cursor < end && *cursor != ';') {
    char ch = *cursor;
    int digit = -1;
    if (ch >= '0' && ch <= '9') {
      digit = ch - '0';
    } else if (ch >= 'a' && ch <= 'f') {
      digit = ch - 'a' + 10;
    } else if (ch >= 'A' && ch <= 'F') {
      digit = ch - 'A' + 10;
    } else if (ch == ' ' || ch == '\t') {
      cursor++;
      continue;
    } else {
      return 0;
    }
    value = (value * 16) + (size_t)digit;
    cursor++;
  }
  *out_size = value;
  return 1;
}

static int ax_find_chunked_end(const char* buffer, size_t len, size_t body_start, size_t* out_end) {
  size_t cursor = body_start;
  for (;;) {
    size_t line_start = cursor;
    const char* line_end = 0;
    while (cursor + 1 < len) {
      if (buffer[cursor] == '\r' && buffer[cursor + 1] == '\n') {
        line_end = buffer + cursor;
        break;
      }
      cursor++;
    }
    if (line_end == 0) {
      return 0;
    }

    size_t chunk_size = 0;
    if (!ax_parse_chunk_size(buffer + line_start, line_end, &chunk_size)) {
      return -1;
    }
    cursor += 2;

    if (chunk_size == 0) {
      for (;;) {
        if (cursor + 1 >= len) {
          return 0;
        }
        if (buffer[cursor] == '\r' && buffer[cursor + 1] == '\n') {
          *out_end = cursor + 2;
          return 1;
        }
        while (cursor + 1 < len && !(buffer[cursor] == '\r' && buffer[cursor + 1] == '\n')) {
          cursor++;
        }
        if (cursor + 1 >= len) {
          return 0;
        }
        cursor += 2;
      }
    }

    if (cursor + chunk_size + 2 > len) {
      return 0;
    }
    cursor += chunk_size;
    if (buffer[cursor] != '\r' || buffer[cursor + 1] != '\n') {
      return -1;
    }
    cursor += 2;
  }
}

static int ax_header_contains_token(const char* headers, const char* header_name, const char* token) {
  size_t name_len = strlen(header_name);
  const char* cursor = headers;
  while (*cursor != 0) {
    const char* line_end = strstr(cursor, "\r\n");
    size_t line_len = line_end == 0 ? strlen(cursor) : (size_t)(line_end - cursor);
    if (line_len > name_len && strncasecmp(cursor, header_name, name_len) == 0 && cursor[name_len] == ':') {
      const char* value = cursor + name_len + 1;
      size_t value_len = line_len - name_len - 1;
      for (size_t i = 0; i + strlen(token) <= value_len; i++) {
        if (strncasecmp(value + i, token, strlen(token)) == 0) {
          return 1;
        }
      }
    }
    if (line_end == 0) {
      break;
    }
    cursor = line_end + 2;
  }
  return 0;
}

static int ax_should_keep_alive(const char* headers, const char* version) {
  if (ax_header_contains_token(headers, "Connection", "close")) {
    return 0;
  }
  if (strcmp(version, "HTTP/1.1") == 0) {
    return 1;
  }
  return ax_header_contains_token(headers, "Connection", "keep-alive");
}

static int ax_drain_bytes(int fd, size_t bytes) {
  char discard[1024];
  while (bytes > 0) {
    size_t want = bytes < sizeof(discard) ? bytes : sizeof(discard);
    ssize_t n = read(fd, discard, want);
    if (n <= 0) {
      return 0;
    }
    bytes -= (size_t)n;
  }
  return 1;
}

static void ax_strip_query(char* path) {
  char* query = strchr(path, '?');
  if (query != 0) {
    *query = 0;
  }
}

static const ax_http_route* ax_find_route(
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

static const ax_http_route* ax_find_get_route_fast(
  const ax_http_route* routes,
  int route_count,
  const char* path
) {
  for (int i = 0; i < route_count; i++) {
    if (routes[i].method[0] == 'G' && strcmp(routes[i].method, "GET") == 0 && strcmp(routes[i].path, path) == 0) {
      return &routes[i];
    }
  }
  return 0;
}

static int ax_path_exists(
  const ax_http_route* routes,
  int route_count,
  const char* path
) {
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

typedef struct {
  int client;
  const ax_http_route* routes;
  int route_count;
} ax_http_client;

static void ax_http_handle_client(int client, const ax_http_route* routes, int route_count) {
  char buffer[AX_HTTP_BUFFER_SIZE + 1];
  size_t buffered = 0;
  int close_client = 0;

  while (!close_client) {
    char* header_end = ax_find_header_end(buffer, buffered);
    while (header_end == 0) {
      int read_result = ax_read_more(client, buffer, &buffered);
      if (read_result <= 0) {
        close_client = 1;
        break;
      }
      header_end = ax_find_header_end(buffer, buffered);
    }
    if (close_client) {
      break;
    }

    size_t header_len = (size_t)(header_end - buffer);
    buffer[buffered] = 0;

    if (strncmp(buffer, "GET ", 4) == 0) {
      char* path_start = buffer + 4;
      char* path_end = strchr(path_start, ' ');
      if (path_end != 0 && path_end < header_end) {
        char saved = *path_end;
        *path_end = 0;
        if (strchr(path_start, '?') != 0) {
          ax_strip_query(path_start);
        }
        const ax_http_route* route = ax_find_get_route_fast(routes, route_count, path_start);
        if (route != 0 && route->response_kind != 2) {
          ax_send_cached_route_response(client, route);
          *path_end = saved;
          break;
        }
        if (route == 0) {
          if (ax_path_exists(routes, route_count, path_start)) {
            ax_send_response(client, 405, "Method Not Allowed", "text/plain", "Method Not Allowed", 0);
          } else {
            ax_send_response(client, 404, "Not Found", "text/plain", "Not Found", 0);
          }
          *path_end = saved;
          break;
        }
        *path_end = saved;
      }
    }

    char headers[AX_HTTP_BUFFER_SIZE + 1];
    memcpy(headers, buffer, header_len);
    headers[header_len] = 0;

    char method[16] = {0};
    char path[512] = {0};
    char version[16] = {0};
    int keep_alive = 0;
    if (sscanf(headers, "%15s %511s %15s", method, path, version) != 3) {
      ax_send_response(client, 400, "Bad Request", "text/plain", "Bad Request", 0);
      break;
    }
    keep_alive = ax_should_keep_alive(headers, version);
    ax_strip_query(path);

    const ax_http_route* route = ax_find_route(routes, route_count, method, path);

    size_t total_needed = header_len;
    int is_chunked = ax_header_contains_token(headers, "Transfer-Encoding", "chunked");
    if (route != 0 && route->response_kind == 2 && !is_chunked) {
      size_t content_length = ax_content_length(headers);
      size_t initial_body = buffered > header_len ? buffered - header_len : 0;
      ax_stream_content_length_body(client, buffer + header_len, initial_body, content_length);
      break;
    }

    if (is_chunked) {
      int chunked_state = ax_find_chunked_end(buffer, buffered, header_len, &total_needed);
      while (chunked_state == 0) {
        int read_result = ax_read_more(client, buffer, &buffered);
        if (read_result <= 0) {
          close_client = 1;
          break;
        }
        chunked_state = ax_find_chunked_end(buffer, buffered, header_len, &total_needed);
      }
      if (close_client) {
        break;
      }
      if (chunked_state < 0) {
        ax_send_response(client, 400, "Bad Request", "text/plain", "Bad Request", 0);
        break;
      }
      if (route != 0 && route->response_kind == 2) {
        ax_send_chunked_stream_response_header(client);
        ax_send_all(client, buffer + header_len, total_needed - header_len);
        break;
      }
    } else {
      size_t content_length = ax_content_length(headers);
      total_needed = header_len + content_length;
      while (buffered < total_needed && total_needed <= AX_HTTP_BUFFER_SIZE) {
        int read_result = ax_read_more(client, buffer, &buffered);
        if (read_result <= 0) {
          close_client = 1;
          break;
        }
      }
      if (close_client) {
        break;
      }
      if (total_needed > AX_HTTP_BUFFER_SIZE) {
        size_t in_buffer_body = buffered > header_len ? buffered - header_len : 0;
        if (content_length > in_buffer_body) {
          ax_drain_bytes(client, content_length - in_buffer_body);
        }
        total_needed = buffered;
        keep_alive = 0;
      }
    }

    if (route == 0) {
      if (ax_path_exists(routes, route_count, path)) {
        ax_send_response(client, 405, "Method Not Allowed", "text/plain", "Method Not Allowed", keep_alive);
      } else {
        ax_send_response(client, 404, "Not Found", "text/plain", "Not Found", keep_alive);
      }
    } else {
      const char* content_type = ax_route_content_type(route);
      ax_send_response(client, 200, "OK", content_type, route->body, keep_alive);
    }

    if (!keep_alive) {
      break;
    }
    if (total_needed < buffered) {
      memmove(buffer, buffer + total_needed, buffered - total_needed);
      buffered -= total_needed;
    } else {
      buffered = 0;
    }
  }

  close(client);
}

static void* ax_http_client_thread(void* data) {
  ax_http_client* client = (ax_http_client*)data;
  ax_http_handle_client(client->client, client->routes, client->route_count);
  free(client);
  return 0;
}

int ax_http_server_start(int port, const ax_http_route* routes, int route_count) {
  signal(SIGPIPE, SIG_IGN);
  int server = ax_create_listener(port);
  if (server < 0) {
    return 1;
  }
  const char* threaded_env = getenv("AX_HTTP_THREADED");
  int threaded = threaded_env != 0 && strcmp(threaded_env, "0") != 0;
  ax_http_response_cache_enabled = !threaded;
  printf("ax http listen :%d\n", port);
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

    if (!threaded) {
      ax_http_handle_client(client, routes, route_count);
      continue;
    }

    ax_http_client* ctx = (ax_http_client*)malloc(sizeof(ax_http_client));
    if (ctx != 0) {
      ctx->client = client;
      ctx->routes = routes;
      ctx->route_count = route_count;
      pthread_t thread;
      if (pthread_create(&thread, 0, ax_http_client_thread, ctx) == 0) {
        pthread_detach(thread);
        continue;
      }
      free(ctx);
    }

    ax_http_handle_client(client, routes, route_count);
  }
}

#endif
