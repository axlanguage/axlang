#include <stdlib.h>
#include <string.h>

static int is_sep(char ch) {
  return ch == '/' || ch == '\\';
}

static char* dup_text(const char* value) {
  size_t len = strlen(value);
  char* out = (char*)malloc(len + 1);
  memcpy(out, value, len + 1);
  return out;
}

static char* normalize_path(const char* value) {
  size_t len = strlen(value);
  char* out = (char*)malloc(len + 1);
  size_t w = 0;
  int last_sep = 0;
  for (size_t i = 0; i < len; i++) {
    char ch = value[i] == '\\' ? '/' : value[i];
    if (ch == '/') {
      if (last_sep) {
        continue;
      }
      last_sep = 1;
    } else {
      last_sep = 0;
    }
    if (ch == '.' && (i == 0 || value[i - 1] == '/') && (i + 1 == len || value[i + 1] == '/')) {
      continue;
    }
    out[w++] = ch;
  }
  out[w] = 0;
  return out;
}

static char* basename_path(const char* value) {
  const char* tail = value;
  for (const char* p = value; *p != 0; p++) {
    if (is_sep(*p)) {
      tail = p + 1;
    }
  }
  return dup_text(tail);
}

static int is_absolute_path(const char* value) {
  return value[0] == '/' || value[0] == '\\';
}

static int path_score(int n) {
  int i = 0;
  int acc = 0;
  const char* text = "examples//agents/./string_agent.ax";
  while (i < n) {
    char* normalized = normalize_path(text);
    char* base = basename_path(normalized);
    acc += (int)strlen(base);
    acc += is_absolute_path(text) ? 1 : 2;
    free(base);
    free(normalized);
    i += 1;
  }
  return acc % 251;
}

int main(void) {
  return path_score(1000000);
}
