#include "ax_runtime.h"

#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>

#ifdef _WIN32
#include <direct.h>
#include <io.h>
#include <windows.h>
#define ax_popen _popen
#define ax_pclose _pclose
#else
#include <dirent.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <unistd.h>
#define ax_popen popen
#define ax_pclose pclose
#endif

#if defined(_MSC_VER)
#define AX_THREAD_LOCAL __declspec(thread)
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#define AX_THREAD_LOCAL _Thread_local
#else
#define AX_THREAD_LOCAL __thread
#endif

static char* ax_strdup(const char* value) {
  const char* source = value == 0 ? "" : value;
  size_t len = strlen(source);
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return 0;
  }
  memcpy(out, source, len + 1);
  return out;
}

static int ax_append_bytes(char** buffer, size_t* len, size_t* cap, const char* value, size_t value_len);
static char* ax_slice_copy(const char* value, size_t len);

static int ax_cli_arg_count = 0;
static char** ax_cli_args = 0;
static char ax_cli_empty[] = "";

#define AX_CLI_CACHE_CAP 16
typedef struct {
  const char* name;
  int found;
  const char* value;
} AxCliLookup;

static AxCliLookup ax_cli_cache[AX_CLI_CACHE_CAP];
static int ax_cli_cache_count = 0;
static int ax_cli_cache_next = 0;

void ax_cli_init(int argc, char** argv) {
  ax_cli_arg_count = argc;
  ax_cli_args = argv;
  ax_cli_cache_count = 0;
  ax_cli_cache_next = 0;
}

int ax_cli_argc(void) {
  return ax_cli_arg_count;
}

char* ax_cli_arg(int index) {
  if (index < 0 || index >= ax_cli_arg_count || ax_cli_args == 0) {
    return ax_cli_empty;
  }
  return ax_cli_args[index] == 0 ? ax_cli_empty : ax_cli_args[index];
}

static int ax_cli_matches_value_form_len(const char* arg, const char* name, size_t name_len) {
  return strncmp(arg, name, name_len) == 0 && arg[name_len] == '=';
}

static const AxCliLookup* ax_cli_lookup(const char* name) {
  static AxCliLookup empty = {0, 0, ""};
  if (name == 0 || *name == 0 || ax_cli_args == 0) {
    return &empty;
  }
  for (int i = 0; i < ax_cli_cache_count; i++) {
    if (ax_cli_cache[i].name == name || strcmp(ax_cli_cache[i].name, name) == 0) {
      return &ax_cli_cache[i];
    }
  }

  size_t name_len = strlen(name);
  AxCliLookup found = {name, 0, ""};
  for (int i = 1; i < ax_cli_arg_count; i++) {
    const char* arg = ax_cli_args[i] == 0 ? "" : ax_cli_args[i];
    if (ax_cli_matches_value_form_len(arg, name, name_len)) {
      found.found = 1;
      found.value = arg + name_len + 1;
      break;
    }
    if (strcmp(arg, name) == 0) {
      found.found = 1;
      found.value = i + 1 < ax_cli_arg_count && ax_cli_args[i + 1] != 0 ? ax_cli_args[i + 1] : "";
      break;
    }
  }

  int slot = ax_cli_cache_count;
  if (slot < AX_CLI_CACHE_CAP) {
    ax_cli_cache_count++;
  } else {
    slot = ax_cli_cache_next;
    ax_cli_cache_next = (ax_cli_cache_next + 1) % AX_CLI_CACHE_CAP;
  }
  ax_cli_cache[slot] = found;
  return &ax_cli_cache[slot];
}

int ax_cli_has(const char* name) {
  return ax_cli_lookup(name)->found;
}

char* ax_cli_value(const char* name) {
  const AxCliLookup* lookup = ax_cli_lookup(name);
  return lookup->found && lookup->value != 0 ? (char*)lookup->value : ax_cli_empty;
}

char* ax_cli_value_or(const char* name, const char* fallback) {
  const char* default_value = fallback == 0 ? "" : fallback;
  const AxCliLookup* lookup = ax_cli_lookup(name);
  return lookup->found && lookup->value != 0 ? (char*)lookup->value : (char*)default_value;
}

char* ax_cli_args_json(void) {
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  for (int i = 0; i < ax_cli_arg_count; i++) {
    char* quoted = ax_json_quote(ax_cli_args == 0 ? "" : ax_cli_args[i]);
    if (quoted == 0) {
      quoted = ax_strdup("\"\"");
    }
    if (i > 0) {
      ax_append_bytes(&items, &len, &cap, ",", 1);
    }
    ax_append_bytes(&items, &len, &cap, quoted, strlen(quoted));
    free(quoted);
    if (items == 0) {
      return ax_strdup("[]");
    }
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result;
}

static int ax_cli_is_option_arg(const char* arg) {
  return arg != 0 && arg[0] == '-' && arg[1] != 0;
}

static int ax_cli_append_json_item(
    char** items,
    size_t* len,
    size_t* cap,
    int* count,
    const char* value) {
  char* quoted = ax_json_quote(value == 0 ? "" : value);
  if (quoted == 0) {
    quoted = ax_strdup("\"\"");
  }
  if (quoted == 0) {
    return 0;
  }
  int ok = 1;
  if (*count > 0) {
    ok = ok && ax_append_bytes(items, len, cap, ",", 1);
  }
  ok = ok && ax_append_bytes(items, len, cap, quoted, strlen(quoted));
  free(quoted);
  if (!ok) {
    return 0;
  }
  (*count)++;
  return 1;
}

static int ax_cli_append_json_field(
    char** fields,
    size_t* len,
    size_t* cap,
    int* count,
    const char* key,
    const char* value) {
  char* pair = ax_json_string_pair(key == 0 ? "" : key, value == 0 ? "" : value);
  if (pair == 0) {
    return 0;
  }
  int ok = 1;
  if (*count > 0) {
    ok = ok && ax_append_bytes(fields, len, cap, ",", 1);
  }
  ok = ok && ax_append_bytes(fields, len, cap, pair, strlen(pair));
  free(pair);
  if (!ok) {
    return 0;
  }
  (*count)++;
  return 1;
}

static int ax_cli_append_object_field(
    char** fields,
    size_t* len,
    size_t* cap,
    int* count,
    const char* pair) {
  if (pair == 0) {
    return 0;
  }
  int ok = 1;
  if (*count > 0) {
    ok = ok && ax_append_bytes(fields, len, cap, ",", 1);
  }
  ok = ok && ax_append_bytes(fields, len, cap, pair, strlen(pair));
  if (!ok) {
    return 0;
  }
  (*count)++;
  return 1;
}

char* ax_cli_parse_json(void) {
  char* positionals = 0;
  char* flags = 0;
  char* options = 0;
  size_t positionals_len = 0;
  size_t positionals_cap = 0;
  size_t flags_len = 0;
  size_t flags_cap = 0;
  size_t options_len = 0;
  size_t options_cap = 0;
  int positionals_count = 0;
  int flags_count = 0;
  int options_count = 0;
  int only_positionals = 0;

  for (int i = 1; i < ax_cli_arg_count; i++) {
    const char* arg = ax_cli_args == 0 ? "" : ax_cli_args[i];
    if (only_positionals) {
      ax_cli_append_json_item(&positionals, &positionals_len, &positionals_cap, &positionals_count, arg);
      continue;
    }
    if (strcmp(arg, "--") == 0) {
      only_positionals = 1;
      continue;
    }
    if (ax_cli_is_option_arg(arg)) {
      const char* equals = strchr(arg, '=');
      if (equals != 0 && equals != arg) {
        char* key = ax_slice_copy(arg, (size_t)(equals - arg));
        ax_cli_append_json_field(&options, &options_len, &options_cap, &options_count, key, equals + 1);
        free(key);
        continue;
      }
      if (i + 1 < ax_cli_arg_count && !ax_cli_is_option_arg(ax_cli_args[i + 1])) {
        ax_cli_append_json_field(&options, &options_len, &options_cap, &options_count, arg, ax_cli_args[i + 1]);
        i++;
        continue;
      }
      ax_cli_append_json_item(&flags, &flags_len, &flags_cap, &flags_count, arg);
      continue;
    }
    ax_cli_append_json_item(&positionals, &positionals_len, &positionals_cap, &positionals_count, arg);
  }

  char* args_json = ax_cli_args_json();
  char* positionals_json = ax_json_array(positionals == 0 ? "" : positionals);
  char* flags_json = ax_json_array(flags == 0 ? "" : flags);
  char* options_json = ax_json_object(options == 0 ? "" : options);
  char* program_pair = ax_json_string_pair("program", ax_cli_arg_count > 0 && ax_cli_args != 0 ? ax_cli_args[0] : "");
  char* args_pair = ax_json_pair("args", args_json);
  char* positionals_pair = ax_json_pair("positionals", positionals_json);
  char* flags_pair = ax_json_pair("flags", flags_json);
  char* options_pair = ax_json_pair("options", options_json);

  char* fields = 0;
  size_t fields_len = 0;
  size_t fields_cap = 0;
  int fields_count = 0;
  ax_cli_append_object_field(&fields, &fields_len, &fields_cap, &fields_count, program_pair);
  ax_cli_append_object_field(&fields, &fields_len, &fields_cap, &fields_count, args_pair);
  ax_cli_append_object_field(&fields, &fields_len, &fields_cap, &fields_count, positionals_pair);
  ax_cli_append_object_field(&fields, &fields_len, &fields_cap, &fields_count, flags_pair);
  ax_cli_append_object_field(&fields, &fields_len, &fields_cap, &fields_count, options_pair);
  char* result = fields == 0 ? ax_strdup("{}") : ax_json_object(fields);

  free(positionals);
  free(flags);
  free(options);
  free(args_json);
  free(positionals_json);
  free(flags_json);
  free(options_json);
  free(program_pair);
  free(args_pair);
  free(positionals_pair);
  free(flags_pair);
  free(options_pair);
  free(fields);
  return result == 0 ? ax_strdup("{}") : result;
}

static char* ax_fs_read_text_with_fallback(const char* path, const char* fallback) {
  const char* default_value = fallback == 0 ? "" : fallback;
  if (path == 0) {
    return ax_strdup(default_value);
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup(default_value);
  }
  if (fseek(file, 0, SEEK_END) != 0) {
    fclose(file);
    return ax_strdup(default_value);
  }
  long len = ftell(file);
  if (len < 0) {
    fclose(file);
    return ax_strdup(default_value);
  }
  rewind(file);
  char* buffer = (char*)malloc((size_t)len + 1);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup(default_value);
  }
  size_t read_len = fread(buffer, 1, (size_t)len, file);
  buffer[read_len] = 0;
  fclose(file);
  return buffer;
}

char* ax_fs_read_text(const char* path) {
  return ax_fs_read_text_with_fallback(path, "");
}

char* ax_fs_read_text_or(const char* path, const char* fallback) {
  return ax_fs_read_text_with_fallback(path, fallback);
}

static char* ax_fs_base64_encode_bytes(const unsigned char* bytes, size_t len) {
  static const char* table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  if (bytes == 0 && len > 0) {
    return ax_strdup("");
  }
  if (len > ((((size_t)-1) / 4) * 3) - 2) {
    return ax_strdup("");
  }
  size_t out_len = ((len + 2) / 3) * 4;
  char* output = (char*)malloc(out_len + 1);
  if (output == 0) {
    return ax_strdup("");
  }

  size_t input_idx = 0;
  size_t output_idx = 0;
  while (input_idx < len) {
    uint32_t octet_a = input_idx < len ? bytes[input_idx++] : 0;
    uint32_t octet_b = input_idx < len ? bytes[input_idx++] : 0;
    uint32_t octet_c = input_idx < len ? bytes[input_idx++] : 0;
    uint32_t triple = (octet_a << 16) | (octet_b << 8) | octet_c;

    output[output_idx++] = table[(triple >> 18) & 0x3f];
    output[output_idx++] = table[(triple >> 12) & 0x3f];
    output[output_idx++] = table[(triple >> 6) & 0x3f];
    output[output_idx++] = table[triple & 0x3f];
  }

  size_t padding = (3 - (len % 3)) % 3;
  for (size_t i = 0; i < padding; i++) {
    output[out_len - 1 - i] = '=';
  }
  output[out_len] = 0;
  return output;
}

static int ax_fs_base64_value(unsigned char ch) {
  if (ch >= 'A' && ch <= 'Z') {
    return ch - 'A';
  }
  if (ch >= 'a' && ch <= 'z') {
    return ch - 'a' + 26;
  }
  if (ch >= '0' && ch <= '9') {
    return ch - '0' + 52;
  }
  if (ch == '+') {
    return 62;
  }
  if (ch == '/') {
    return 63;
  }
  return -1;
}

static int ax_fs_base64_space(unsigned char ch) {
  return ch == ' ' || ch == '\n' || ch == '\r' || ch == '\t';
}

static unsigned char* ax_fs_base64_decode_bytes(const char* input, size_t* out_len) {
  if (out_len != 0) {
    *out_len = 0;
  }
  const char* source = input == 0 ? "" : input;
  size_t len = strlen(source);
  size_t data_count = 0;
  size_t padding_count = 0;
  size_t significant_count = 0;
  int seen_padding = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = (unsigned char)source[i];
    if (ax_fs_base64_space(ch)) {
      continue;
    }
    significant_count++;
    if (ch == '=') {
      seen_padding = 1;
      padding_count++;
      continue;
    }
    if (seen_padding || ax_fs_base64_value(ch) < 0) {
      return 0;
    }
    data_count++;
  }
  if (padding_count > 2 || (padding_count > 0 && significant_count % 4 != 0)) {
    return 0;
  }
  if (padding_count == 1 && data_count % 4 != 3) {
    return 0;
  }
  if (padding_count == 2 && data_count % 4 != 2) {
    return 0;
  }
  if (padding_count == 0 && data_count % 4 == 1) {
    return 0;
  }

  unsigned char* output = (unsigned char*)malloc(((len + 3) / 4) * 3 + 1);
  if (output == 0) {
    return 0;
  }

  uint32_t buffer = 0;
  int bits = 0;
  seen_padding = 0;
  size_t output_idx = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = (unsigned char)source[i];
    if (ax_fs_base64_space(ch)) {
      continue;
    }
    if (ch == '=') {
      seen_padding = 1;
      continue;
    }
    if (seen_padding) {
      free(output);
      return 0;
    }
    int value = ax_fs_base64_value(ch);
    if (value < 0) {
      free(output);
      return 0;
    }
    buffer = (buffer << 6) | (uint32_t)value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      output[output_idx++] = (unsigned char)((buffer >> bits) & 0xff);
    }
  }

  if (out_len != 0) {
    *out_len = output_idx;
  }
  return output;
}

