#include "ax_runtime.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char* ax_io_strdup(const char* value) {
  const char* source = value == 0 ? "" : value;
  size_t len = strlen(source);
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return 0;
  }
  memcpy(out, source, len + 1);
  return out;
}

void ax_io_println(const char* s) {
  if (s == 0) {
    puts("");
    return;
  }
  puts(s);
}

void ax_io_print(const char* s) {
  if (s != 0) {
    fputs(s, stdout);
  }
  fflush(stdout);
}

void ax_io_eprintln(const char* s) {
  if (s == 0) {
    fputc('\n', stderr);
    return;
  }
  fputs(s, stderr);
  fputc('\n', stderr);
}

char* ax_io_read_line(void) {
  size_t cap = 128;
  size_t len = 0;
  char* line = (char*)malloc(cap);
  if (line == 0) {
    return ax_io_strdup("");
  }
  for (;;) {
    int ch = fgetc(stdin);
    if (ch == EOF || ch == '\n') {
      break;
    }
    if (len + 1 >= cap) {
      size_t next = cap * 2;
      char* resized = (char*)realloc(line, next);
      if (resized == 0) {
        free(line);
        return ax_io_strdup("");
      }
      line = resized;
      cap = next;
    }
    line[len++] = (char)ch;
  }
  if (len > 0 && line[len - 1] == '\r') {
    len--;
  }
  line[len] = 0;
  return line;
}
