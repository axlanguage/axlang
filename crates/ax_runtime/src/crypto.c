#include "ax_runtime.h"

#ifdef _WIN32
#ifndef _CRT_RAND_S
#define _CRT_RAND_S
#endif
#endif

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
  uint32_t state[8];
  uint64_t bit_len;
  uint8_t data[64];
  size_t data_len;
} ax_sha256;

static const uint32_t ax_sha256_k[64] = {
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
};

static uint32_t ax_rotr(uint32_t value, uint32_t bits) {
  return (value >> bits) | (value << (32 - bits));
}

static void ax_sha256_transform(ax_sha256* ctx, const uint8_t data[64]) {
  uint32_t m[64];
  for (int i = 0; i < 16; i++) {
    m[i] = ((uint32_t)data[i * 4] << 24) |
           ((uint32_t)data[i * 4 + 1] << 16) |
           ((uint32_t)data[i * 4 + 2] << 8) |
           ((uint32_t)data[i * 4 + 3]);
  }
  for (int i = 16; i < 64; i++) {
    uint32_t s0 = ax_rotr(m[i - 15], 7) ^ ax_rotr(m[i - 15], 18) ^ (m[i - 15] >> 3);
    uint32_t s1 = ax_rotr(m[i - 2], 17) ^ ax_rotr(m[i - 2], 19) ^ (m[i - 2] >> 10);
    m[i] = m[i - 16] + s0 + m[i - 7] + s1;
  }

  uint32_t a = ctx->state[0];
  uint32_t b = ctx->state[1];
  uint32_t c = ctx->state[2];
  uint32_t d = ctx->state[3];
  uint32_t e = ctx->state[4];
  uint32_t f = ctx->state[5];
  uint32_t g = ctx->state[6];
  uint32_t h = ctx->state[7];

  for (int i = 0; i < 64; i++) {
    uint32_t s1 = ax_rotr(e, 6) ^ ax_rotr(e, 11) ^ ax_rotr(e, 25);
    uint32_t ch = (e & f) ^ ((~e) & g);
    uint32_t temp1 = h + s1 + ch + ax_sha256_k[i] + m[i];
    uint32_t s0 = ax_rotr(a, 2) ^ ax_rotr(a, 13) ^ ax_rotr(a, 22);
    uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
    uint32_t temp2 = s0 + maj;
    h = g;
    g = f;
    f = e;
    e = d + temp1;
    d = c;
    c = b;
    b = a;
    a = temp1 + temp2;
  }

  ctx->state[0] += a;
  ctx->state[1] += b;
  ctx->state[2] += c;
  ctx->state[3] += d;
  ctx->state[4] += e;
  ctx->state[5] += f;
  ctx->state[6] += g;
  ctx->state[7] += h;
}

static void ax_sha256_init(ax_sha256* ctx) {
  ctx->data_len = 0;
  ctx->bit_len = 0;
  ctx->state[0] = 0x6a09e667;
  ctx->state[1] = 0xbb67ae85;
  ctx->state[2] = 0x3c6ef372;
  ctx->state[3] = 0xa54ff53a;
  ctx->state[4] = 0x510e527f;
  ctx->state[5] = 0x9b05688c;
  ctx->state[6] = 0x1f83d9ab;
  ctx->state[7] = 0x5be0cd19;
}

static void ax_sha256_update(ax_sha256* ctx, const uint8_t* data, size_t len) {
  for (size_t i = 0; i < len; i++) {
    ctx->data[ctx->data_len++] = data[i];
    if (ctx->data_len == 64) {
      ax_sha256_transform(ctx, ctx->data);
      ctx->bit_len += 512;
      ctx->data_len = 0;
    }
  }
}

