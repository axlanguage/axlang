#include <ctype.h>
#include <stdlib.h>
#include <string.h>

static char* slice_copy(const char* value, size_t len) {
  char* out = (char*)malloc(len + 1);
  memcpy(out, value, len);
  out[len] = 0;
  return out;
}

static int hex_value(char ch) {
  if (ch >= '0' && ch <= '9') return ch - '0';
  if (ch >= 'a' && ch <= 'f') return ch - 'a' + 10;
  if (ch >= 'A' && ch <= 'F') return ch - 'A' + 10;
  return -1;
}

static char* decode_slice(const char* value, size_t len) {
  char* out = (char*)malloc(len + 1);
  size_t out_len = 0;
  for (size_t i = 0; i < len; i++) {
    if (value[i] == '%' && i + 2 < len) {
      int hi = hex_value(value[i + 1]);
      int lo = hex_value(value[i + 2]);
      if (hi >= 0 && lo >= 0) {
        out[out_len++] = (char)((hi << 4) | lo);
        i += 2;
        continue;
      }
    }
    out[out_len++] = value[i] == '+' ? ' ' : value[i];
  }
  out[out_len] = 0;
  return out;
}

static int unreserved(unsigned char ch) {
  return isalnum(ch) || ch == '-' || ch == '.' || ch == '_' || ch == '~';
}

static char hex_digit(unsigned char value) {
  return value < 10 ? (char)('0' + value) : (char)('A' + value - 10);
}

static char* encode_url(const char* value) {
  size_t len = strlen(value);
  char* out = (char*)malloc((len * 3) + 1);
  size_t out_len = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = (unsigned char)value[i];
    if (unreserved(ch)) {
      out[out_len++] = (char)ch;
    } else {
      out[out_len++] = '%';
      out[out_len++] = hex_digit((unsigned char)(ch >> 4));
      out[out_len++] = hex_digit((unsigned char)(ch & 15));
    }
  }
  out[out_len] = 0;
  return out;
}

static const char* authority_start(const char* value) {
  const char* marker = strstr(value, "://");
  return marker == 0 ? value : marker + 3;
}

static char* host_url(const char* value) {
  const char* start = authority_start(value);
  const char* end = start;
  while (*end != 0 && *end != '/' && *end != '?' && *end != '#') end++;
  const char* colon = start;
  while (colon < end && *colon != ':') colon++;
  if (colon < end) end = colon;
  return slice_copy(start, (size_t)(end - start));
}

static char* path_url(const char* value) {
  const char* start = authority_start(value);
  const char* path = strchr(start, '/');
  if (path == 0) return slice_copy("/", 1);
  const char* end = path;
  while (*end != 0 && *end != '?' && *end != '#') end++;
  return slice_copy(path, (size_t)(end - path));
}

static char* query_get(const char* value, const char* key) {
  const char* cursor = strchr(value, '?');
  cursor = cursor == 0 ? value : cursor + 1;
  while (*cursor != 0 && *cursor != '#') {
    const char* key_start = cursor;
    while (*cursor != 0 && *cursor != '=' && *cursor != '&' && *cursor != '#') cursor++;
    size_t key_len = (size_t)(cursor - key_start);
    const char* value_start = cursor;
    size_t value_len = 0;
    if (*cursor == '=') {
      cursor++;
      value_start = cursor;
      while (*cursor != 0 && *cursor != '&' && *cursor != '#') cursor++;
      value_len = (size_t)(cursor - value_start);
    }
    char* decoded_key = decode_slice(key_start, key_len);
    int matched = strcmp(decoded_key, key) == 0;
    free(decoded_key);
    if (matched) return decode_slice(value_start, value_len);
    if (*cursor == '&') cursor++;
  }
  return slice_copy("", 0);
}

static int url_score(int n) {
  int i = 0;
  int acc = 0;
  const char* target = "https://agent.local/tools/search?q=Ax%20language&mode=fast";
  while (i < n) {
    char* host = host_url(target);
    char* path = path_url(target);
    char* query = query_get(target, "q");
    char* encoded = encode_url(path);
    char* decoded = decode_slice(encoded, strlen(encoded));
    acc += (int)strlen(host);
    acc += (int)strlen(decoded);
    acc += (int)strlen(query);
    free(host);
    free(path);
    free(query);
    free(encoded);
    free(decoded);
    i += 1;
  }
  return acc % 251;
}

int main(void) {
  return url_score(100000);
}
