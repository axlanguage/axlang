#include <string.h>
#include <stdlib.h>

static int starts_with(const char* value, const char* prefix) {
  return strncmp(value, prefix, strlen(prefix)) == 0;
}

static int ends_with(const char* value, const char* suffix) {
  size_t value_len = strlen(value);
  size_t suffix_len = strlen(suffix);
  return suffix_len <= value_len && strcmp(value + value_len - suffix_len, suffix) == 0;
}

static int text_score(int n) {
  int i = 0;
  int acc = 0;
  const char* text = getenv("AX_TEXT");
  if (text == 0 || *text == 0) {
    text = "agent-native-compiler-runtime-pack";
  }
  while (i < n) {
    if (strstr(text, "runtime") != 0) {
      acc += (int)strlen(text);
    }
    if (starts_with(text, "agent")) {
      acc += 1;
    }
    if (ends_with(text, "pack")) {
      acc += 2;
    }
    i += 1;
  }
  return acc % 251;
}

int main(void) {
  return text_score(5000000);
}