static void ax_sha256_final(ax_sha256* ctx, uint8_t hash[32]) {
  size_t i = ctx->data_len;
  ctx->data[i++] = 0x80;
  if (i > 56) {
    while (i < 64) {
      ctx->data[i++] = 0;
    }
    ax_sha256_transform(ctx, ctx->data);
    i = 0;
  }
  while (i < 56) {
    ctx->data[i++] = 0;
  }

  ctx->bit_len += ctx->data_len * 8;
  for (int j = 0; j < 8; j++) {
    ctx->data[63 - j] = (uint8_t)(ctx->bit_len >> (j * 8));
  }
  ax_sha256_transform(ctx, ctx->data);

  for (int j = 0; j < 4; j++) {
    for (int k = 0; k < 8; k++) {
      hash[j + (k * 4)] = (uint8_t)(ctx->state[k] >> (24 - j * 8));
    }
  }
}

static char* ax_crypto_empty(void) {
  char* output = (char*)malloc(1);
  if (output != 0) {
    output[0] = 0;
  }
  return output;
}

static void ax_sha256_bytes(const uint8_t* input, size_t len, uint8_t hash[32]) {
  ax_sha256 ctx;
  ax_sha256_init(&ctx);
  if (input != 0 && len > 0) {
    ax_sha256_update(&ctx, input, len);
  }
  ax_sha256_final(&ctx, hash);
}

static char* ax_sha256_hex_from_hash(const uint8_t hash[32]) {
  static const char* hex = "0123456789abcdef";
  char* output = (char*)malloc(65);
  if (output == 0) {
    return 0;
  }
  for (int i = 0; i < 32; i++) {
    output[i * 2] = hex[hash[i] >> 4];
    output[i * 2 + 1] = hex[hash[i] & 0x0f];
  }
  output[64] = 0;
  return output;
}

static char* ax_bytes_to_hex(const uint8_t* bytes, size_t len) {
  static const char* hex = "0123456789abcdef";
  char* output = (char*)malloc(len * 2 + 1);
  if (output == 0) {
    return 0;
  }
  for (size_t i = 0; i < len; i++) {
    output[i * 2] = hex[bytes[i] >> 4];
    output[i * 2 + 1] = hex[bytes[i] & 0x0f];
  }
  output[len * 2] = 0;
  return output;
}

static int ax_crypto_fill_random(uint8_t* bytes, size_t len) {
  if (len == 0) {
    return 1;
  }
#ifdef _WIN32
  size_t offset = 0;
  while (offset < len) {
    unsigned int value = 0;
    if (rand_s(&value) != 0) {
      return 0;
    }
    for (size_t idx = 0; idx < sizeof(value) && offset < len; idx++) {
      bytes[offset++] = (uint8_t)(value >> (idx * 8));
    }
  }
  return 1;
#else
  FILE* file = fopen("/dev/urandom", "rb");
  if (file == 0) {
    return 0;
  }
  size_t read_len = fread(bytes, 1, len, file);
  fclose(file);
  return read_len == len;
#endif
}

char* ax_crypto_sha256_hex(const char* input) {
  uint8_t hash[32];
  ax_sha256_bytes((const uint8_t*)input, input == 0 ? 0 : strlen(input), hash);
  return ax_sha256_hex_from_hash(hash);
}

