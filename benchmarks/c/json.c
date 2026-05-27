#include <ctype.h>
#include <stdlib.h>
#include <string.h>

static const char* PAYLOAD =
    "{ \"task\" : \"summarize\", \"ok\" : true, \"tokens\" : 2048, "
    "\"agent\" : { \"name\" : \"codex\", \"limits\" : { \"tokens\" : 2048 } }, "
    "\"steps\" : [{ \"name\" : \"read\" }, { \"name\" : \"verify\" }], "
    "\"tools\" : [\"read\", \"write\", \"verify\"] }";

static char* compact_json(const char* value) {
  size_t len = strlen(value);
  char* out = (char*)malloc(len + 1);
  size_t out_len = 0;
  int in_string = 0;
  int escaped = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = (unsigned char)value[i];
    if (in_string) {
      out[out_len++] = (char)ch;
      if (escaped) {
        escaped = 0;
      } else if (ch == '\\') {
        escaped = 1;
      } else if (ch == '"') {
        in_string = 0;
      }
    } else if (ch == '"') {
      in_string = 1;
      out[out_len++] = (char)ch;
    } else if (!isspace(ch)) {
      out[out_len++] = (char)ch;
    }
  }
  out[out_len] = 0;
  return out;
}

static int value_len_after(const char* value, const char* marker) {
  const char* start = strstr(value, marker);
  if (start == 0) return 0;
  start += strlen(marker);
  const char* end = strchr(start, '"');
  return end == 0 ? 0 : (int)(end - start);
}

static int int_after(const char* value, const char* marker) {
  const char* start = strstr(value, marker);
  if (start == 0) return 0;
  return atoi(start + strlen(marker));
}

static const char* array_segment(const char* value, const char* marker, const char** end_out) {
  const char* start = strstr(value, marker);
  if (start == 0) return 0;
  start += strlen(marker);
  const char* end = strchr(start, ']');
  if (end == 0) return 0;
  *end_out = end;
  return start;
}

static int array_len(const char* value, const char* marker) {
  const char* end = 0;
  const char* start = array_segment(value, marker, &end);
  if (start == 0 || start == end) return 0;
  int count = 1;
  for (const char* cursor = start; cursor < end; cursor++) {
    if (*cursor == ',') count += 1;
  }
  return count;
}

static int array_contains(const char* value, const char* marker, const char* needle) {
  const char* end = 0;
  const char* start = array_segment(value, marker, &end);
  if (start == 0) return 0;
  const char* found = strstr(start, needle);
  return found != 0 && found < end;
}

static int json_score(int n) {
  int i = 0;
  int acc = 0;
  while (i < n) {
    char* compact = compact_json(PAYLOAD);
    if (strstr(compact, "\"task\":") != 0 && strstr(compact, "\"ok\":true") != 0) {
      int task = value_len_after(compact, "\"task\":\"");
      int agent = value_len_after(compact, "\"agent\":{\"name\":\"");
      int first_step = value_len_after(compact, "\"steps\":[{\"name\":\"");
      int first_tool = value_len_after(compact, "\"tools\":[\"");
      int tokens = int_after(compact, "\"limits\":{\"tokens\":");
      int tool_count = array_len(compact, "\"tools\":[");
      if (tokens == 2048 && array_contains(compact, "\"tools\":[", "\"verify\"") &&
          strstr(compact, "\"steps\"") != 0 && strstr(compact, "\"limits\":{\"tokens\":2048}") != 0) {
        acc += task;
        acc += agent;
        acc += first_step;
        acc += (int)strlen("string");
        acc += (int)strlen("number");
        acc += first_tool;
        acc += tool_count;
        acc += tokens % 97;
      }
    }
    free(compact);
    i += 1;
  }
  return acc % 251;
}

int main(void) {
  return json_score(10000);
}