char* ax_fs_read_base64(const char* path) {
  if (path == 0) {
    return ax_strdup("");
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  if (fseek(file, 0, SEEK_END) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  long len = ftell(file);
  if (len < 0) {
    fclose(file);
    return ax_strdup("");
  }
  rewind(file);
  size_t byte_len = (size_t)len;
  unsigned char* buffer = (unsigned char*)malloc(byte_len == 0 ? 1 : byte_len);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, byte_len, file);
  fclose(file);
  char* encoded = ax_fs_base64_encode_bytes(buffer, read_len);
  free(buffer);
  return encoded == 0 ? ax_strdup("") : encoded;
}

char* ax_fs_read_base64_range(const char* path, int offset, int max_bytes) {
  if (path == 0 || max_bytes <= 0) {
    return ax_strdup("");
  }
  if (offset < 0) {
    offset = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  if (fseek(file, (long)offset, SEEK_SET) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  unsigned char* buffer = (unsigned char*)malloc((size_t)max_bytes);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, (size_t)max_bytes, file);
  fclose(file);
  char* encoded = ax_fs_base64_encode_bytes(buffer, read_len);
  free(buffer);
  return encoded == 0 ? ax_strdup("") : encoded;
}

char* ax_fs_read_base64_tail(const char* path, int max_bytes) {
  if (path == 0 || max_bytes <= 0) {
    return ax_strdup("");
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  if (fseek(file, 0, SEEK_END) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  long len = ftell(file);
  if (len < 0) {
    fclose(file);
    return ax_strdup("");
  }
  long start = len > max_bytes ? len - max_bytes : 0;
  if (fseek(file, start, SEEK_SET) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  unsigned char* buffer = (unsigned char*)malloc((size_t)max_bytes);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, (size_t)max_bytes, file);
  fclose(file);
  char* encoded = ax_fs_base64_encode_bytes(buffer, read_len);
  free(buffer);
  return encoded == 0 ? ax_strdup("") : encoded;
}

char* ax_fs_read_text_limit(const char* path, int max_bytes) {
  if (path == 0 || max_bytes <= 0) {
    return ax_strdup("");
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  char* buffer = (char*)malloc((size_t)max_bytes + 1);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, (size_t)max_bytes, file);
  buffer[read_len] = 0;
  fclose(file);
  return buffer;
}

char* ax_fs_read_text_range(const char* path, int offset, int max_bytes) {
  if (path == 0 || max_bytes <= 0) {
    return ax_strdup("");
  }
  if (offset < 0) {
    offset = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  if (fseek(file, (long)offset, SEEK_SET) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  char* buffer = (char*)malloc((size_t)max_bytes + 1);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, (size_t)max_bytes, file);
  buffer[read_len] = 0;
  fclose(file);
  return buffer;
}

char* ax_fs_read_text_tail(const char* path, int max_bytes) {
  if (path == 0 || max_bytes <= 0) {
    return ax_strdup("");
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }
  if (fseek(file, 0, SEEK_END) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  long len = ftell(file);
  if (len < 0) {
    fclose(file);
    return ax_strdup("");
  }
  long start = len > max_bytes ? len - max_bytes : 0;
  if (fseek(file, start, SEEK_SET) != 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t want = (size_t)(len - start);
  char* buffer = (char*)malloc(want + 1);
  if (buffer == 0) {
    fclose(file);
    return ax_strdup("");
  }
  size_t read_len = fread(buffer, 1, want, file);
  buffer[read_len] = 0;
  fclose(file);
  return buffer;
}

char* ax_fs_read_lines(const char* path, int start_line, int max_lines) {
  if (path == 0 || max_lines <= 0) {
    return ax_strdup("");
  }
  if (start_line < 0) {
    start_line = 0;
  }
  if (max_lines > 10000) {
    max_lines = 10000;
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_strdup("");
  }

  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  int current_line = 0;
  int captured_lines = 0;
  int ch = 0;
  while ((ch = fgetc(file)) != EOF) {
    if (current_line >= start_line && captured_lines < max_lines) {
      char byte = (char)ch;
      if (len >= 1024 * 1024 || !ax_append_bytes(&buffer, &len, &cap, &byte, 1)) {
        fclose(file);
        return ax_strdup("");
      }
    }
    if (ch == '\n') {
      if (current_line >= start_line && captured_lines < max_lines) {
        captured_lines++;
        if (captured_lines >= max_lines) {
          break;
        }
      }
      current_line++;
    }
    if (len >= 1024 * 1024) {
      break;
    }
  }
  fclose(file);
  return buffer == 0 ? ax_strdup("") : buffer;
}

static char* ax_text_lines_json_array(const char* text) {
  if (text == 0 || *text == 0) {
    return ax_strdup("[]");
  }
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const char* cursor = text;
  const char* end = text + strlen(text);
  while (cursor < end) {
    const char* line = cursor;
    while (cursor < end && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0 && line[line_len - 1] == '\r') {
      line_len--;
    }
    char* line_text = (char*)malloc(line_len + 1);
    if (line_text == 0) {
      free(items);
      return ax_strdup("[]");
    }
    if (line_len > 0) {
      memcpy(line_text, line, line_len);
    }
    line_text[line_len] = 0;
    char* quoted = ax_json_quote(line_text);
    free(line_text);
    if (quoted == 0) {
      quoted = ax_strdup("\"\"");
    }
    if (quoted == 0) {
      free(items);
      return ax_strdup("[]");
    }
    if (count > 0 && !ax_append_bytes(&items, &len, &cap, ",", 1)) {
      free(quoted);
      return ax_strdup("[]");
    }
    if (!ax_append_bytes(&items, &len, &cap, quoted, strlen(quoted))) {
      free(quoted);
      return ax_strdup("[]");
    }
    free(quoted);
    count++;
    if (cursor < end && *cursor == '\n') {
      cursor++;
    }
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_fs_read_lines_json(const char* path, int start_line, int max_lines) {
  char* lines = ax_fs_read_lines(path, start_line, max_lines);
  char* result = ax_text_lines_json_array(lines);
  free(lines);
  return result;
}

static char* ax_text_jsonl_array(const char* text) {
  if (text == 0 || *text == 0) {
    return ax_strdup("[]");
  }
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const char* cursor = text;
  const char* end = text + strlen(text);
  while (cursor < end) {
    const char* line = cursor;
    while (cursor < end && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0 && line[line_len - 1] == '\r') {
      line_len--;
    }
    char* line_text = (char*)malloc(line_len + 1);
    if (line_text == 0) {
      free(items);
      return ax_strdup("[]");
    }
    if (line_len > 0) {
      memcpy(line_text, line, line_len);
    }
    line_text[line_len] = 0;
    char* value = ax_json_valid(line_text) ? ax_json_compact(line_text) : ax_strdup("null");
    free(line_text);
    if (value == 0) {
      value = ax_strdup("null");
    }
    if (value == 0) {
      free(items);
      return ax_strdup("[]");
    }
    if (count > 0 && !ax_append_bytes(&items, &len, &cap, ",", 1)) {
      free(value);
      return ax_strdup("[]");
    }
    if (!ax_append_bytes(&items, &len, &cap, value, strlen(value))) {
      free(value);
      return ax_strdup("[]");
    }
    free(value);
    count++;
    if (cursor < end && *cursor == '\n') {
      cursor++;
    }
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_fs_read_jsonl(const char* path, int start_line, int max_lines) {
  char* lines = ax_fs_read_lines(path, start_line, max_lines);
  char* result = ax_text_jsonl_array(lines);
  free(lines);
  return result;
}

char* ax_fs_read_json(const char* path) {
  char* text = ax_fs_read_text(path);
  if (text == 0) {
    return ax_strdup("");
  }
  if (!ax_json_valid(text)) {
    free(text);
    return ax_strdup("");
  }
  char* compact = ax_json_compact(text);
  free(text);
  return compact == 0 ? ax_strdup("") : compact;
}

char* ax_fs_read_json_or(const char* path, const char* fallback_json) {
  char* text = ax_fs_read_json(path);
  if (text != 0 && *text != 0) {
    return text;
  }
  free(text);
  const char* fallback = fallback_json == 0 ? "" : fallback_json;
  if (!ax_json_valid(fallback)) {
    return ax_strdup("");
  }
  char* compact = ax_json_compact(fallback);
  return compact == 0 ? ax_strdup("") : compact;
}

void ax_fs_write_text(const char* path, const char* content) {
  if (path == 0) {
    return;
  }
  FILE* file = fopen(path, "wb");
  if (file == 0) {
    return;
  }
  if (content != 0) {
    fwrite(content, 1, strlen(content), file);
  }
  fclose(file);
}

static unsigned int ax_fs_atomic_write_counter = 0;

static char* ax_fs_atomic_temp_path(const char* path) {
  if (path == 0 || *path == 0) {
    return 0;
  }
#ifdef _WIN32
  unsigned long pid = (unsigned long)GetCurrentProcessId();
#else
  unsigned long pid = (unsigned long)getpid();
#endif
  unsigned int counter = ++ax_fs_atomic_write_counter;
  char suffix[96];
  int suffix_len = snprintf(
      suffix,
      sizeof(suffix),
      ".ax-tmp.%lu.%ld.%u",
      pid,
      (long)time(0),
      counter);
  if (suffix_len <= 0 || (size_t)suffix_len >= sizeof(suffix)) {
    return 0;
  }
  size_t path_len = strlen(path);
  char* tmp_path = (char*)malloc(path_len + (size_t)suffix_len + 1);
  if (tmp_path == 0) {
    return 0;
  }
  memcpy(tmp_path, path, path_len);
  memcpy(tmp_path + path_len, suffix, (size_t)suffix_len + 1);
  return tmp_path;
}

void ax_fs_write_text_atomic(const char* path, const char* content) {
  char* tmp_path = ax_fs_atomic_temp_path(path);
  if (tmp_path == 0) {
    return;
  }
  FILE* file = fopen(tmp_path, "wb");
  if (file == 0) {
    free(tmp_path);
    return;
  }
  int ok = 1;
  if (content != 0) {
    size_t len = strlen(content);
    if (fwrite(content, 1, len, file) != len) {
      ok = 0;
    }
  }
  if (fflush(file) != 0) {
    ok = 0;
  }
  if (fclose(file) != 0) {
    ok = 0;
  }
  if (!ok) {
    remove(tmp_path);
    free(tmp_path);
    return;
  }
#ifdef _WIN32
  if (!MoveFileExA(tmp_path, path, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    remove(tmp_path);
  }
#else
  if (rename(tmp_path, path) != 0) {
    remove(tmp_path);
  }
#endif
  free(tmp_path);
}

void ax_fs_write_json_atomic(const char* path, const char* json) {
  if (path == 0 || json == 0 || !ax_json_valid(json)) {
    return;
  }
  char* compact = ax_json_compact(json);
  if (compact == 0) {
    return;
  }
  ax_fs_write_text_atomic(path, compact);
  free(compact);
}

void ax_fs_write_base64(const char* path, const char* data_base64) {
  if (path == 0) {
    return;
  }
  size_t byte_len = 0;
  unsigned char* bytes = ax_fs_base64_decode_bytes(data_base64, &byte_len);
  if (bytes == 0) {
    return;
  }
  FILE* file = fopen(path, "wb");
  if (file == 0) {
    free(bytes);
    return;
  }
  if (byte_len > 0) {
    fwrite(bytes, 1, byte_len, file);
  }
  fclose(file);
  free(bytes);
}

void ax_fs_append_text(const char* path, const char* content) {
  if (path == 0) {
    return;
  }
  FILE* file = fopen(path, "ab");
  if (file == 0) {
    return;
  }
  if (content != 0) {
    fwrite(content, 1, strlen(content), file);
  }
  fclose(file);
}

void ax_fs_append_jsonl(const char* path, const char* json) {
  if (path == 0) {
    return;
  }
  const char* input = json == 0 ? "null" : json;
  char* compact = ax_json_valid(input) ? ax_json_compact(input) : ax_strdup("null");
  if (compact == 0) {
    return;
  }
  FILE* file = fopen(path, "ab");
  if (file == 0) {
    free(compact);
    return;
  }
  size_t len = strlen(compact);
  if (len > 0) {
    fwrite(compact, 1, len, file);
  }
  fwrite("\n", 1, 1, file);
  fclose(file);
  free(compact);
}

int ax_fs_exists(const char* path) {
  if (path == 0) {
    return 0;
  }
#ifdef _WIN32
  return _access(path, 0) == 0 ? 1 : 0;
#else
  return access(path, F_OK) == 0 ? 1 : 0;
#endif
}

void ax_fs_remove(const char* path) {
  if (path != 0) {
    remove(path);
  }
}

static int ax_fs_is_dangerous_remove_dir_target(const char* path) {
  return path == 0 || *path == 0 || strcmp(path, ".") == 0 || strcmp(path, "..") == 0 ||
         strcmp(path, "/") == 0 || strcmp(path, "\\") == 0;
}

static int ax_fs_is_path_separator(char value) {
  return value == '/' || value == '\\';
}

static int ax_fs_is_root_or_drive_prefix(const char* path) {
  if (path == 0 || *path == 0) {
    return 1;
  }
  size_t len = strlen(path);
  if (len == 1 && ax_fs_is_path_separator(path[0])) {
    return 1;
  }
  if (len == 2 && isalpha((unsigned char)path[0]) && path[1] == ':') {
    return 1;
  }
  if (len == 3 && isalpha((unsigned char)path[0]) && path[1] == ':' && ax_fs_is_path_separator(path[2])) {
    return 1;
  }
  return 0;
}

static void ax_fs_mkdir_one(const char* path) {
  if (path != 0 && *path != 0 && !ax_fs_is_root_or_drive_prefix(path)) {
#ifdef _WIN32
    _mkdir(path);
#else
    mkdir(path, 0755);
#endif
  }
}

void ax_fs_mkdir(const char* path) {
  ax_fs_mkdir_one(path);
}

void ax_fs_mkdir_all(const char* path) {
  if (path == 0 || *path == 0) {
    return;
  }
  char* copy = ax_strdup(path);
  if (copy == 0) {
    return;
  }
  size_t len = strlen(copy);
  while (len > 1 && ax_fs_is_path_separator(copy[len - 1])) {
    if (len == 3 && isalpha((unsigned char)copy[0]) && copy[1] == ':') {
      break;
    }
    copy[len - 1] = 0;
    len--;
  }
  for (char* p = copy + 1; *p != 0; p++) {
    if (!ax_fs_is_path_separator(*p)) {
      continue;
    }
    char saved = *p;
    *p = 0;
    ax_fs_mkdir_one(copy);
    *p = saved;
    while (ax_fs_is_path_separator(*(p + 1))) {
      p++;
    }
  }
  ax_fs_mkdir_one(copy);
  free(copy);
}

void ax_fs_ensure_parent(const char* path) {
  if (path == 0 || *path == 0) {
    return;
  }
  size_t len = strlen(path);
  while (len > 0 && ax_fs_is_path_separator(path[len - 1])) {
    len--;
  }
  if (len == 0) {
    return;
  }
  size_t parent_end = len;
  while (parent_end > 0 && !ax_fs_is_path_separator(path[parent_end - 1])) {
    parent_end--;
  }
  if (parent_end == 0) {
    return;
  }
  size_t parent_len = parent_end - 1;
  while (parent_len > 1 && ax_fs_is_path_separator(path[parent_len - 1])) {
    parent_len--;
  }
  if (parent_len == 0) {
    return;
  }
  char* parent = (char*)malloc(parent_len + 1);
  if (parent == 0) {
    return;
  }
  memcpy(parent, path, parent_len);
  parent[parent_len] = 0;
  ax_fs_mkdir_all(parent);
  free(parent);
}

void ax_fs_copy(const char* source, const char* target) {
  if (source == 0 || target == 0) {
    return;
  }
  FILE* input = fopen(source, "rb");
  if (input == 0) {
    return;
  }
  FILE* output = fopen(target, "wb");
  if (output == 0) {
    fclose(input);
    return;
  }
  char buffer[4096];
  size_t read_len = 0;
  while ((read_len = fread(buffer, 1, sizeof(buffer), input)) > 0) {
    fwrite(buffer, 1, read_len, output);
  }
  fclose(output);
  fclose(input);
}

long long ax_fs_size(const char* path) {
  if (path == 0) {
    return -1;
  }
#ifdef _WIN32
  struct _stat64 info;
  if (_stat64(path, &info) != 0) {
    return -1;
  }
  return (long long)info.st_size;
#else
  struct stat info;
  if (stat(path, &info) != 0) {
    return -1;
  }
  return (long long)info.st_size;
#endif
}

char* ax_fs_stat_json(const char* path) {
  const char* safe_path = path == 0 ? "" : path;
  int exists = 0;
  int is_file = 0;
  int is_dir = 0;
  long long size = -1;
  long long modified = -1;

#ifdef _WIN32
  struct _stat64 info;
  if (_stat64(safe_path, &info) == 0) {
    exists = 1;
    is_file = (info.st_mode & _S_IFREG) != 0 ? 1 : 0;
    is_dir = (info.st_mode & _S_IFDIR) != 0 ? 1 : 0;
    size = (long long)info.st_size;
    modified = (long long)info.st_mtime;
  }
#else
  struct stat info;
  if (stat(safe_path, &info) == 0) {
    exists = 1;
    is_file = S_ISREG(info.st_mode) ? 1 : 0;
    is_dir = S_ISDIR(info.st_mode) ? 1 : 0;
    size = (long long)info.st_size;
    modified = (long long)info.st_mtime;
  }
#endif

  char* quoted_path = ax_json_quote(safe_path);
  if (quoted_path == 0) {
    quoted_path = ax_strdup("\"\"");
  }
  int needed = snprintf(
      0,
      0,
      "{\"path\":%s,\"exists\":%s,\"is_file\":%s,\"is_dir\":%s,\"size\":%lld,\"modified\":%lld}",
      quoted_path,
      exists ? "true" : "false",
      is_file ? "true" : "false",
      is_dir ? "true" : "false",
      size,
      modified);
  if (needed <= 0) {
    free(quoted_path);
    return ax_strdup("{}");
  }
  char* result = (char*)malloc((size_t)needed + 1);
  if (result == 0) {
    free(quoted_path);
    return ax_strdup("{}");
  }
  snprintf(
      result,
      (size_t)needed + 1,
      "{\"path\":%s,\"exists\":%s,\"is_file\":%s,\"is_dir\":%s,\"size\":%lld,\"modified\":%lld}",
      quoted_path,
      exists ? "true" : "false",
      is_file ? "true" : "false",
      is_dir ? "true" : "false",
      size,
      modified);
  free(quoted_path);
  return result;
}

void ax_fs_rename(const char* source, const char* target) {
  if (source != 0 && target != 0) {
    rename(source, target);
  }
}

int ax_fs_is_file(const char* path) {
  if (path == 0) {
    return 0;
  }
#ifdef _WIN32
  struct _stat64 info;
  if (_stat64(path, &info) != 0) {
    return 0;
  }
  return (info.st_mode & _S_IFREG) != 0 ? 1 : 0;
#else
  struct stat info;
  if (stat(path, &info) != 0) {
    return 0;
  }
  return S_ISREG(info.st_mode) ? 1 : 0;
#endif
}

int ax_fs_is_dir(const char* path) {
  if (path == 0) {
    return 0;
  }
#ifdef _WIN32
  struct _stat64 info;
  if (_stat64(path, &info) != 0) {
    return 0;
  }
  return (info.st_mode & _S_IFDIR) != 0 ? 1 : 0;
#else
  struct stat info;
  if (stat(path, &info) != 0) {
    return 0;
  }
  return S_ISDIR(info.st_mode) ? 1 : 0;
#endif
}

long long ax_fs_modified(const char* path) {
  if (path == 0) {
    return -1;
  }
#ifdef _WIN32
  struct _stat64 info;
  if (_stat64(path, &info) != 0) {
    return -1;
  }
#else
  struct stat info;
  if (stat(path, &info) != 0) {
    return -1;
  }
#endif
  return (long long)info.st_mtime;
}

char* ax_fs_cwd(void) {
#ifdef _WIN32
  char buffer[MAX_PATH];
  if (_getcwd(buffer, sizeof(buffer)) == 0) {
    return ax_strdup("");
  }
#else
  char buffer[4096];
  if (getcwd(buffer, sizeof(buffer)) == 0) {
    return ax_strdup("");
  }
#endif
  return ax_strdup(buffer);
}

char* ax_fs_temp_dir(void) {
#ifdef _WIN32
  char buffer[MAX_PATH];
  DWORD len = GetTempPathA((DWORD)sizeof(buffer), buffer);
  if (len == 0 || len >= sizeof(buffer)) {
    return ax_strdup("");
  }
  return ax_strdup(buffer);
#else
  const char* value = getenv("TMPDIR");
  if (value == 0 || *value == 0) {
    value = "/tmp";
  }
  return ax_strdup(value);
#endif
}

static void ax_append_text(char** buffer, size_t* len, size_t* cap, const char* value) {
  size_t value_len = strlen(value);
  size_t needed = *len + value_len + 2;
  if (needed > *cap) {
    size_t next = *cap == 0 ? 128 : *cap;
    while (next < needed) {
      next *= 2;
    }
    char* resized = (char*)realloc(*buffer, next);
    if (resized == 0) {
      free(*buffer);
      *buffer = 0;
      *len = 0;
      *cap = 0;
      return;
    }
    *buffer = resized;
    *cap = next;
  }
  memcpy(*buffer + *len, value, value_len);
  *len += value_len;
  (*buffer)[(*len)++] = '\n';
  (*buffer)[*len] = 0;
}

static int ax_append_bytes(char** buffer, size_t* len, size_t* cap, const char* value, size_t value_len) {
  size_t needed = *len + value_len + 1;
  if (needed > *cap) {
    size_t next = *cap == 0 ? 256 : *cap;
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

static char* ax_slice_copy(const char* value, size_t len) {
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  if (len > 0) {
    memcpy(out, value, len);
  }
  out[len] = 0;
  return out;
}

static char* ax_lines_json_array(const char* lines) {
  if (lines == 0 || *lines == 0) {
    return ax_strdup("[]");
  }
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const char* cursor = lines;
  while (*cursor != 0) {
    const char* line = cursor;
    while (*cursor != 0 && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0) {
      char* item = ax_slice_copy(line, line_len);
      char* quoted = ax_json_quote(item);
      free(item);
      if (quoted == 0) {
        quoted = ax_strdup("\"\"");
      }
      if (quoted == 0) {
        return ax_strdup("[]");
      }
      if (count > 0 && !ax_append_bytes(&items, &len, &cap, ",", 1)) {
        free(quoted);
        return ax_strdup("[]");
      }
      if (!ax_append_bytes(&items, &len, &cap, quoted, strlen(quoted))) {
        free(quoted);
        return ax_strdup("[]");
      }
      free(quoted);
      count++;
    }
    if (*cursor == '\n') {
      cursor++;
    }
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_fs_list(const char* path) {
#ifdef _WIN32
  const char* base = path == 0 ? "." : path;
  size_t base_len = strlen(base);
  int needs_sep = base_len > 0 && base[base_len - 1] != '\\' && base[base_len - 1] != '/';
  char* pattern = (char*)malloc(base_len + (needs_sep ? 3 : 2));
  if (pattern == 0) {
    return ax_strdup("");
  }
  memcpy(pattern, base, base_len);
  size_t offset = base_len;
  if (needs_sep) {
    pattern[offset++] = '\\';
  }
  pattern[offset++] = '*';
  pattern[offset] = 0;

  WIN32_FIND_DATAA data;
  HANDLE handle = FindFirstFileA(pattern, &data);
  free(pattern);
  if (handle == INVALID_HANDLE_VALUE) {
    return ax_strdup("");
  }

  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  do {
    if (strcmp(data.cFileName, ".") == 0 || strcmp(data.cFileName, "..") == 0) {
      continue;
    }
    ax_append_text(&buffer, &len, &cap, data.cFileName);
    if (buffer == 0) {
      break;
    }
  } while (FindNextFileA(handle, &data));
  FindClose(handle);
  if (buffer == 0) {
    return ax_strdup("");
  }
  return buffer;
#else
  DIR* dir = opendir(path == 0 ? "." : path);
  if (dir == 0) {
    return ax_strdup("");
  }
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  struct dirent* entry = 0;
  while ((entry = readdir(dir)) != 0) {
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
      continue;
    }
    ax_append_text(&buffer, &len, &cap, entry->d_name);
    if (buffer == 0) {
      break;
    }
  }
  closedir(dir);
  if (buffer == 0) {
    return ax_strdup("");
  }
  return buffer;
#endif
}

char* ax_fs_list_json(const char* path) {
  char* listed = ax_fs_list(path);
  char* result = ax_lines_json_array(listed);
  free(listed);
  return result;
}

static char* ax_fs_join_child_path(const char* base, const char* name, char separator) {
  const char* root = base == 0 || *base == 0 ? "." : base;
  const char* child_name = name == 0 ? "" : name;
  size_t root_len = strlen(root);
  size_t name_len = strlen(child_name);
  int needs_sep = root_len > 0 && !ax_fs_is_path_separator(root[root_len - 1]);
  char* child = (char*)malloc(root_len + (needs_sep ? 1 : 0) + name_len + 1);
  if (child == 0) {
    return 0;
  }
  memcpy(child, root, root_len);
  size_t offset = root_len;
  if (needs_sep) {
    child[offset++] = separator;
  }
  memcpy(child + offset, child_name, name_len + 1);
  return child;
}

static char ax_fs_host_separator(void) {
#ifdef _WIN32
  return '\\';
#else
  return '/';
#endif
}

static char* ax_lines_stat_json_array(const char* lines, const char* base, int join_base) {
  if (lines == 0 || *lines == 0) {
    return ax_strdup("[]");
  }
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const char* cursor = lines;
  while (*cursor != 0) {
    const char* line = cursor;
    while (*cursor != 0 && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0) {
      char* item = ax_slice_copy(line, line_len);
      char* stat_path =
          join_base ? ax_fs_join_child_path(base, item, ax_fs_host_separator()) : ax_strdup(item);
      free(item);
      char* stat = ax_fs_stat_json(stat_path);
      free(stat_path);
      if (stat == 0) {
        stat = ax_strdup("{}");
      }
      if (stat == 0) {
        free(items);
        return ax_strdup("[]");
      }
      if (count > 0 && !ax_append_bytes(&items, &len, &cap, ",", 1)) {
        free(stat);
        return ax_strdup("[]");
      }
      if (!ax_append_bytes(&items, &len, &cap, stat, strlen(stat))) {
        free(stat);
        return ax_strdup("[]");
      }
      free(stat);
      count++;
    }
    if (*cursor == '\n') {
      cursor++;
    }
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_fs_list_stat_json(const char* path) {
  char* listed = ax_fs_list(path);
  char* result = ax_lines_stat_json_array(listed, path == 0 || *path == 0 ? "." : path, 1);
  free(listed);
  return result;
}

#ifdef _WIN32
static void ax_fs_walk_into(const char* path, char** buffer, size_t* len, size_t* cap) {
  if (path == 0 || buffer == 0 || (*buffer == 0 && *cap == 0 && *len != 0)) {
    return;
  }
  size_t base_len = strlen(path);
  int needs_sep = base_len > 0 && !ax_fs_is_path_separator(path[base_len - 1]);
  char* pattern = (char*)malloc(base_len + (needs_sep ? 3 : 2));
  if (pattern == 0) {
    return;
  }
  memcpy(pattern, path, base_len);
  size_t offset = base_len;
  if (needs_sep) {
    pattern[offset++] = '\\';
  }
  pattern[offset++] = '*';
  pattern[offset] = 0;

  WIN32_FIND_DATAA data;
  HANDLE handle = FindFirstFileA(pattern, &data);
  free(pattern);
  if (handle == INVALID_HANDLE_VALUE) {
    return;
  }
  do {
    if (strcmp(data.cFileName, ".") == 0 || strcmp(data.cFileName, "..") == 0) {
      continue;
    }
    char* child = ax_fs_join_child_path(path, data.cFileName, '\\');
    if (child == 0) {
      continue;
    }
    ax_append_text(buffer, len, cap, child);
    if (*buffer == 0) {
      free(child);
      break;
    }
    if ((data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
      ax_fs_walk_into(child, buffer, len, cap);
    }
    free(child);
  } while (FindNextFileA(handle, &data));
  FindClose(handle);
}
#else
static void ax_fs_walk_into(const char* path, char** buffer, size_t* len, size_t* cap) {
  if (path == 0 || buffer == 0 || (*buffer == 0 && *cap == 0 && *len != 0)) {
    return;
  }
  DIR* dir = opendir(path);
  if (dir == 0) {
    return;
  }
  struct dirent* entry = 0;
  while ((entry = readdir(dir)) != 0) {
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
      continue;
    }
    char* child = ax_fs_join_child_path(path, entry->d_name, '/');
    if (child == 0) {
      continue;
    }
    ax_append_text(buffer, len, cap, child);
    if (buffer == 0 || *buffer == 0) {
      free(child);
      break;
    }
    struct stat info;
    if (lstat(child, &info) == 0 && S_ISDIR(info.st_mode)) {
      ax_fs_walk_into(child, buffer, len, cap);
    }
    free(child);
  }
  closedir(dir);
}
#endif

char* ax_fs_walk(const char* path) {
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  ax_fs_walk_into(path == 0 || *path == 0 ? "." : path, &buffer, &len, &cap);
  return buffer == 0 ? ax_strdup("") : buffer;
}

char* ax_fs_walk_json(const char* path) {
  char* walked = ax_fs_walk(path);
  char* result = ax_lines_json_array(walked);
  free(walked);
  return result;
}

char* ax_fs_walk_stat_json(const char* path) {
  char* walked = ax_fs_walk(path);
  char* result = ax_lines_stat_json_array(walked, 0, 0);
  free(walked);
  return result;
}

char* ax_fs_find(const char* path, const char* needle) {
  const char* query = needle == 0 ? "" : needle;
  char* walked = ax_fs_walk(path);
  if (walked == 0) {
    return ax_strdup("");
  }
  if (*query == 0) {
    return walked;
  }

  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  char* cursor = walked;
  while (*cursor != 0) {
    char* line = cursor;
    while (*cursor != 0 && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0) {
      char* item = ax_slice_copy(line, line_len);
      if (item != 0 && strstr(item, query) != 0) {
        ax_append_text(&buffer, &len, &cap, item);
        if (buffer == 0) {
          free(item);
          break;
        }
      }
      free(item);
    }
    if (*cursor == '\n') {
      cursor++;
    }
  }
  free(walked);
  return buffer == 0 ? ax_strdup("") : buffer;
}

static int ax_glob_matches(const char* pattern, const char* value) {
  if (pattern == 0 || value == 0) {
    return 0;
  }
  while (*pattern != 0) {
    if (*pattern == '*') {
      while (*(pattern + 1) == '*') {
        pattern++;
      }
      pattern++;
      if (*pattern == 0) {
        return 1;
      }
      while (*value != 0) {
        if (ax_glob_matches(pattern, value)) {
          return 1;
        }
        value++;
      }
      return ax_glob_matches(pattern, value);
    }
    if (*pattern == '?') {
      if (*value == 0) {
        return 0;
      }
      pattern++;
      value++;
      continue;
    }
    if (*pattern != *value) {
      return 0;
    }
    pattern++;
    value++;
  }
  return *value == 0 ? 1 : 0;
}

static const char* ax_path_basename_ptr(const char* path) {
  if (path == 0) {
    return "";
  }
  const char* base = path;
  for (const char* cursor = path; *cursor != 0; cursor++) {
    if (ax_fs_is_path_separator(*cursor)) {
      base = cursor + 1;
    }
  }
  return base;
}

char* ax_fs_glob(const char* path, const char* pattern) {
  const char* query = pattern == 0 ? "" : pattern;
  char* walked = ax_fs_walk(path);
  if (walked == 0) {
    return ax_strdup("");
  }
  if (*query == 0) {
    return walked;
  }

  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  char* cursor = walked;
  while (*cursor != 0) {
    char* line = cursor;
    while (*cursor != 0 && *cursor != '\n') {
      cursor++;
    }
    size_t line_len = (size_t)(cursor - line);
    if (line_len > 0) {
      char* item = ax_slice_copy(line, line_len);
      const char* base = ax_path_basename_ptr(item);
      if (item != 0 && (ax_glob_matches(query, item) || ax_glob_matches(query, base))) {
        ax_append_text(&buffer, &len, &cap, item);
        if (buffer == 0) {
          free(item);
          break;
        }
      }
      free(item);
    }
    if (*cursor == '\n') {
      cursor++;
    }
  }
  free(walked);
  return buffer == 0 ? ax_strdup("") : buffer;
}

void ax_fs_remove_dir(const char* path) {
  if (ax_fs_is_dangerous_remove_dir_target(path)) {
    return;
  }
#ifdef _WIN32
  size_t base_len = strlen(path);
  int needs_sep = base_len > 0 && path[base_len - 1] != '\\' && path[base_len - 1] != '/';
  char* pattern = (char*)malloc(base_len + (needs_sep ? 3 : 2));
  if (pattern == 0) {
    return;
  }
  memcpy(pattern, path, base_len);
  size_t offset = base_len;
  if (needs_sep) {
    pattern[offset++] = '\\';
  }
  pattern[offset++] = '*';
  pattern[offset] = 0;

  WIN32_FIND_DATAA data;
  HANDLE handle = FindFirstFileA(pattern, &data);
  free(pattern);
  if (handle != INVALID_HANDLE_VALUE) {
    do {
      if (strcmp(data.cFileName, ".") == 0 || strcmp(data.cFileName, "..") == 0) {
        continue;
      }
      size_t name_len = strlen(data.cFileName);
      char* child = (char*)malloc(base_len + (needs_sep ? 1 : 0) + name_len + 1);
      if (child == 0) {
        continue;
      }
      memcpy(child, path, base_len);
      offset = base_len;
      if (needs_sep) {
        child[offset++] = '\\';
      }
      memcpy(child + offset, data.cFileName, name_len + 1);
      if ((data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
        ax_fs_remove_dir(child);
      } else {
        DeleteFileA(child);
      }
      free(child);
    } while (FindNextFileA(handle, &data));
    FindClose(handle);
  }
  RemoveDirectoryA(path);
#else
  DIR* dir = opendir(path);
  if (dir != 0) {
    struct dirent* entry = 0;
    while ((entry = readdir(dir)) != 0) {
      if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
        continue;
      }
      size_t base_len = strlen(path);
      size_t name_len = strlen(entry->d_name);
      int needs_sep = base_len > 0 && path[base_len - 1] != '/';
      char* child = (char*)malloc(base_len + (needs_sep ? 1 : 0) + name_len + 1);
      if (child == 0) {
        continue;
      }
      memcpy(child, path, base_len);
      size_t offset = base_len;
      if (needs_sep) {
        child[offset++] = '/';
      }
      memcpy(child + offset, entry->d_name, name_len + 1);
      struct stat info;
      if (lstat(child, &info) == 0 && S_ISDIR(info.st_mode)) {
        ax_fs_remove_dir(child);
      } else {
        remove(child);
      }
      free(child);
    }
    closedir(dir);
  }
  rmdir(path);
#endif
}

char* ax_env_get(const char* name) {
  if (name == 0) {
    return ax_strdup("");
  }
  const char* value = getenv(name);
  return ax_strdup(value == 0 ? "" : value);
}

int ax_env_has(const char* name) {
  if (name == 0) {
    return 0;
  }
  return getenv(name) == 0 ? 0 : 1;
}

void ax_env_set(const char* name, const char* value) {
  if (name == 0) {
    return;
  }
#ifdef _WIN32
  _putenv_s(name, value == 0 ? "" : value);
#else
  setenv(name, value == 0 ? "" : value, 1);
#endif
}

char* ax_env_get_or(const char* name, const char* fallback) {
  if (name == 0) {
    return ax_strdup(fallback == 0 ? "" : fallback);
  }
  const char* value = getenv(name);
  return ax_strdup(value == 0 ? (fallback == 0 ? "" : fallback) : value);
}

static void ax_env_snapshot_append_pair(char** fields, size_t* len, size_t* cap, int* count, const char* name, const char* value) {
  char* pair = ax_json_string_pair(name, value);
  if (pair == 0) {
    return;
  }
  if (*count > 0) {
    ax_append_bytes(fields, len, cap, ",", 1);
  }
  ax_append_bytes(fields, len, cap, pair, strlen(pair));
  free(pair);
  (*count)++;
}

char* ax_env_snapshot_json(const char* prefix) {
  const char* want = prefix == 0 ? "" : prefix;
  size_t want_len = strlen(want);
  if (want_len == 0) {
    return ax_strdup("{}");
  }
  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
#ifdef _WIN32
  LPCH block = GetEnvironmentStringsA();
  if (block == 0) {
    return ax_strdup("{}");
  }
  for (const char* entry = block; *entry != 0; entry += strlen(entry) + 1) {
    const char* equals = strchr(entry, '=');
    if (equals == 0 || equals == entry) {
      continue;
    }
    size_t name_len = (size_t)(equals - entry);
    if (name_len < want_len || strncmp(entry, want, want_len) != 0) {
      continue;
    }
    char* name = ax_slice_copy(entry, name_len);
    ax_env_snapshot_append_pair(&fields, &len, &cap, &count, name, equals + 1);
    free(name);
    if (fields == 0 && count > 0) {
      break;
    }
  }
  FreeEnvironmentStringsA(block);
#else
  extern char** environ;
  for (char** cursor = environ; cursor != 0 && *cursor != 0; cursor++) {
    const char* entry = *cursor;
    const char* equals = strchr(entry, '=');
    if (equals == 0 || equals == entry) {
      continue;
    }
    size_t name_len = (size_t)(equals - entry);
    if (name_len < want_len || strncmp(entry, want, want_len) != 0) {
      continue;
    }
    char* name = ax_slice_copy(entry, name_len);
    ax_env_snapshot_append_pair(&fields, &len, &cap, &count, name, equals + 1);
    free(name);
    if (fields == 0 && count > 0) {
      break;
    }
  }
#endif
  char* result = ax_json_object(fields == 0 ? "" : fields);
  free(fields);
  return result;
}

static char* ax_trim_slice_copy(const char* start, const char* end) {
  if (start == 0 || end == 0 || end < start) {
    return ax_strdup("");
  }
  while (start < end && isspace((unsigned char)*start)) {
    start++;
  }
  while (end > start && isspace((unsigned char)*(end - 1))) {
    end--;
  }
  return ax_slice_copy(start, (size_t)(end - start));
}

static int ax_env_valid_name(const char* name) {
  if (name == 0 || *name == 0) {
    return 0;
  }
  unsigned char first = (unsigned char)name[0];
  if (!(isalpha(first) || first == '_')) {
    return 0;
  }
  for (const char* p = name + 1; *p != 0; p++) {
    unsigned char ch = (unsigned char)*p;
    if (!(isalnum(ch) || ch == '_')) {
      return 0;
    }
  }
  return 1;
}

static char* ax_env_dotenv_value(const char* start, const char* end) {
  char* trimmed = ax_trim_slice_copy(start, end);
  if (trimmed == 0) {
    return ax_strdup("");
  }
  size_t len = strlen(trimmed);
  if (len >= 2 &&
      ((trimmed[0] == '"' && trimmed[len - 1] == '"') ||
       (trimmed[0] == '\'' && trimmed[len - 1] == '\''))) {
    char* unquoted = ax_slice_copy(trimmed + 1, len - 2);
    free(trimmed);
    return unquoted;
  }
  return trimmed;
}

static int ax_env_load_dotenv_text(const char* text, char** fields, size_t* len, size_t* cap) {
  if (text == 0) {
    return 0;
  }
  int count = 0;
  const char* cursor = text;
  while (*cursor != 0) {
    const char* line = cursor;
    while (*cursor != 0 && *cursor != '\n') {
      cursor++;
    }
    const char* line_end = cursor;
    if (line_end > line && *(line_end - 1) == '\r') {
      line_end--;
    }
    if (*cursor == '\n') {
      cursor++;
    }

    const char* start = line;
    while (start < line_end && isspace((unsigned char)*start)) {
      start++;
    }
    if (start >= line_end || *start == '#') {
      continue;
    }
    if ((size_t)(line_end - start) > 6 && strncmp(start, "export", 6) == 0 &&
        isspace((unsigned char)start[6])) {
      start += 6;
      while (start < line_end && isspace((unsigned char)*start)) {
        start++;
      }
    }

    const char* equals = start;
    while (equals < line_end && *equals != '=') {
      equals++;
    }
    if (equals >= line_end) {
      continue;
    }
    char* name = ax_trim_slice_copy(start, equals);
    if (!ax_env_valid_name(name)) {
      free(name);
      continue;
    }
    char* value = ax_env_dotenv_value(equals + 1, line_end);
    ax_env_set(name, value);
    if (fields != 0 && len != 0 && cap != 0) {
      char* pair = ax_json_string_pair(name, value);
      if (pair != 0) {
        if (count > 0) {
          ax_append_bytes(fields, len, cap, ",", 1);
        }
        ax_append_bytes(fields, len, cap, pair, strlen(pair));
        free(pair);
      }
    }
    free(name);
    free(value);
    count++;
  }
  return count;
}

int ax_env_load_dotenv(const char* path) {
  char* text = ax_fs_read_text(path);
  int count = ax_env_load_dotenv_text(text, 0, 0, 0);
  free(text);
  return count;
}

char* ax_env_load_dotenv_json(const char* path) {
  char* text = ax_fs_read_text(path);
  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  ax_env_load_dotenv_text(text, &fields, &len, &cap);
  char* result = ax_json_object(fields == 0 ? "" : fields);
  free(text);
  free(fields);
  return result == 0 ? ax_strdup("{}") : result;
}

static int ax_process_normalize_status(int status) {
  if (status < 0) {
    return 1;
  }
#ifdef _WIN32
  return status;
#else
  if (WIFEXITED(status)) {
    return WEXITSTATUS(status);
  }
  if (WIFSIGNALED(status)) {
    return 128 + WTERMSIG(status);
  }
  return status;
#endif
}

static char* ax_process_exec_capture_with_status(const char* command, int max_bytes, int* out_status, int* out_truncated) {
  if (out_status != 0) {
    *out_status = 1;
  }
  if (out_truncated != 0) {
    *out_truncated = 0;
  }
  if (command == 0) {
    return ax_strdup("");
  }
  FILE* pipe = ax_popen(command, "r");
  if (pipe == 0) {
    return ax_strdup("");
  }
  char chunk[512];
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  size_t limit = max_bytes < 0 ? (size_t)-1 : (size_t)max_bytes;
  size_t read_len = 0;
  while ((read_len = fread(chunk, 1, sizeof(chunk), pipe)) > 0) {
    size_t copy_len = 0;
    if (len < limit) {
      size_t remaining = limit - len;
      copy_len = read_len < remaining ? read_len : remaining;
      if (copy_len > 0 && !ax_append_bytes(&buffer, &len, &cap, chunk, copy_len)) {
        len = limit;
      }
    }
    if (max_bytes >= 0 && out_truncated != 0 && copy_len < read_len) {
      *out_truncated = 1;
    }
    if (max_bytes >= 0 && out_truncated != 0 && len >= limit && read_len > 0 && copy_len == 0) {
      *out_truncated = 1;
    }
  }
  int status = ax_pclose(pipe);
  if (out_status != 0) {
    *out_status = ax_process_normalize_status(status);
  }
  if (buffer == 0) {
    return ax_strdup("");
  }
  return buffer;
}

static char* ax_process_exec_capture(const char* command, int max_bytes) {
  return ax_process_exec_capture_with_status(command, max_bytes, 0, 0);
}

char* ax_process_exec(const char* command) {
  return ax_process_exec_capture(command, -1);
}

char* ax_process_exec_limit(const char* command, int max_bytes) {
  if (max_bytes <= 0) {
    return ax_strdup("");
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  return ax_process_exec_capture(command, max_bytes);
}

int ax_process_status(const char* command) {
  if (command == 0) {
    return 1;
  }
  int status = system(command);
  if (status < 0) {
    return 1;
  }
  return ax_process_normalize_status(status);
}

static char* ax_process_stderr_merge_command(const char* command) {
  const char* source = command == 0 ? "" : command;
  size_t len = strlen(source);
  char* merged = (char*)malloc(len + 8);
  if (merged == 0) {
    return ax_strdup("");
  }
  snprintf(merged, len + 8, "(%s) 2>&1", source);
  return merged;
}

static char* ax_process_run_output_json(const char* command, int max_bytes, const char* output_key, int merge_stderr) {
  if (max_bytes < 0) {
    max_bytes = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  int status = 1;
  int truncated = 0;
  char* merged = merge_stderr ? ax_process_stderr_merge_command(command) : 0;
  const char* run_command = merge_stderr ? merged : command;
  char* output = ax_process_exec_capture_with_status(run_command, max_bytes, &status, &truncated);
  char status_json[32];
  snprintf(status_json, sizeof(status_json), "%d", status);

  char* ok_pair = ax_json_pair("ok", status == 0 ? "true" : "false");
  char* status_pair = ax_json_pair("status", status_json);
  char* output_pair = ax_json_string_pair(output_key, output);
  char* truncated_pair = ax_json_pair("truncated", truncated ? "true" : "false");
  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  ax_append_bytes(&fields, &len, &cap, ok_pair, strlen(ok_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, status_pair, strlen(status_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, output_pair, strlen(output_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, truncated_pair, strlen(truncated_pair));

  char* result = fields == 0 ? ax_strdup("{}") : ax_json_object(fields);
  free(merged);
  free(output);
  free(ok_pair);
  free(status_pair);
  free(output_pair);
  free(truncated_pair);
  free(fields);
  return result;
}

char* ax_process_run_json(const char* command, int max_bytes) {
  return ax_process_run_output_json(command, max_bytes, "stdout", 0);
}

char* ax_process_run_log_json(const char* command, int max_bytes) {
  return ax_process_run_output_json(command, max_bytes, "output", 1);
}

static char* ax_process_run_lines_output_json(const char* command, int max_bytes, int merge_stderr) {
  if (max_bytes < 0) {
    max_bytes = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  int status = 1;
  int truncated = 0;
  char* merged = merge_stderr ? ax_process_stderr_merge_command(command) : 0;
  const char* run_command = merge_stderr ? merged : command;
  char* output = ax_process_exec_capture_with_status(run_command, max_bytes, &status, &truncated);
  char* lines_json = ax_text_lines_json_array(output);
  char status_json[32];
  snprintf(status_json, sizeof(status_json), "%d", status);

  char* ok_pair = ax_json_pair("ok", status == 0 ? "true" : "false");
  char* status_pair = ax_json_pair("status", status_json);
  char* lines_pair = ax_json_pair("lines", lines_json);
  char* truncated_pair = ax_json_pair("truncated", truncated ? "true" : "false");
  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  ax_append_bytes(&fields, &len, &cap, ok_pair, strlen(ok_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, status_pair, strlen(status_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, lines_pair, strlen(lines_pair));
  ax_append_bytes(&fields, &len, &cap, ",", 1);
  ax_append_bytes(&fields, &len, &cap, truncated_pair, strlen(truncated_pair));

  char* result = fields == 0 ? ax_strdup("{}") : ax_json_object(fields);
  free(merged);
  free(output);
  free(lines_json);
  free(ok_pair);
  free(status_pair);
  free(lines_pair);
  free(truncated_pair);
  free(fields);
  return result;
}

char* ax_process_run_lines_json(const char* command, int max_bytes) {
  return ax_process_run_lines_output_json(command, max_bytes, 0);
}

char* ax_process_run_log_lines_json(const char* command, int max_bytes) {
  return ax_process_run_lines_output_json(command, max_bytes, 1);
}

static const char* ax_json_skip_ws(const char* p) {
  while (p != 0 && *p != 0 && isspace((unsigned char)*p)) {
    p++;
  }
  return p;
}

static const char* ax_json_skip_value(const char* p);

static const char* ax_json_skip_string(const char* p) {
  if (p == 0 || *p != '"') {
    return 0;
  }
  p++;
  while (*p != 0) {
    if (*p == '\\') {
      p++;
      if (*p == 0) {
        return 0;
      }
      p++;
      continue;
    }
    if (*p == '"') {
      return p + 1;
    }
    if ((unsigned char)*p < 0x20) {
      return 0;
    }
    p++;
  }
  return 0;
}

static const char* ax_json_skip_literal(const char* p, const char* literal) {
  size_t len = strlen(literal);
  return strncmp(p, literal, len) == 0 ? p + len : 0;
}

static const char* ax_json_skip_number(const char* p) {
  if (*p == '-') {
    p++;
  }
  if (!isdigit((unsigned char)*p)) {
    return 0;
  }
  if (*p == '0') {
    p++;
  } else {
    while (isdigit((unsigned char)*p)) {
      p++;
    }
  }
  if (*p == '.') {
    p++;
    if (!isdigit((unsigned char)*p)) {
      return 0;
    }
    while (isdigit((unsigned char)*p)) {
      p++;
    }
  }
  if (*p == 'e' || *p == 'E') {
    p++;
    if (*p == '+' || *p == '-') {
      p++;
    }
    if (!isdigit((unsigned char)*p)) {
      return 0;
    }
    while (isdigit((unsigned char)*p)) {
      p++;
    }
  }
  return p;
}

static const char* ax_json_skip_array(const char* p) {
  if (*p != '[') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  if (*p == ']') {
    return p + 1;
  }
  for (;;) {
    p = ax_json_skip_value(p);
    if (p == 0) {
      return 0;
    }
    p = ax_json_skip_ws(p);
    if (*p == ']') {
      return p + 1;
    }
    if (*p != ',') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
  }
}

static const char* ax_json_skip_object(const char* p) {
  if (*p != '{') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  if (*p == '}') {
    return p + 1;
  }
  for (;;) {
    p = ax_json_skip_string(p);
    if (p == 0) {
      return 0;
    }
    p = ax_json_skip_ws(p);
    if (*p != ':') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
    p = ax_json_skip_value(p);
    if (p == 0) {
      return 0;
    }
    p = ax_json_skip_ws(p);
    if (*p == '}') {
      return p + 1;
    }
    if (*p != ',') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
  }
}

static const char* ax_json_skip_value(const char* p) {
  p = ax_json_skip_ws(p);
  if (p == 0 || *p == 0) {
    return 0;
  }
  if (*p == '"') {
    return ax_json_skip_string(p);
  }
  if (*p == '{') {
    return ax_json_skip_object(p);
  }
  if (*p == '[') {
    return ax_json_skip_array(p);
  }
  if (*p == 't') {
    return ax_json_skip_literal(p, "true");
  }
  if (*p == 'f') {
    return ax_json_skip_literal(p, "false");
  }
  if (*p == 'n') {
    return ax_json_skip_literal(p, "null");
  }
  return ax_json_skip_number(p);
}

char* ax_json_escape(const char* value) {
  if (value == 0) {
    return ax_strdup("");
  }
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  char escaped[7];
  for (const unsigned char* p = (const unsigned char*)value; *p != 0; p++) {
    switch (*p) {
      case '"':
        ax_append_bytes(&buffer, &len, &cap, "\\\"", 2);
        break;
      case '\\':
        ax_append_bytes(&buffer, &len, &cap, "\\\\", 2);
        break;
      case '\b':
        ax_append_bytes(&buffer, &len, &cap, "\\b", 2);
        break;
      case '\f':
        ax_append_bytes(&buffer, &len, &cap, "\\f", 2);
        break;
      case '\n':
        ax_append_bytes(&buffer, &len, &cap, "\\n", 2);
        break;
      case '\r':
        ax_append_bytes(&buffer, &len, &cap, "\\r", 2);
        break;
      case '\t':
        ax_append_bytes(&buffer, &len, &cap, "\\t", 2);
        break;
      default:
        if (*p < 0x20) {
          snprintf(escaped, sizeof(escaped), "\\u%04x", *p);
          ax_append_bytes(&buffer, &len, &cap, escaped, 6);
        } else {
          ax_append_bytes(&buffer, &len, &cap, (const char*)p, 1);
        }
        break;
    }
    if (buffer == 0) {
      return ax_strdup("");
    }
  }
  return buffer == 0 ? ax_strdup("") : buffer;
}

char* ax_json_quote(const char* value) {
  char* escaped = ax_json_escape(value);
  if (escaped == 0) {
    return ax_strdup("\"\"");
  }
  size_t escaped_len = strlen(escaped);
  char* out = (char*)malloc(escaped_len + 3);
  if (out == 0) {
    free(escaped);
    return ax_strdup("\"\"");
  }
  out[0] = '"';
  memcpy(out + 1, escaped, escaped_len);
  out[escaped_len + 1] = '"';
  out[escaped_len + 2] = 0;
  free(escaped);
  return out;
}

static char* ax_json_join3(const char* left, const char* middle, const char* right) {
  const char* a = left == 0 ? "" : left;
  const char* b = middle == 0 ? "" : middle;
  const char* c = right == 0 ? "" : right;
  size_t a_len = strlen(a);
  size_t b_len = strlen(b);
  size_t c_len = strlen(c);
  char* out = (char*)malloc(a_len + b_len + c_len + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  memcpy(out, a, a_len);
  memcpy(out + a_len, b, b_len);
  memcpy(out + a_len + b_len, c, c_len + 1);
  return out;
}

char* ax_json_pair(const char* key, const char* value_json) {
  char* quoted_key = ax_json_quote(key);
  char* value = ax_json_valid(value_json) ? ax_json_compact(value_json) : ax_strdup("null");
  if (quoted_key == 0 || value == 0) {
    free(quoted_key);
    free(value);
    return ax_strdup("\"\":null");
  }
  char* prefix = ax_json_join3(quoted_key, ":", value);
  free(quoted_key);
  free(value);
  return prefix;
}

char* ax_json_string_pair(const char* key, const char* value) {
  char* quoted_value = ax_json_quote(value);
  char* pair = ax_json_pair(key, quoted_value);
  free(quoted_value);
  return pair;
}

static char* ax_json_unquote(const char* p, const char** out_end);

static int ax_json_append_field(char** fields, size_t* len, size_t* cap, int* count, const char* field, size_t field_len) {
  if (field == 0) {
    return 1;
  }
  if (*count > 0 && !ax_append_bytes(fields, len, cap, ",", 1)) {
    return 0;
  }
  if (!ax_append_bytes(fields, len, cap, field, field_len)) {
    return 0;
  }
  (*count)++;
  return 1;
}

char* ax_json_set(const char* json, const char* key, const char* value_json) {
  char* replacement = ax_json_pair(key, value_json);
  if (replacement == 0) {
    replacement = ax_strdup("\"\":null");
  }
  char* base = ax_json_valid(json) ? ax_json_compact(json) : ax_strdup("{}");
  if (base == 0) {
    free(replacement);
    return ax_strdup("{}");
  }
  const char* p = ax_json_skip_ws(base);
  if (p == 0 || *p != '{') {
    free(base);
    free(replacement);
    return ax_strdup("{}");
  }

  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  int replaced = 0;
  p = ax_json_skip_ws(p + 1);
  while (*p != 0 && *p != '}') {
    const char* field_start = p;
    const char* key_end = 0;
    char* parsed_key = ax_json_unquote(p, &key_end);
    if (key_end == 0) {
      free(parsed_key);
      free(fields);
      free(base);
      free(replacement);
      return ax_strdup("{}");
    }
    p = ax_json_skip_ws(key_end);
    if (*p != ':') {
      free(parsed_key);
      free(fields);
      free(base);
      free(replacement);
      return ax_strdup("{}");
    }
    p = ax_json_skip_ws(p + 1);
    const char* value_end = ax_json_skip_value(p);
    if (value_end == 0) {
      free(parsed_key);
      free(fields);
      free(base);
      free(replacement);
      return ax_strdup("{}");
    }
    if (strcmp(parsed_key, key == 0 ? "" : key) == 0) {
      if (!replaced &&
          !ax_json_append_field(&fields, &len, &cap, &count, replacement, strlen(replacement))) {
        free(parsed_key);
        free(base);
        free(replacement);
        return ax_strdup("{}");
      }
      replaced = 1;
    } else if (!ax_json_append_field(&fields,
                                     &len,
                                     &cap,
                                     &count,
                                     field_start,
                                     (size_t)(value_end - field_start))) {
      free(parsed_key);
      free(base);
      free(replacement);
      return ax_strdup("{}");
    }
    free(parsed_key);
    p = ax_json_skip_ws(value_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == '}') {
      break;
    }
    free(fields);
    free(base);
    free(replacement);
    return ax_strdup("{}");
  }
  if (!replaced && !ax_json_append_field(&fields, &len, &cap, &count, replacement, strlen(replacement))) {
    free(base);
    free(replacement);
    return ax_strdup("{}");
  }

  char* result = ax_json_object(fields == 0 ? "" : fields);
  free(fields);
  free(base);
  free(replacement);
  return result == 0 ? ax_strdup("{}") : result;
}

char* ax_json_remove(const char* json, const char* key) {
  char* base = ax_json_valid(json) ? ax_json_compact(json) : ax_strdup("{}");
  if (base == 0) {
    return ax_strdup("{}");
  }
  const char* p = ax_json_skip_ws(base);
  if (p == 0 || *p != '{') {
    free(base);
    return ax_strdup("{}");
  }

  char* fields = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const char* target = key == 0 ? "" : key;
  p = ax_json_skip_ws(p + 1);
  while (*p != 0 && *p != '}') {
    const char* field_start = p;
    const char* key_end = 0;
    char* parsed_key = ax_json_unquote(p, &key_end);
    if (key_end == 0) {
      free(parsed_key);
      free(fields);
      free(base);
      return ax_strdup("{}");
    }
    p = ax_json_skip_ws(key_end);
    if (*p != ':') {
      free(parsed_key);
      free(fields);
      free(base);
      return ax_strdup("{}");
    }
    p = ax_json_skip_ws(p + 1);
    const char* value_end = ax_json_skip_value(p);
    if (value_end == 0) {
      free(parsed_key);
      free(fields);
      free(base);
      return ax_strdup("{}");
    }
    if (strcmp(parsed_key, target) != 0 &&
        !ax_json_append_field(&fields,
                              &len,
                              &cap,
                              &count,
                              field_start,
                              (size_t)(value_end - field_start))) {
      free(parsed_key);
      free(fields);
      free(base);
      return ax_strdup("{}");
    }
    free(parsed_key);
    p = ax_json_skip_ws(value_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == '}') {
      break;
    }
    free(fields);
    free(base);
    return ax_strdup("{}");
  }

  char* result = ax_json_object(fields == 0 ? "" : fields);
  free(fields);
  free(base);
  return result == 0 ? ax_strdup("{}") : result;
}

char* ax_json_string_set(const char* json, const char* key, const char* value) {
  char* quoted_value = ax_json_quote(value);
  char* result = ax_json_set(json, key, quoted_value);
  free(quoted_value);
  return result;
}

char* ax_json_object(const char* fields) {
  char* wrapped = ax_json_join3("{", fields == 0 ? "" : fields, "}");
  if (wrapped == 0) {
    return ax_strdup("{}");
  }
  char* out = ax_json_valid(wrapped) ? ax_json_compact(wrapped) : ax_strdup("{}");
  free(wrapped);
  return out;
}

char* ax_json_array(const char* items) {
  char* wrapped = ax_json_join3("[", items == 0 ? "" : items, "]");
  if (wrapped == 0) {
    return ax_strdup("[]");
  }
  char* out = ax_json_valid(wrapped) ? ax_json_compact(wrapped) : ax_strdup("[]");
  free(wrapped);
  return out;
}

char* ax_json_array_push(const char* json, const char* value_json) {
  char* value = ax_json_valid(value_json) ? ax_json_compact(value_json) : ax_strdup("null");
  if (value == 0) {
    value = ax_strdup("null");
  }
  char* base = ax_json_valid(json) ? ax_json_compact(json) : ax_strdup("[]");
  if (base == 0) {
    free(value);
    return ax_strdup("[]");
  }
  size_t base_len = strlen(base);
  if (base_len < 2 || base[0] != '[' || base[base_len - 1] != ']') {
    free(base);
    base = ax_strdup("[]");
    base_len = 2;
  }

  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int ok = 1;
  if (base_len > 2) {
    ok = ok && ax_append_bytes(&items, &len, &cap, base + 1, base_len - 2);
    ok = ok && ax_append_bytes(&items, &len, &cap, ",", 1);
  }
  ok = ok && ax_append_bytes(&items, &len, &cap, value, strlen(value));

  char* result = ok ? ax_json_array(items == 0 ? "" : items) : ax_strdup("[]");
  free(items);
  free(base);
  free(value);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_json_string_array_push(const char* json, const char* value) {
  char* quoted_value = ax_json_quote(value);
  char* result = ax_json_array_push(json, quoted_value);
  free(quoted_value);
  return result;
}

char* ax_json_compact(const char* json) {
  if (json == 0) {
    return ax_strdup("");
  }
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  int in_string = 0;
  int escaped = 0;
  for (const char* p = json; *p != 0; p++) {
    if (!in_string && isspace((unsigned char)*p)) {
      continue;
    }
    ax_append_bytes(&buffer, &len, &cap, p, 1);
    if (buffer == 0) {
      return ax_strdup("");
    }
    if (in_string) {
      if (escaped) {
        escaped = 0;
      } else if (*p == '\\') {
        escaped = 1;
      } else if (*p == '"') {
        in_string = 0;
      }
    } else if (*p == '"') {
      in_string = 1;
    }
  }
  return buffer == 0 ? ax_strdup("") : buffer;
}

int ax_json_valid(const char* json) {
  if (json == 0) {
    return 0;
  }
  const char* end = ax_json_skip_value(json);
  if (end == 0) {
    return 0;
  }
  end = ax_json_skip_ws(end);
  return *end == 0 ? 1 : 0;
}

static char* ax_json_unquote(const char* p, const char** out_end) {
  if (out_end != 0) {
    *out_end = 0;
  }
  if (p == 0 || *p != '"') {
    return ax_strdup("");
  }
  p++;
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  while (*p != 0) {
    if (*p == '"') {
      if (out_end != 0) {
        *out_end = p + 1;
      }
      return buffer == 0 ? ax_strdup("") : buffer;
    }
    if (*p == '\\') {
      p++;
      if (*p == 0) {
        break;
      }
      char value = *p;
      switch (*p) {
        case '"':
        case '\\':
        case '/':
          value = *p;
          break;
        case 'b':
          value = '\b';
          break;
        case 'f':
          value = '\f';
          break;
        case 'n':
          value = '\n';
          break;
        case 'r':
          value = '\r';
          break;
        case 't':
          value = '\t';
          break;
        default:
          value = *p;
          break;
      }
      ax_append_bytes(&buffer, &len, &cap, &value, 1);
      p++;
      continue;
    }
    ax_append_bytes(&buffer, &len, &cap, p, 1);
    p++;
  }
  free(buffer);
  return ax_strdup("");
}

static char* ax_json_slice_compact(const char* start, const char* end) {
  if (start == 0 || end == 0 || end < start) {
    return ax_strdup("");
  }
  size_t len = (size_t)(end - start);
  char* raw = (char*)malloc(len + 1);
  if (raw == 0) {
    return ax_strdup("");
  }
  memcpy(raw, start, len);
  raw[len] = 0;
  char* compact = ax_json_compact(raw);
  free(raw);
  return compact;
}

static int ax_json_find_top_property(const char* json, const char* key, const char** value_start, const char** value_end) {
  if (value_start != 0) {
    *value_start = 0;
  }
  if (value_end != 0) {
    *value_end = 0;
  }
  if (json == 0 || key == 0) {
    return 0;
  }
  const char* p = ax_json_skip_ws(json);
  if (*p != '{') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  while (*p != 0 && *p != '}') {
    const char* key_end = 0;
    char* parsed_key = ax_json_unquote(p, &key_end);
    if (key_end == 0) {
      free(parsed_key);
      return 0;
    }
    p = ax_json_skip_ws(key_end);
    if (*p != ':') {
      free(parsed_key);
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
    const char* current_value_start = p;
    const char* current_value_end = ax_json_skip_value(p);
    if (current_value_end == 0) {
      free(parsed_key);
      return 0;
    }
    if (strcmp(parsed_key, key) == 0) {
      free(parsed_key);
      if (value_start != 0) {
        *value_start = current_value_start;
      }
      if (value_end != 0) {
        *value_end = current_value_end;
      }
      return 1;
    }
    free(parsed_key);
    p = ax_json_skip_ws(current_value_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == '}') {
      break;
    }
    return 0;
  }
  return 0;
}

static int ax_json_parse_path_index(const char* segment, size_t len, int* out) {
  if (out != 0) {
    *out = 0;
  }
  if (segment == 0 || len == 0) {
    return 0;
  }
  long long value = 0;
  for (size_t i = 0; i < len; i++) {
    if (!isdigit((unsigned char)segment[i])) {
      return 0;
    }
    value = value * 10 + (long long)(segment[i] - '0');
    if (value > INT_MAX) {
      return 0;
    }
  }
  if (out != 0) {
    *out = (int)value;
  }
  return 1;
}

static int ax_json_find_array_index(const char* array_start, int index, const char** value_start, const char** value_end) {
  if (value_start != 0) {
    *value_start = 0;
  }
  if (value_end != 0) {
    *value_end = 0;
  }
  if (array_start == 0 || index < 0) {
    return 0;
  }
  const char* p = ax_json_skip_ws(array_start);
  if (p == 0 || *p != '[') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  int current = 0;
  while (*p != 0 && *p != ']') {
    const char* item_start = p;
    const char* item_end = ax_json_skip_value(p);
    if (item_end == 0) {
      return 0;
    }
    if (current == index) {
      if (value_start != 0) {
        *value_start = item_start;
      }
      if (value_end != 0) {
        *value_end = item_end;
      }
      return 1;
    }
    current++;
    p = ax_json_skip_ws(item_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == ']') {
      break;
    }
    return 0;
  }
  return 0;
}

static int ax_json_find_path(const char* json, const char* path, const char** value_start, const char** value_end) {
  if (value_start != 0) {
    *value_start = 0;
  }
  if (value_end != 0) {
    *value_end = 0;
  }
  if (json == 0 || path == 0 || *path == 0) {
    return 0;
  }
  const char* current_start = ax_json_skip_ws(json);
  const char* current_end = ax_json_skip_value(current_start);
  if (current_start == 0 || current_end == 0) {
    return 0;
  }
  const char* segment = path;
  while (*segment != 0) {
    const char* next = segment;
    while (*next != 0 && *next != '.') {
      next++;
    }
    size_t segment_len = (size_t)(next - segment);
    if (segment_len == 0) {
      return 0;
    }
    current_start = ax_json_skip_ws(current_start);
    if (current_start == 0 || current_end == 0) {
      return 0;
    }
    if (*current_start == '{') {
      char* key = ax_slice_copy(segment, segment_len);
      int found = ax_json_find_top_property(current_start, key, &current_start, &current_end);
      free(key);
      if (!found) {
        return 0;
      }
    } else if (*current_start == '[') {
      int index = 0;
      if (!ax_json_parse_path_index(segment, segment_len, &index)) {
        return 0;
      }
      if (!ax_json_find_array_index(current_start, index, &current_start, &current_end)) {
        return 0;
      }
    } else {
      return 0;
    }
    if (*next == '.') {
      if (*(next + 1) == 0) {
        return 0;
      }
      segment = next + 1;
      continue;
    }
    break;
  }
  if (value_start != 0) {
    *value_start = current_start;
  }
  if (value_end != 0) {
    *value_end = current_end;
  }
  return 1;
}

char* ax_json_get(const char* json, const char* key) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return ax_strdup("");
  }
  if (*value_start == '"') {
    return ax_json_unquote(value_start, 0);
  }
  return ax_json_slice_compact(value_start, value_end);
}

char* ax_json_query(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return ax_strdup("");
  }
  if (*value_start == '"') {
    return ax_json_unquote(value_start, 0);
  }
  return ax_json_slice_compact(value_start, value_end);
}

char* ax_json_get_or(const char* json, const char* key, const char* fallback) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return ax_strdup(fallback == 0 ? "" : fallback);
  }
  if (*value_start == '"') {
    return ax_json_unquote(value_start, 0);
  }
  return ax_json_slice_compact(value_start, value_end);
}

char* ax_json_query_or(const char* json, const char* path, const char* fallback) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return ax_strdup(fallback == 0 ? "" : fallback);
  }
  if (*value_start == '"') {
    return ax_json_unquote(value_start, 0);
  }
  return ax_json_slice_compact(value_start, value_end);
}

int ax_json_has(const char* json, const char* key) {
  return ax_json_find_top_property(json, key, 0, 0);
}

int ax_json_query_has(const char* json, const char* path) {
  return ax_json_find_path(json, path, 0, 0);
}

static int ax_json_parse_int_slice(const char* start, const char* end, int* out) {
  if (out != 0) {
    *out = 0;
  }
  if (start == 0 || end == 0 || end < start) {
    return 0;
  }
  start = ax_json_skip_ws(start);
  while (end > start && isspace((unsigned char)*(end - 1))) {
    end--;
  }
  if (start >= end) {
    return 0;
  }
  int negative = 0;
  if (*start == '-') {
    negative = 1;
    start++;
  }
  if (start >= end || !isdigit((unsigned char)*start)) {
    return 0;
  }
  long long value = 0;
  long long limit = negative ? (long long)INT_MAX + 1LL : (long long)INT_MAX;
  const char* p = start;
  while (p < end && isdigit((unsigned char)*p)) {
    value = value * 10 + (long long)(*p - '0');
    if (value > limit) {
      return 0;
    }
    p++;
  }
  if (p != end) {
    return 0;
  }
  if (out != 0) {
    if (negative && value == (long long)INT_MAX + 1LL) {
      *out = INT_MIN;
    } else {
      *out = negative ? -(int)value : (int)value;
    }
  }
  return 1;
}

int ax_json_int(const char* json, const char* key) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return 0;
  }
  int value = 0;
  return ax_json_parse_int_slice(value_start, value_end, &value) ? value : 0;
}

int ax_json_query_int(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return 0;
  }
  int value = 0;
  return ax_json_parse_int_slice(value_start, value_end, &value) ? value : 0;
}

static int ax_json_parse_bool_slice(const char* value_start, const char* value_end) {
  value_start = ax_json_skip_ws(value_start);
  while (value_end > value_start && isspace((unsigned char)*(value_end - 1))) {
    value_end--;
  }
  return value_end - value_start == 4 && strncmp(value_start, "true", 4) == 0;
}

int ax_json_bool(const char* json, const char* key) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return 0;
  }
  return ax_json_parse_bool_slice(value_start, value_end);
}

int ax_json_query_bool(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return 0;
  }
  return ax_json_parse_bool_slice(value_start, value_end);
}

static int ax_json_array_contains_value(const char* array_start, const char* needle) {
  if (needle == 0) {
    return 0;
  }
  const char* p = ax_json_skip_ws(array_start);
  if (p == 0 || *p != '[') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  while (*p != 0 && *p != ']') {
    const char* item_start = p;
    const char* item_end = ax_json_skip_value(p);
    if (item_end == 0) {
      return 0;
    }
    int matched = 0;
    if (*item_start == '"') {
      char* item = ax_json_unquote(item_start, 0);
      matched = strcmp(item, needle) == 0;
      free(item);
    } else {
      char* item = ax_json_slice_compact(item_start, item_end);
      matched = strcmp(item, needle) == 0;
      free(item);
    }
    if (matched) {
      return 1;
    }
    p = ax_json_skip_ws(item_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == ']') {
      break;
    }
    return 0;
  }
  return 0;
}

int ax_json_contains(const char* json, const char* key, const char* needle) {
  const char* value_start = 0;
  if (!ax_json_find_top_property(json, key, &value_start, 0)) {
    return 0;
  }
  return ax_json_array_contains_value(value_start, needle);
}

int ax_json_query_contains(const char* json, const char* path, const char* needle) {
  const char* value_start = 0;
  if (!ax_json_find_path(json, path, &value_start, 0)) {
    return 0;
  }
  return ax_json_array_contains_value(value_start, needle);
}

static const char* ax_json_value_kind(const char* value_start, const char* value_end) {
  value_start = ax_json_skip_ws(value_start);
  if (value_start == 0 || value_end == 0) {
    return "invalid";
  }
  switch (*value_start) {
    case '"':
      return "string";
    case '{':
      return "object";
    case '[':
      return "array";
    case 't':
    case 'f':
      return "bool";
    case 'n':
      return "null";
    default:
      return "number";
  }
}

char* ax_json_kind(const char* json, const char* key) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return ax_strdup("missing");
  }
  return ax_strdup(ax_json_value_kind(value_start, value_end));
}

char* ax_json_query_kind(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return ax_strdup("missing");
  }
  return ax_strdup(ax_json_value_kind(value_start, value_end));
}

char* ax_json_keys(const char* json) {
  if (json == 0) {
    return ax_strdup("");
  }
  const char* p = ax_json_skip_ws(json);
  if (*p != '{') {
    return ax_strdup("");
  }
  p = ax_json_skip_ws(p + 1);
  char* buffer = 0;
  size_t len = 0;
  size_t cap = 0;
  while (*p != 0 && *p != '}') {
    const char* key_end = 0;
    char* parsed_key = ax_json_unquote(p, &key_end);
    if (key_end == 0) {
      free(parsed_key);
      free(buffer);
      return ax_strdup("");
    }
    ax_append_text(&buffer, &len, &cap, parsed_key);
    free(parsed_key);
    if (buffer == 0) {
      return ax_strdup("");
    }
    p = ax_json_skip_ws(key_end);
    if (*p != ':') {
      free(buffer);
      return ax_strdup("");
    }
    p = ax_json_skip_ws(p + 1);
    const char* current_value_end = ax_json_skip_value(p);
    if (current_value_end == 0) {
      free(buffer);
      return ax_strdup("");
    }
    p = ax_json_skip_ws(current_value_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == '}') {
      break;
    }
    free(buffer);
    return ax_strdup("");
  }
  return buffer == 0 ? ax_strdup("") : buffer;
}

static char* ax_json_object_keys_array(const char* object_start) {
  const char* p = ax_json_skip_ws(object_start);
  if (p == 0 || *p != '{') {
    return ax_strdup("[]");
  }
  p = ax_json_skip_ws(p + 1);
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  while (*p != 0 && *p != '}') {
    const char* key_end = 0;
    char* parsed_key = ax_json_unquote(p, &key_end);
    if (key_end == 0) {
      free(parsed_key);
      free(items);
      return ax_strdup("[]");
    }
    char* quoted_key = ax_json_quote(parsed_key);
    free(parsed_key);
    if (quoted_key == 0) {
      free(items);
      return ax_strdup("[]");
    }
    if (count > 0 && !ax_append_bytes(&items, &len, &cap, ",", 1)) {
      free(quoted_key);
      return ax_strdup("[]");
    }
    if (!ax_append_bytes(&items, &len, &cap, quoted_key, strlen(quoted_key))) {
      free(quoted_key);
      return ax_strdup("[]");
    }
    free(quoted_key);
    count++;
    p = ax_json_skip_ws(key_end);
    if (*p != ':') {
      free(items);
      return ax_strdup("[]");
    }
    p = ax_json_skip_ws(p + 1);
    const char* current_value_end = ax_json_skip_value(p);
    if (current_value_end == 0) {
      free(items);
      return ax_strdup("[]");
    }
    p = ax_json_skip_ws(current_value_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == '}') {
      break;
    }
    free(items);
    return ax_strdup("[]");
  }
  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_json_keys_json(const char* json) {
  return ax_json_object_keys_array(json);
}

char* ax_json_query_keys_json(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return ax_strdup("[]");
  }
  (void)value_end;
  return ax_json_object_keys_array(value_start);
}

static int ax_json_array_len(const char* p) {
  p = ax_json_skip_ws(p);
  if (p == 0 || *p != '[') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  if (*p == ']') {
    return 0;
  }
  int count = 0;
  for (;;) {
    p = ax_json_skip_value(p);
    if (p == 0) {
      return 0;
    }
    count++;
    p = ax_json_skip_ws(p);
    if (*p == ']') {
      return count;
    }
    if (*p != ',') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
  }
}

static int ax_json_object_len(const char* p) {
  p = ax_json_skip_ws(p);
  if (p == 0 || *p != '{') {
    return 0;
  }
  p = ax_json_skip_ws(p + 1);
  if (*p == '}') {
    return 0;
  }
  int count = 0;
  for (;;) {
    p = ax_json_skip_string(p);
    if (p == 0) {
      return 0;
    }
    p = ax_json_skip_ws(p);
    if (*p != ':') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
    p = ax_json_skip_value(p);
    if (p == 0) {
      return 0;
    }
    count++;
    p = ax_json_skip_ws(p);
    if (*p == '}') {
      return count;
    }
    if (*p != ',') {
      return 0;
    }
    p = ax_json_skip_ws(p + 1);
  }
}

static int ax_json_value_len(const char* value_start, const char* value_end) {
  value_start = ax_json_skip_ws(value_start);
  if (value_start == 0 || value_end == 0) {
    return 0;
  }
  if (*value_start == '[') {
    return ax_json_array_len(value_start);
  }
  if (*value_start == '{') {
    return ax_json_object_len(value_start);
  }
  if (*value_start == '"') {
    char* text = ax_json_unquote(value_start, 0);
    int len = (int)strlen(text);
    free(text);
    return len;
  }
  return 0;
}

int ax_json_len(const char* json, const char* key) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_top_property(json, key, &value_start, &value_end)) {
    return 0;
  }
  return ax_json_value_len(value_start, value_end);
}

int ax_json_query_len(const char* json, const char* path) {
  const char* value_start = 0;
  const char* value_end = 0;
  if (!ax_json_find_path(json, path, &value_start, &value_end)) {
    return 0;
  }
  return ax_json_value_len(value_start, value_end);
}

static char* ax_json_array_at_value(const char* array_start, int index) {
  if (index < 0) {
    return ax_strdup("");
  }
  const char* p = ax_json_skip_ws(array_start);
  if (p == 0 || *p != '[') {
    return ax_strdup("");
  }
  p = ax_json_skip_ws(p + 1);
  int current = 0;
  while (*p != 0 && *p != ']') {
    const char* item_start = p;
    const char* item_end = ax_json_skip_value(p);
    if (item_end == 0) {
      return ax_strdup("");
    }
    if (current == index) {
      if (*item_start == '"') {
        return ax_json_unquote(item_start, 0);
      }
      return ax_json_slice_compact(item_start, item_end);
    }
    current++;
    p = ax_json_skip_ws(item_end);
    if (*p == ',') {
      p = ax_json_skip_ws(p + 1);
      continue;
    }
    if (*p == ']') {
      break;
    }
    return ax_strdup("");
  }
  return ax_strdup("");
}

char* ax_json_at(const char* json, const char* key, int index) {
  const char* value_start = 0;
  if (!ax_json_find_top_property(json, key, &value_start, 0)) {
    return ax_strdup("");
  }
  return ax_json_array_at_value(value_start, index);
}

char* ax_json_query_at(const char* json, const char* path, int index) {
  const char* value_start = 0;
  if (!ax_json_find_path(json, path, &value_start, 0)) {
    return ax_strdup("");
  }
  return ax_json_array_at_value(value_start, index);
}

int ax_str_len(const char* value) {
  if (value == 0) {
    return 0;
  }
  static AX_THREAD_LOCAL const char* cached_value = 0;
  static AX_THREAD_LOCAL int cached_len = 0;
  if (cached_value == value) {
    return cached_len;
  }
  cached_value = value;
  cached_len = (int)strlen(value);
  return cached_len;
}

char* ax_str_from_i64(long long value) {
  char buffer[32];
  int len = snprintf(buffer, sizeof(buffer), "%lld", value);
  if (len < 0) {
    return ax_strdup("0");
  }
  return ax_slice_copy(buffer, (size_t)len);
}

long long ax_str_parse_i64(const char* value) {
  const char* text = value == 0 ? "" : value;
  char* end = 0;
  errno = 0;
  long long parsed = strtoll(text, &end, 10);
  if (errno != 0 || end == text) {
    return 0;
  }
  while (*end != 0) {
    if (!isspace((unsigned char)*end)) {
      return 0;
    }
    end++;
  }
  return parsed;
}

int ax_str_parse_i32(const char* value) {
  long long parsed = ax_str_parse_i64(value);
  if (parsed > 2147483647ll) {
    return 2147483647;
  }
  if (parsed < -2147483647ll - 1ll) {
    return -2147483647 - 1;
  }
  return (int)parsed;
}

char* ax_str_token(const char* value, int index) {
  if (index < 0) {
    return ax_strdup("");
  }
  const char* cursor = value == 0 ? "" : value;
  int current = 0;
  while (*cursor != 0) {
    while (*cursor != 0 && isspace((unsigned char)*cursor)) {
      cursor++;
    }
    if (*cursor == 0) {
      break;
    }
    const char* start = cursor;
    while (*cursor != 0 && !isspace((unsigned char)*cursor)) {
      cursor++;
    }
    if (current == index) {
      return ax_slice_copy(start, (size_t)(cursor - start));
    }
    current++;
  }
  return ax_strdup("");
}

char* ax_str_token_upper(const char* value, int index) {
  if (index < 0) {
    return ax_strdup("");
  }
  const char* cursor = value == 0 ? "" : value;
  int current = 0;
  while (*cursor != 0) {
    while (*cursor != 0 && isspace((unsigned char)*cursor)) {
      cursor++;
    }
    if (*cursor == 0) {
      break;
    }
    const char* start = cursor;
    while (*cursor != 0 && !isspace((unsigned char)*cursor)) {
      cursor++;
    }
    if (current == index) {
      size_t len = (size_t)(cursor - start);
      char* out = ax_slice_copy(start, len);
      if (out == 0) {
        return ax_strdup("");
      }
      for (size_t i = 0; i < len; i++) {
        out[i] = (char)toupper((unsigned char)out[i]);
      }
      return out;
    }
    current++;
  }
  return ax_strdup("");
}

char* ax_str_line(const char* value, int index) {
  if (index < 0) {
    return ax_strdup("");
  }
  const char* cursor = value == 0 ? "" : value;
  const char* start = cursor;
  int current = 0;
  for (;;) {
    if (*cursor == '\n' || *cursor == 0) {
      if (current == index) {
        const char* end = cursor;
        if (end > start && end[-1] == '\r') {
          end--;
        }
        return ax_slice_copy(start, (size_t)(end - start));
      }
      if (*cursor == 0) {
        break;
      }
      cursor++;
      start = cursor;
      current++;
      continue;
    }
    cursor++;
  }
  return ax_strdup("");
}

int ax_str_eq(const char* left, const char* right) {
  const char* a = left == 0 ? "" : left;
  const char* b = right == 0 ? "" : right;
  return strcmp(a, b) == 0 ? 1 : 0;
}

typedef struct {
  const char* value;
  const char* needle;
  int op;
  int result;
} AxStrPredicateCache;

static AX_THREAD_LOCAL AxStrPredicateCache ax_str_predicate_cache[16];
static AX_THREAD_LOCAL int ax_str_predicate_cache_count = 0;
static AX_THREAD_LOCAL int ax_str_predicate_cache_next = 0;
static AX_THREAD_LOCAL const char* ax_str_last_value[3] = {0, 0, 0};
static AX_THREAD_LOCAL const char* ax_str_last_needle[3] = {0, 0, 0};
static AX_THREAD_LOCAL int ax_str_last_result[3] = {0, 0, 0};

static int ax_str_cached_predicate(const char* value, const char* needle, int op) {
  if (op >= 0 && op < 3 && ax_str_last_value[op] == value && ax_str_last_needle[op] == needle) {
    return ax_str_last_result[op];
  }
  for (int i = 0; i < ax_str_predicate_cache_count; i++) {
    AxStrPredicateCache* entry = &ax_str_predicate_cache[i];
    if (entry->value == value && entry->needle == needle && entry->op == op) {
      if (op >= 0 && op < 3) {
        ax_str_last_value[op] = value;
        ax_str_last_needle[op] = needle;
        ax_str_last_result[op] = entry->result;
      }
      return entry->result;
    }
  }

  const char* haystack = value == 0 ? "" : value;
  const char* pattern = needle == 0 ? "" : needle;
  int result = 0;
  if (op == 0) {
    result = strstr(haystack, pattern) != 0 ? 1 : 0;
  } else if (op == 1) {
    size_t pattern_len = strlen(pattern);
    result = strncmp(haystack, pattern, pattern_len) == 0 ? 1 : 0;
  } else {
    size_t haystack_len = strlen(haystack);
    size_t pattern_len = strlen(pattern);
    result = pattern_len <= haystack_len && strcmp(haystack + haystack_len - pattern_len, pattern) == 0 ? 1 : 0;
  }

  int slot = ax_str_predicate_cache_count;
  if (slot < 16) {
    ax_str_predicate_cache_count++;
  } else {
    slot = ax_str_predicate_cache_next;
    ax_str_predicate_cache_next = (ax_str_predicate_cache_next + 1) % 16;
  }
  ax_str_predicate_cache[slot].value = value;
  ax_str_predicate_cache[slot].needle = needle;
  ax_str_predicate_cache[slot].op = op;
  ax_str_predicate_cache[slot].result = result;
  if (op >= 0 && op < 3) {
    ax_str_last_value[op] = value;
    ax_str_last_needle[op] = needle;
    ax_str_last_result[op] = result;
  }
  return result;
}

int ax_str_contains(const char* value, const char* needle) {
  return ax_str_cached_predicate(value, needle, 0);
}

int ax_str_index_of(const char* value, const char* needle) {
  const char* haystack = value == 0 ? "" : value;
  const char* pattern = needle == 0 ? "" : needle;
  const char* match = strstr(haystack, pattern);
  if (match == 0) {
    return -1;
  }
  return (int)(match - haystack);
}

int ax_str_count(const char* value, const char* needle) {
  const char* haystack = value == 0 ? "" : value;
  const char* pattern = needle == 0 ? "" : needle;
  size_t pattern_len = strlen(pattern);
  if (pattern_len == 0) {
    return 0;
  }
  int count = 0;
  const char* scan = haystack;
  const char* match = 0;
  while ((match = strstr(scan, pattern)) != 0) {
    count++;
    scan = match + pattern_len;
  }
  return count;
}

int ax_str_starts_with(const char* value, const char* prefix) {
  return ax_str_cached_predicate(value, prefix, 1);
}

int ax_str_ends_with(const char* value, const char* suffix) {
  return ax_str_cached_predicate(value, suffix, 2);
}

char* ax_str_trim(const char* value) {
  const char* text = value == 0 ? "" : value;
  while (*text != 0 && isspace((unsigned char)*text)) {
    text++;
  }
  const char* end = text + strlen(text);
  while (end > text && isspace((unsigned char)*(end - 1))) {
    end--;
  }
  return ax_slice_copy(text, (size_t)(end - text));
}

char* ax_str_upper(const char* value) {
  const char* text = value == 0 ? "" : value;
  size_t len = strlen(text);
  char* out = ax_slice_copy(text, len);
  for (size_t i = 0; i < len; i++) {
    out[i] = (char)toupper((unsigned char)out[i]);
  }
  return out;
}

char* ax_str_lower(const char* value) {
  const char* text = value == 0 ? "" : value;
  size_t len = strlen(text);
  char* out = ax_slice_copy(text, len);
  for (size_t i = 0; i < len; i++) {
    out[i] = (char)tolower((unsigned char)out[i]);
  }
  return out;
}

char* ax_str_concat(const char* left, const char* right) {
  const char* a = left == 0 ? "" : left;
  const char* b = right == 0 ? "" : right;
  size_t a_len = strlen(a);
  size_t b_len = strlen(b);
  char* out = (char*)malloc(a_len + b_len + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  memcpy(out, a, a_len);
  memcpy(out + a_len, b, b_len + 1);
  return out;
}

char* ax_str_repeat(const char* value, int count) {
  const char* text = value == 0 ? "" : value;
  if (count <= 0 || *text == 0) {
    return ax_strdup("");
  }
  size_t len = strlen(text);
  size_t total = len * (size_t)count;
  char* out = (char*)malloc(total + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  char* cursor = out;
  for (int i = 0; i < count; i++) {
    memcpy(cursor, text, len);
    cursor += len;
  }
  out[total] = 0;
  return out;
}

char* ax_str_replace(const char* value, const char* needle, const char* replacement) {
  const char* text = value == 0 ? "" : value;
  const char* old = needle == 0 ? "" : needle;
  const char* new_value = replacement == 0 ? "" : replacement;
  size_t old_len = strlen(old);
  if (old_len == 0) {
    return ax_strdup(text);
  }
  size_t new_len = strlen(new_value);
  size_t count = 0;
  const char* scan = text;
  while ((scan = strstr(scan, old)) != 0) {
    count++;
    scan += old_len;
  }
  size_t text_len = strlen(text);
  size_t out_len = text_len;
  if (new_len >= old_len) {
    out_len += count * (new_len - old_len);
  } else {
    out_len -= count * (old_len - new_len);
  }
  char* out = (char*)malloc(out_len + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  char* cursor = out;
  scan = text;
  const char* match = 0;
  while ((match = strstr(scan, old)) != 0) {
    size_t prefix_len = (size_t)(match - scan);
    memcpy(cursor, scan, prefix_len);
    cursor += prefix_len;
    memcpy(cursor, new_value, new_len);
    cursor += new_len;
    scan = match + old_len;
  }
  strcpy(cursor, scan);
  return out;
}

char* ax_str_slice(const char* value, int start, int length) {
  const char* text = value == 0 ? "" : value;
  if (start < 0) {
    start = 0;
  }
  if (length <= 0) {
    return ax_strdup("");
  }
  size_t text_len = strlen(text);
  size_t begin = (size_t)start;
  if (begin >= text_len) {
    return ax_strdup("");
  }
  size_t available = text_len - begin;
  size_t requested = (size_t)length;
  if (requested > available) {
    requested = available;
  }
  return ax_slice_copy(text + begin, requested);
}

static int ax_str_append_split_part(char** items,
                                    size_t* len,
                                    size_t* cap,
                                    const char* start,
                                    size_t part_len,
                                    int count) {
  const size_t output_limit = 1024 * 1024;
  char* part = ax_slice_copy(start, part_len);
  char* quoted = ax_json_quote(part);
  free(part);
  if (quoted == 0) {
    quoted = ax_strdup("\"\"");
  }
  if (quoted == 0) {
    return 0;
  }
  size_t quoted_len = strlen(quoted);
  size_t comma_len = count > 0 ? 1 : 0;
  if (*len + comma_len + quoted_len > output_limit) {
    free(quoted);
    return 0;
  }
  if (count > 0 && !ax_append_bytes(items, len, cap, ",", 1)) {
    free(quoted);
    return 0;
  }
  if (!ax_append_bytes(items, len, cap, quoted, quoted_len)) {
    free(quoted);
    return 0;
  }
  free(quoted);
  return 1;
}

char* ax_str_split_json(const char* value, const char* delimiter) {
  const char* text = value == 0 ? "" : value;
  const char* delim = delimiter == 0 ? "" : delimiter;
  size_t delim_len = strlen(delim);
  char* items = 0;
  size_t len = 0;
  size_t cap = 0;
  int count = 0;
  const int part_limit = 10000;
  if (delim_len == 0) {
    if (ax_str_append_split_part(&items, &len, &cap, text, strlen(text), count)) {
      count++;
    }
    char* result = ax_json_array(items == 0 ? "" : items);
    free(items);
    return result == 0 ? ax_strdup("[]") : result;
  }

  const char* cursor = text;
  while (count < part_limit) {
    const char* match = strstr(cursor, delim);
    size_t part_len = match == 0 ? strlen(cursor) : (size_t)(match - cursor);
    if (!ax_str_append_split_part(&items, &len, &cap, cursor, part_len, count)) {
      break;
    }
    count++;
    if (match == 0) {
      break;
    }
    cursor = match + delim_len;
  }

  char* result = ax_json_array(items == 0 ? "" : items);
  free(items);
  return result == 0 ? ax_strdup("[]") : result;
}

char* ax_str_lines_json(const char* value) {
  return ax_text_lines_json_array(value == 0 ? "" : value);
}

static int ax_path_is_sep(char ch) {
  return ch == '/' || ch == '\\';
}

static int ax_path_has_drive(const char* value) {
  return value != 0 && isalpha((unsigned char)value[0]) && value[1] == ':';
}

int ax_path_is_absolute(const char* value) {
  if (value == 0 || *value == 0) {
    return 0;
  }
  if (ax_path_is_sep(value[0])) {
    return 1;
  }
  return ax_path_has_drive(value) && ax_path_is_sep(value[2]) ? 1 : 0;
}

static int ax_path_has_parent_segment(const char* text, size_t len) {
  size_t i = 0;
  while (i < len) {
    while (i < len && ax_path_is_sep(text[i])) {
      i++;
    }
    size_t start = i;
    while (i < len && !ax_path_is_sep(text[i])) {
      i++;
    }
    if (i - start == 2 && text[start] == '.' && text[start + 1] == '.') {
      return 1;
    }
  }
  return 0;
}

static char* ax_path_normalize_no_parent(const char* text, size_t len) {
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return ax_strdup(".");
  }
  size_t i = 0;
  size_t w = 0;
  if (ax_path_has_drive(text)) {
    out[w++] = text[0];
    out[w++] = ':';
    i = 2;
    if (i < len && ax_path_is_sep(text[i])) {
      out[w++] = '/';
      while (i < len && ax_path_is_sep(text[i])) {
        i++;
      }
    }
  } else if (len > 0 && ax_path_is_sep(text[0])) {
    out[w++] = '/';
    while (i < len && ax_path_is_sep(text[i])) {
      i++;
    }
  }

  while (i < len) {
    while (i < len && ax_path_is_sep(text[i])) {
      i++;
    }
    size_t start = i;
    while (i < len && !ax_path_is_sep(text[i])) {
      i++;
    }
    size_t seg_len = i - start;
    if (seg_len == 0 || (seg_len == 1 && text[start] == '.')) {
      continue;
    }
    if (w > 0 && out[w - 1] != '/') {
      out[w++] = '/';
    }
    for (size_t j = 0; j < seg_len; j++) {
      char ch = text[start + j];
      out[w++] = ch == '\\' ? '/' : ch;
    }
  }

  if (w == 0) {
    free(out);
    return ax_strdup(".");
  }
  out[w] = 0;
  return out;
}

char* ax_path_normalize(const char* value) {
  const char* text = value == 0 ? "" : value;
  size_t len = strlen(text);
  if (len == 0) {
    return ax_strdup(".");
  }
  if (!ax_path_has_parent_segment(text, len)) {
    return ax_path_normalize_no_parent(text, len);
  }
  char* scratch = ax_slice_copy(text, len);
  for (size_t i = 0; i < len; i++) {
    if (scratch[i] == '\\') {
      scratch[i] = '/';
    }
  }

  size_t prefix_len = 0;
  int absolute = 0;
  if (ax_path_has_drive(scratch)) {
    prefix_len = 2;
    absolute = scratch[2] == '/';
  } else {
    absolute = scratch[0] == '/';
  }

  char** segments = (char**)calloc(len + 1, sizeof(char*));
  size_t* segment_lens = (size_t*)calloc(len + 1, sizeof(size_t));
  if (segments == 0 || segment_lens == 0) {
    free(segments);
    free(segment_lens);
    free(scratch);
    return ax_strdup(".");
  }

  size_t count = 0;
  size_t i = prefix_len + (absolute ? 1 : 0);
  while (i < len) {
    while (i < len && scratch[i] == '/') {
      i++;
    }
    size_t start = i;
    while (i < len && scratch[i] != '/') {
      i++;
    }
    size_t seg_len = i - start;
    if (seg_len == 0 || (seg_len == 1 && scratch[start] == '.')) {
      continue;
    }
    if (seg_len == 2 && scratch[start] == '.' && scratch[start + 1] == '.') {
      if (count > 0 && !(segment_lens[count - 1] == 2 && strncmp(segments[count - 1], "..", 2) == 0)) {
        count--;
      } else if (!absolute) {
        segments[count] = scratch + start;
        segment_lens[count] = seg_len;
        count++;
      }
      continue;
    }
    segments[count] = scratch + start;
    segment_lens[count] = seg_len;
    count++;
  }

  char* out = 0;
  size_t out_len = 0;
  size_t out_cap = 0;
  if (prefix_len > 0) {
    ax_append_bytes(&out, &out_len, &out_cap, scratch, prefix_len);
  }
  if (absolute) {
    ax_append_bytes(&out, &out_len, &out_cap, "/", 1);
  }
  for (size_t idx = 0; idx < count; idx++) {
    if ((absolute || prefix_len > 0 || idx > 0) && out_len > 0 && out[out_len - 1] != '/') {
      ax_append_bytes(&out, &out_len, &out_cap, "/", 1);
    }
    ax_append_bytes(&out, &out_len, &out_cap, segments[idx], segment_lens[idx]);
  }
  if (out == 0 || out_len == 0) {
    free(out);
    out = ax_strdup(absolute ? "/" : ".");
  }
  free(segments);
  free(segment_lens);
  free(scratch);
  return out;
}

char* ax_path_join(const char* left, const char* right) {
  const char* a = left == 0 ? "" : left;
  const char* b = right == 0 ? "" : right;
  if (ax_path_is_absolute(b)) {
    return ax_path_normalize(b);
  }
  char* prefix = ax_str_concat(a, "/");
  char* joined = ax_str_concat(prefix, b);
  char* normalized = ax_path_normalize(joined);
  free(prefix);
  free(joined);
  return normalized;
}

static const char* ax_path_tail_start(const char* value) {
  const char* text = value == 0 ? "" : value;
  const char* tail = text;
  for (const char* p = text; *p != 0; p++) {
    if (ax_path_is_sep(*p)) {
      tail = p + 1;
    }
  }
  return tail;
}

char* ax_path_basename(const char* value) {
  char* normalized = ax_path_normalize(value);
  size_t len = strlen(normalized);
  while (len > 1 && normalized[len - 1] == '/') {
    normalized[--len] = 0;
  }
  const char* tail = ax_path_tail_start(normalized);
  char* out = ax_strdup(*tail == 0 ? normalized : tail);
  free(normalized);
  return out;
}

char* ax_path_dirname(const char* value) {
  char* normalized = ax_path_normalize(value);
  size_t len = strlen(normalized);
  while (len > 1 && normalized[len - 1] == '/') {
    normalized[--len] = 0;
  }
  char* last = 0;
  for (char* p = normalized; *p != 0; p++) {
    if (*p == '/') {
      last = p;
    }
  }
  if (last == 0) {
    free(normalized);
    return ax_strdup(".");
  }
  if (last == normalized || (ax_path_has_drive(normalized) && last == normalized + 2)) {
    last[1] = 0;
    return normalized;
  }
  *last = 0;
  char* out = ax_strdup(normalized);
  free(normalized);
  return out;
}

char* ax_path_extname(const char* value) {
  char* base = ax_path_basename(value);
  char* dot = strrchr(base, '.');
  if (dot == 0 || dot == base) {
    free(base);
    return ax_strdup("");
  }
  char* out = ax_strdup(dot);
  free(base);
  return out;
}

char* ax_path_stem(const char* value) {
  char* base = ax_path_basename(value);
  char* dot = strrchr(base, '.');
  if (dot != 0 && dot != base) {
    *dot = 0;
  }
  return base;
}

static int ax_url_is_unreserved(unsigned char ch) {
  return (ch >= 'A' && ch <= 'Z') ||
         (ch >= 'a' && ch <= 'z') ||
         (ch >= '0' && ch <= '9') ||
         ch == '-' ||
         ch == '.' ||
         ch == '_' ||
         ch == '~';
}

static char ax_url_hex_digit(unsigned char value) {
  return value < 10 ? (char)('0' + value) : (char)('A' + value - 10);
}

static int ax_url_hex_value(char ch) {
  if (ch >= '0' && ch <= '9') {
    return ch - '0';
  }
  if (ch >= 'a' && ch <= 'f') {
    return ch - 'a' + 10;
  }
  if (ch >= 'A' && ch <= 'F') {
    return ch - 'A' + 10;
  }
  return -1;
}

char* ax_url_encode(const char* value) {
  const unsigned char* text = (const unsigned char*)(value == 0 ? "" : value);
  size_t len = strlen((const char*)text);
  char* out = (char*)malloc((len * 3) + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  size_t out_len = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = text[i];
    if (ax_url_is_unreserved(ch)) {
      out[out_len++] = (char)ch;
    } else {
      out[out_len++] = '%';
      out[out_len++] = ax_url_hex_digit((unsigned char)(ch >> 4));
      out[out_len++] = ax_url_hex_digit((unsigned char)(ch & 15));
    }
  }
  out[out_len] = 0;
  return out;
}

static char* ax_url_decode_slice(const char* value, size_t len, int plus_to_space) {
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return ax_strdup("");
  }
  size_t out_len = 0;
  for (size_t i = 0; i < len; i++) {
    if (value[i] == '%' && i + 2 < len) {
      int hi = ax_url_hex_value(value[i + 1]);
      int lo = ax_url_hex_value(value[i + 2]);
      if (hi >= 0 && lo >= 0) {
        out[out_len++] = (char)((hi << 4) | lo);
        i += 2;
        continue;
      }
    }
    out[out_len++] = plus_to_space && value[i] == '+' ? ' ' : value[i];
  }
  out[out_len] = 0;
  return out;
}

char* ax_url_decode(const char* value) {
  const char* text = value == 0 ? "" : value;
  return ax_url_decode_slice(text, strlen(text), 1);
}

static const char* ax_url_authority_start(const char* text) {
  const char* scheme = strstr(text, "://");
  if (scheme != 0) {
    return scheme + 3;
  }
  if (text[0] == '/' && text[1] == '/') {
    return text + 2;
  }
  return text;
}

static const char* ax_url_authority_end(const char* start) {
  const char* end = start;
  while (*end != 0 && *end != '/' && *end != '?' && *end != '#') {
    end++;
  }
  return end;
}

char* ax_url_scheme(const char* value) {
  const char* text = value == 0 ? "" : value;
  const char* scheme_end = strstr(text, "://");
  if (scheme_end == 0 || scheme_end == text) {
    return ax_strdup("");
  }
  return ax_slice_copy(text, (size_t)(scheme_end - text));
}

char* ax_url_host(const char* value) {
  const char* text = value == 0 ? "" : value;
  const char* start = ax_url_authority_start(text);
  const char* end = ax_url_authority_end(start);
  if (start == end || *start == '/' || *start == '?' || *start == '#') {
    return ax_strdup("");
  }
  for (const char* p = start; p < end; p++) {
    if (*p == '@') {
      start = p + 1;
    }
  }
  const char* host_end = end;
  if (*start == '[') {
    for (const char* p = start; p < end; p++) {
      if (*p == ']') {
        host_end = p + 1;
        break;
      }
    }
  } else {
    for (const char* p = start; p < end; p++) {
      if (*p == ':') {
        host_end = p;
        break;
      }
    }
  }
  return ax_slice_copy(start, (size_t)(host_end - start));
}

char* ax_url_path(const char* value) {
  const char* text = value == 0 ? "" : value;
  const char* start = ax_url_authority_start(text);
  const char* path_start = strchr(start, '/');
  const char* query = strchr(start, '?');
  const char* fragment = strchr(start, '#');
  if (path_start == 0 || (query != 0 && query < path_start) || (fragment != 0 && fragment < path_start)) {
    return ax_strdup("/");
  }
  const char* end = path_start;
  while (*end != 0 && *end != '?' && *end != '#') {
    end++;
  }
  return ax_slice_copy(path_start, (size_t)(end - path_start));
}

static char* ax_url_query_find(const char* value, const char* key, int* found) {
  if (found != 0) {
    *found = 0;
  }
  const char* text = value == 0 ? "" : value;
  const char* wanted = key == 0 ? "" : key;
  const char* query = strchr(text, '?');
  const char* cursor = query == 0 ? text : query + 1;
  if (*cursor == '?') {
    cursor++;
  }
  while (*cursor != 0 && *cursor != '#') {
    const char* key_start = cursor;
    while (*cursor != 0 && *cursor != '=' && *cursor != '&' && *cursor != '#') {
      cursor++;
    }
    size_t key_len = (size_t)(cursor - key_start);
    const char* value_start = cursor;
    size_t value_len = 0;
    if (*cursor == '=') {
      cursor++;
      value_start = cursor;
      while (*cursor != 0 && *cursor != '&' && *cursor != '#') {
        cursor++;
      }
      value_len = (size_t)(cursor - value_start);
    }
    char* decoded_key = ax_url_decode_slice(key_start, key_len, 1);
    int matched = strcmp(decoded_key, wanted) == 0;
    free(decoded_key);
    if (matched) {
      if (found != 0) {
        *found = 1;
      }
      return ax_url_decode_slice(value_start, value_len, 1);
    }
    if (*cursor == '&') {
      cursor++;
    }
  }
  return ax_strdup("");
}

char* ax_url_query_get(const char* value, const char* key) {
  return ax_url_query_find(value, key, 0);
}

char* ax_url_query_or(const char* value, const char* key, const char* fallback) {
  int found = 0;
  char* result = ax_url_query_find(value, key, &found);
  if (found) {
    return result;
  }
  free(result);
  return ax_strdup(fallback == 0 ? "" : fallback);
}

int ax_url_query_has(const char* value, const char* key) {
  int found = 0;
  char* result = ax_url_query_find(value, key, &found);
  free(result);
  return found;
}

char* ax_url_query_json(const char* value) {
  const char* text = value == 0 ? "" : value;
  const char* query = strchr(text, '?');
  if (query == 0 && strchr(text, '=') == 0) {
    return ax_strdup("{}");
  }
  const char* cursor = query == 0 ? text : query + 1;
  if (*cursor == '?') {
    cursor++;
  }

  char* fields = 0;
  size_t fields_len = 0;
  size_t fields_cap = 0;
  int emitted = 0;
  while (*cursor != 0 && *cursor != '#') {
    const char* key_start = cursor;
    while (*cursor != 0 && *cursor != '=' && *cursor != '&' && *cursor != '#') {
      cursor++;
    }
    size_t key_len = (size_t)(cursor - key_start);
    const char* value_start = cursor;
    size_t value_len = 0;
    if (*cursor == '=') {
      cursor++;
      value_start = cursor;
      while (*cursor != 0 && *cursor != '&' && *cursor != '#') {
        cursor++;
      }
      value_len = (size_t)(cursor - value_start);
    }
    if (key_len > 0) {
      char* decoded_key = ax_url_decode_slice(key_start, key_len, 1);
      char* decoded_value = ax_url_decode_slice(value_start, value_len, 1);
      char* pair = ax_json_string_pair(decoded_key, decoded_value);
      if (emitted) {
        ax_append_bytes(&fields, &fields_len, &fields_cap, ",", 1);
      }
      ax_append_bytes(&fields, &fields_len, &fields_cap, pair, strlen(pair));
      emitted = 1;
      free(decoded_key);
      free(decoded_value);
      free(pair);
      if (fields == 0) {
        return ax_strdup("{}");
      }
    }
    if (*cursor == '&') {
      cursor++;
      continue;
    }
    if (*cursor != 0 && *cursor != '#') {
      cursor++;
    }
  }
  char* out = ax_json_object(fields == 0 ? "" : fields);
  free(fields);
  return out;
}

long long ax_time_now(void) {
  return (long long)time(0);
}

long long ax_time_now_ms(void) {
#ifdef _WIN32
  FILETIME file_time;
  GetSystemTimeAsFileTime(&file_time);
  ULARGE_INTEGER ticks;
  ticks.LowPart = file_time.dwLowDateTime;
  ticks.HighPart = file_time.dwHighDateTime;
  return (long long)((ticks.QuadPart - 116444736000000000ULL) / 10000ULL);
#else
  struct timeval value;
  if (gettimeofday(&value, 0) != 0) {
    return 0;
  }
  return ((long long)value.tv_sec * 1000LL) + ((long long)value.tv_usec / 1000LL);
#endif
}

char* ax_time_iso_utc(long long epoch_seconds) {
  time_t raw = (time_t)epoch_seconds;
  struct tm utc;
#ifdef _WIN32
  if (gmtime_s(&utc, &raw) != 0) {
    return ax_strdup("");
  }
#else
  if (gmtime_r(&raw, &utc) == 0) {
    return ax_strdup("");
  }
#endif
  char buffer[32];
  size_t len = strftime(buffer, sizeof(buffer), "%Y-%m-%dT%H:%M:%SZ", &utc);
  if (len == 0) {
    return ax_strdup("");
  }
  return ax_strdup(buffer);
}

void ax_time_sleep_ms(int milliseconds) {
  if (milliseconds <= 0) {
    return;
  }
#ifdef _WIN32
  Sleep((DWORD)milliseconds);
#else
  struct timespec duration;
  duration.tv_sec = milliseconds / 1000;
  duration.tv_nsec = (long)(milliseconds % 1000) * 1000000L;
  while (nanosleep(&duration, &duration) != 0 && errno == EINTR) {
  }
#endif
}

void* ax_heap_alloc(int size) {
  if (size <= 0) {
    return 0;
  }
  return calloc(1, (size_t)size);
}

void ax_heap_free(void* ptr) {
  free(ptr);
}

void ax_panic(const char* message) {
  fprintf(stderr, "panic: %s\n", message == 0 ? "panic" : message);
  exit(1);
}