static char* ax_crypto_digest_json(
    const char* algorithm,
    const char* input_kind,
    const char* path,
    int include_range,
    int offset,
    int max_bytes,
    long long bytes,
    const char* digest) {
  if (digest == 0 || *digest == 0) {
    return ax_crypto_empty();
  }

  char* quoted_path = 0;
  if (path != 0) {
    quoted_path = ax_json_quote(path);
    if (quoted_path == 0) {
      quoted_path = (char*)malloc(3);
      if (quoted_path != 0) {
        strcpy(quoted_path, "\"\"");
      }
    }
    if (quoted_path == 0) {
      return ax_crypto_empty();
    }
  }

  const char* safe_algorithm = algorithm == 0 ? "" : algorithm;
  const char* safe_input_kind = input_kind == 0 ? "" : input_kind;
  int needed = 0;
  if (quoted_path != 0 && include_range) {
    needed = snprintf(
        0,
        0,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"path\":%s,\"offset\":%d,\"max_bytes\":%d,\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        quoted_path,
        offset,
        max_bytes,
        bytes,
        digest);
  } else if (quoted_path != 0) {
    needed = snprintf(
        0,
        0,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"path\":%s,\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        quoted_path,
        bytes,
        digest);
  } else {
    needed = snprintf(
        0,
        0,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        bytes,
        digest);
  }
  if (needed <= 0) {
    free(quoted_path);
    return ax_crypto_empty();
  }

  char* result = (char*)malloc((size_t)needed + 1);
  if (result == 0) {
    free(quoted_path);
    return ax_crypto_empty();
  }
  if (quoted_path != 0 && include_range) {
    snprintf(
        result,
        (size_t)needed + 1,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"path\":%s,\"offset\":%d,\"max_bytes\":%d,\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        quoted_path,
        offset,
        max_bytes,
        bytes,
        digest);
  } else if (quoted_path != 0) {
    snprintf(
        result,
        (size_t)needed + 1,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"path\":%s,\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        quoted_path,
        bytes,
        digest);
  } else {
    snprintf(
        result,
        (size_t)needed + 1,
        "{\"algorithm\":\"%s\",\"encoding\":\"hex\",\"input\":\"%s\",\"bytes\":%lld,\"digest\":\"%s\"}",
        safe_algorithm,
        safe_input_kind,
        bytes,
        digest);
  }
  free(quoted_path);
  return result;
}

char* ax_crypto_sha256_json(const char* input) {
  char* digest = ax_crypto_sha256_hex(input);
  long long bytes = (long long)strlen(input == 0 ? "" : input);
  char* result = ax_crypto_digest_json("sha256", "text", 0, 0, 0, 0, bytes, digest);
  free(digest);
  return result;
}

static void ax_hmac_sha256_prepare(const char* key, uint8_t ipad[64], uint8_t opad[64]) {
  uint8_t key_block[64];
  uint8_t key_hash[32];
  const uint8_t* key_bytes = (const uint8_t*)(key == 0 ? "" : key);
  size_t key_len = strlen((const char*)key_bytes);

  memset(key_block, 0, sizeof(key_block));
  if (key_len > sizeof(key_block)) {
    ax_sha256_bytes(key_bytes, key_len, key_hash);
    memcpy(key_block, key_hash, sizeof(key_hash));
  } else if (key_len > 0) {
    memcpy(key_block, key_bytes, key_len);
  }

  for (int i = 0; i < 64; i++) {
    ipad[i] = (uint8_t)(key_block[i] ^ 0x36);
    opad[i] = (uint8_t)(key_block[i] ^ 0x5c);
  }
}

static char* ax_hmac_sha256_finish(const uint8_t opad[64], ax_sha256* inner) {
  uint8_t inner_hash[32];
  uint8_t output_hash[32];
  ax_sha256_final(inner, inner_hash);

  ax_sha256 outer;
  ax_sha256_init(&outer);
  ax_sha256_update(&outer, opad, 64);
  ax_sha256_update(&outer, inner_hash, sizeof(inner_hash));
  ax_sha256_final(&outer, output_hash);

  return ax_sha256_hex_from_hash(output_hash);
}

char* ax_crypto_hmac_sha256_hex(const char* key, const char* data) {
  uint8_t ipad[64];
  uint8_t opad[64];
  const uint8_t* data_bytes = (const uint8_t*)(data == 0 ? "" : data);
  size_t data_len = strlen((const char*)data_bytes);
  ax_hmac_sha256_prepare(key, ipad, opad);

  ax_sha256 inner;
  ax_sha256_init(&inner);
  ax_sha256_update(&inner, ipad, sizeof(ipad));
  ax_sha256_update(&inner, data_bytes, data_len);
  return ax_hmac_sha256_finish(opad, &inner);
}

char* ax_crypto_hmac_sha256_json(const char* key, const char* data) {
  char* digest = ax_crypto_hmac_sha256_hex(key, data);
  long long bytes = (long long)strlen(data == 0 ? "" : data);
  char* result = ax_crypto_digest_json("hmac-sha256", "text", 0, 0, 0, 0, bytes, digest);
  free(digest);
  return result;
}

