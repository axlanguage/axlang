#include <string.h>

static int has_arg(int argc, char** argv, const char* name) {
  size_t name_len = strlen(name);
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], name) == 0) {
      return 1;
    }
    if (strncmp(argv[i], name, name_len) == 0 && argv[i][name_len] == '=') {
      return 1;
    }
  }
  return 0;
}

static const char* value_arg(int argc, char** argv, const char* name) {
  size_t name_len = strlen(name);
  for (int i = 1; i < argc; i++) {
    if (strncmp(argv[i], name, name_len) == 0 && argv[i][name_len] == '=') {
      return argv[i] + name_len + 1;
    }
    if (strcmp(argv[i], name) == 0 && i + 1 < argc) {
      return argv[i + 1];
    }
  }
  return "";
}

static int cli_score(int argc, char** argv, int n) {
  int i = 0;
  int acc = 0;
  while (i < n) {
    if (has_arg(argc, argv, "--input")) {
      acc += (int)strlen(value_arg(argc, argv, "--input"));
    }
    if (has_arg(argc, argv, "--mode")) {
      acc += (int)strlen(value_arg(argc, argv, "--mode"));
    }
    i += 1;
  }
  return acc % 251;
}

int main(int argc, char** argv) {
  return cli_score(argc, argv, 100000);
}