static char* ax_crypto_hmac_sha256_file_hex_with_bytes(
    const char* key,
    const char* path,
    long long* bytes_read) {
  if (bytes_read != 0) {
    *bytes_read = 0;
  }
  if (path == 0) {
    return ax_crypto_empty();
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_crypto_empty();
  }

  uint8_t ipad[64];
  uint8_t opad[64];
  ax_hmac_sha256_prepare(key, ipad, opad);
  ax_sha256 inner;
  ax_sha256_init(&inner);
  ax_sha256_update(&inner, ipad, sizeof(ipad));

  uint8_t buffer[4096];
  size_t read_len = 0;
  while ((read_len = fread(buffer, 1, sizeof(buffer), file)) > 0) {
    ax_sha256_update(&inner, buffer, read_len);
    if (bytes_read != 0) {
      *bytes_read += (long long)read_len;
    }
  }
  fclose(file);
  return ax_hmac_sha256_finish(opad, &inner);
}

char* ax_crypto_hmac_sha256_file_hex(const char* key, const char* path) {
  long long bytes = 0;
  return ax_crypto_hmac_sha256_file_hex_with_bytes(key, path, &bytes);
}

char* ax_crypto_hmac_sha256_file_json(const char* key, const char* path) {
  long long bytes = 0;
  char* digest = ax_crypto_hmac_sha256_file_hex_with_bytes(key, path, &bytes);
  char* result = ax_crypto_digest_json("hmac-sha256", "file", path, 0, 0, 0, bytes, digest);
  free(digest);
  return result;
}

static char* ax_crypto_hmac_sha256_file_range_hex_with_bytes(
    const char* key,
    const char* path,
    int offset,
    int max_bytes,
    int* normalized_offset,
    int* normalized_max_bytes,
    long long* bytes_read) {
  if (bytes_read != 0) {
    *bytes_read = 0;
  }
  if (offset < 0) {
    offset = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  if (normalized_offset != 0) {
    *normalized_offset = offset;
  }
  if (normalized_max_bytes != 0) {
    *normalized_max_bytes = max_bytes;
  }
  if (path == 0 || max_bytes <= 0) {
    return ax_crypto_empty();
  }

  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_crypto_empty();
  }
  if (fseek(file, (long)offset, SEEK_SET) != 0) {
    fclose(file);
    return ax_crypto_empty();
  }

  uint8_t ipad[64];
  uint8_t opad[64];
  ax_hmac_sha256_prepare(key, ipad, opad);
  ax_sha256 inner;
  ax_sha256_init(&inner);
  ax_sha256_update(&inner, ipad, sizeof(ipad));

  uint8_t buffer[4096];
  size_t remaining = (size_t)max_bytes;
  while (remaining > 0) {
    size_t want = remaining < sizeof(buffer) ? remaining : sizeof(buffer);
    size_t read_len = fread(buffer, 1, want, file);
    if (read_len == 0) {
      break;
    }
    ax_sha256_update(&inner, buffer, read_len);
    if (bytes_read != 0) {
      *bytes_read += (long long)read_len;
    }
    remaining -= read_len;
  }
  fclose(file);
  return ax_hmac_sha256_finish(opad, &inner);
}

char* ax_crypto_hmac_sha256_file_range_hex(const char* key, const char* path, int offset, int max_bytes) {
  long long bytes = 0;
  int normalized_offset = 0;
  int normalized_max_bytes = 0;
  return ax_crypto_hmac_sha256_file_range_hex_with_bytes(
      key,
      path,
      offset,
      max_bytes,
      &normalized_offset,
      &normalized_max_bytes,
      &bytes);
}

char* ax_crypto_hmac_sha256_file_range_json(const char* key, const char* path, int offset, int max_bytes) {
  long long bytes = 0;
  int normalized_offset = 0;
  int normalized_max_bytes = 0;
  char* digest = ax_crypto_hmac_sha256_file_range_hex_with_bytes(
      key,
      path,
      offset,
      max_bytes,
      &normalized_offset,
      &normalized_max_bytes,
      &bytes);
  char* result = ax_crypto_digest_json(
      "hmac-sha256",
      "file_range",
      path,
      1,
      normalized_offset,
      normalized_max_bytes,
      bytes,
      digest);
  free(digest);
  return result;
}

static char* ax_crypto_sha256_file_hex_with_bytes(const char* path, long long* bytes_read) {
  if (bytes_read != 0) {
    *bytes_read = 0;
  }
  if (path == 0) {
    return ax_crypto_empty();
  }
  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_crypto_empty();
  }

  ax_sha256 ctx;
  ax_sha256_init(&ctx);
  uint8_t buffer[4096];
  size_t read_len = 0;
  while ((read_len = fread(buffer, 1, sizeof(buffer), file)) > 0) {
    ax_sha256_update(&ctx, buffer, read_len);
    if (bytes_read != 0) {
      *bytes_read += (long long)read_len;
    }
  }
  fclose(file);

  uint8_t hash[32];
  ax_sha256_final(&ctx, hash);
  return ax_sha256_hex_from_hash(hash);
}

char* ax_crypto_sha256_file_hex(const char* path) {
  long long bytes = 0;
  return ax_crypto_sha256_file_hex_with_bytes(path, &bytes);
}

char* ax_crypto_sha256_file_json(const char* path) {
  long long bytes = 0;
  char* digest = ax_crypto_sha256_file_hex_with_bytes(path, &bytes);
  char* result = ax_crypto_digest_json("sha256", "file", path, 0, 0, 0, bytes, digest);
  free(digest);
  return result;
}

static char* ax_crypto_sha256_file_range_hex_with_bytes(
    const char* path,
    int offset,
    int max_bytes,
    int* normalized_offset,
    int* normalized_max_bytes,
    long long* bytes_read) {
  if (bytes_read != 0) {
    *bytes_read = 0;
  }
  if (offset < 0) {
    offset = 0;
  }
  if (max_bytes > 1024 * 1024) {
    max_bytes = 1024 * 1024;
  }
  if (normalized_offset != 0) {
    *normalized_offset = offset;
  }
  if (normalized_max_bytes != 0) {
    *normalized_max_bytes = max_bytes;
  }
  if (path == 0 || max_bytes <= 0) {
    return ax_crypto_empty();
  }

  FILE* file = fopen(path, "rb");
  if (file == 0) {
    return ax_crypto_empty();
  }
  if (fseek(file, (long)offset, SEEK_SET) != 0) {
    fclose(file);
    return ax_crypto_empty();
  }

  ax_sha256 ctx;
  ax_sha256_init(&ctx);
  uint8_t buffer[4096];
  size_t remaining = (size_t)max_bytes;
  while (remaining > 0) {
    size_t want = remaining < sizeof(buffer) ? remaining : sizeof(buffer);
    size_t read_len = fread(buffer, 1, want, file);
    if (read_len == 0) {
      break;
    }
    ax_sha256_update(&ctx, buffer, read_len);
    if (bytes_read != 0) {
      *bytes_read += (long long)read_len;
    }
    remaining -= read_len;
  }
  fclose(file);

  uint8_t hash[32];
  ax_sha256_final(&ctx, hash);
  return ax_sha256_hex_from_hash(hash);
}

char* ax_crypto_sha256_file_range_hex(const char* path, int offset, int max_bytes) {
  long long bytes = 0;
  int normalized_offset = 0;
  int normalized_max_bytes = 0;
  return ax_crypto_sha256_file_range_hex_with_bytes(
      path,
      offset,
      max_bytes,
      &normalized_offset,
      &normalized_max_bytes,
      &bytes);
}

char* ax_crypto_sha256_file_range_json(const char* path, int offset, int max_bytes) {
  long long bytes = 0;
  int normalized_offset = 0;
  int normalized_max_bytes = 0;
  char* digest = ax_crypto_sha256_file_range_hex_with_bytes(
      path,
      offset,
      max_bytes,
      &normalized_offset,
      &normalized_max_bytes,
      &bytes);
  char* result = ax_crypto_digest_json(
      "sha256",
      "file_range",
      path,
      1,
      normalized_offset,
      normalized_max_bytes,
      bytes,
      digest);
  free(digest);
  return result;
}

char* ax_crypto_base64_encode(const char* input) {
  static const char* table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const unsigned char* bytes = (const unsigned char*)(input == 0 ? "" : input);
  size_t len = strlen((const char*)bytes);
  size_t out_len = ((len + 2) / 3) * 4;
  char* output = (char*)malloc(out_len + 1);
  if (output == 0) {
    return 0;
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

static int ax_crypto_base64_value(unsigned char ch) {
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

static int ax_crypto_base64_space(unsigned char ch) {
  return ch == ' ' || ch == '\n' || ch == '\r' || ch == '\t';
}

char* ax_crypto_base64_decode(const char* input) {
  if (input == 0) {
    return ax_crypto_empty();
  }
  size_t len = strlen(input);
  char* output = (char*)malloc(((len + 3) / 4) * 3 + 1);
  if (output == 0) {
    return 0;
  }

  uint32_t buffer = 0;
  int bits = 0;
  int seen_padding = 0;
  size_t output_idx = 0;
  for (size_t i = 0; i < len; i++) {
    unsigned char ch = (unsigned char)input[i];
    if (ax_crypto_base64_space(ch)) {
      continue;
    }
    if (ch == '=') {
      seen_padding = 1;
      continue;
    }
    if (seen_padding) {
      free(output);
      return ax_crypto_empty();
    }
    int value = ax_crypto_base64_value(ch);
    if (value < 0) {
      free(output);
      return ax_crypto_empty();
    }
    buffer = (buffer << 6) | (uint32_t)value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      output[output_idx++] = (char)((buffer >> bits) & 0xff);
    }
  }

  output[output_idx] = 0;
  return output;
}

int ax_crypto_constant_time_eq(const char* left, const char* right) {
  const unsigned char* left_bytes = (const unsigned char*)(left == 0 ? "" : left);
  const unsigned char* right_bytes = (const unsigned char*)(right == 0 ? "" : right);
  size_t left_len = strlen((const char*)left_bytes);
  size_t right_len = strlen((const char*)right_bytes);
  size_t max_len = left_len > right_len ? left_len : right_len;
  unsigned char diff = (unsigned char)(left_len ^ right_len);

  for (size_t i = 0; i < max_len; i++) {
    unsigned char a = i < left_len ? left_bytes[i] : 0;
    unsigned char b = i < right_len ? right_bytes[i] : 0;
    diff |= (unsigned char)(a ^ b);
  }

  return diff == 0;
}

static int ax_crypto_verify_digest_hex(char* digest, const char* expected) {
  if (digest == 0 || *digest == 0) {
    free(digest);
    return 0;
  }
  int ok = ax_crypto_constant_time_eq(digest, expected);
  free(digest);
  return ok;
}

int ax_crypto_sha256_verify_hex(const char* input, const char* expected) {
  return ax_crypto_verify_digest_hex(ax_crypto_sha256_hex(input), expected);
}

int ax_crypto_hmac_sha256_verify_hex(const char* key, const char* data, const char* expected) {
  return ax_crypto_verify_digest_hex(ax_crypto_hmac_sha256_hex(key, data), expected);
}

int ax_crypto_hmac_sha256_file_verify_hex(const char* key, const char* path, const char* expected) {
  return ax_crypto_verify_digest_hex(ax_crypto_hmac_sha256_file_hex(key, path), expected);
}

int ax_crypto_hmac_sha256_file_range_verify_hex(
    const char* key,
    const char* path,
    int offset,
    int max_bytes,
    const char* expected) {
  return ax_crypto_verify_digest_hex(
      ax_crypto_hmac_sha256_file_range_hex(key, path, offset, max_bytes),
      expected);
}

int ax_crypto_sha256_file_verify_hex(const char* path, const char* expected) {
  return ax_crypto_verify_digest_hex(ax_crypto_sha256_file_hex(path), expected);
}

int ax_crypto_sha256_file_range_verify_hex(const char* path, int offset, int max_bytes, const char* expected) {
  return ax_crypto_verify_digest_hex(ax_crypto_sha256_file_range_hex(path, offset, max_bytes), expected);
}

char* ax_crypto_random_hex(int byte_count) {
  if (byte_count <= 0 || byte_count > 4096) {
    return ax_crypto_empty();
  }
  size_t len = (size_t)byte_count;
  uint8_t* bytes = (uint8_t*)malloc(len);
  if (bytes == 0) {
    return ax_crypto_empty();
  }
  if (!ax_crypto_fill_random(bytes, len)) {
    free(bytes);
    return ax_crypto_empty();
  }
  char* output = ax_bytes_to_hex(bytes, len);
  free(bytes);
  return output == 0 ? ax_crypto_empty() : output;
}

static char* ax_crypto_base64url_encode_bytes(const uint8_t* bytes, size_t len) {
  static const char* table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  if (bytes == 0 && len > 0) {
    return ax_crypto_empty();
  }
  if (len > (((size_t)-1) - 2) / 4) {
    return ax_crypto_empty();
  }
  size_t out_len = (len * 4 + 2) / 3;
  char* output = (char*)malloc(out_len + 1);
  if (output == 0) {
    return ax_crypto_empty();
  }

  size_t input_idx = 0;
  size_t output_idx = 0;
  while (input_idx < len) {
    size_t remaining = len - input_idx;
    uint32_t octet_a = bytes[input_idx++];
    uint32_t octet_b = remaining > 1 ? bytes[input_idx++] : 0;
    uint32_t octet_c = remaining > 2 ? bytes[input_idx++] : 0;
    uint32_t triple = (octet_a << 16) | (octet_b << 8) | octet_c;

    output[output_idx++] = table[(triple >> 18) & 0x3f];
    output[output_idx++] = table[(triple >> 12) & 0x3f];
    if (remaining > 1) {
      output[output_idx++] = table[(triple >> 6) & 0x3f];
    }
    if (remaining > 2) {
      output[output_idx++] = table[triple & 0x3f];
    }
  }

  output[out_len] = 0;
  return output;
}

char* ax_crypto_random_base64url(int byte_count) {
  if (byte_count <= 0 || byte_count > 4096) {
    return ax_crypto_empty();
  }
  size_t len = (size_t)byte_count;
  uint8_t* bytes = (uint8_t*)malloc(len);
  if (bytes == 0) {
    return ax_crypto_empty();
  }
  if (!ax_crypto_fill_random(bytes, len)) {
    free(bytes);
    return ax_crypto_empty();
  }
  char* output = ax_crypto_base64url_encode_bytes(bytes, len);
  free(bytes);
  return output == 0 ? ax_crypto_empty() : output;
}

char* ax_crypto_uuid_v4(void) {
  uint8_t bytes[16];
  if (!ax_crypto_fill_random(bytes, sizeof(bytes))) {
    return ax_crypto_empty();
  }
  bytes[6] = (uint8_t)((bytes[6] & 0x0f) | 0x40);
  bytes[8] = (uint8_t)((bytes[8] & 0x3f) | 0x80);

  static const char* hex = "0123456789abcdef";
  char* output = (char*)malloc(37);
  if (output == 0) {
    return ax_crypto_empty();
  }
  int out = 0;
  for (int i = 0; i < 16; i++) {
    if (i == 4 || i == 6 || i == 8 || i == 10) {
      output[out++] = '-';
    }
    output[out++] = hex[bytes[i] >> 4];
    output[out++] = hex[bytes[i] & 0x0f];
  }
  output[out] = 0;
  return output;
}
