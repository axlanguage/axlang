#include "ax_runtime.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#ifndef _WIN32
#include <pthread.h>
#endif

#define AX_MAP_SHARDS 64u

typedef struct {
  uint64_t hash;
  char* key;
  size_t key_len;
  char* value;
  size_t value_len;
  unsigned char state;
} ax_map_entry;

typedef struct {
  ax_map_entry* entries;
  size_t capacity;
  size_t len;
  size_t tombstones;
#ifndef _WIN32
  pthread_mutex_t lock;
#endif
} ax_map_shard;

typedef struct {
  ax_map_shard shards[AX_MAP_SHARDS];
} ax_map;

static void ax_map_lock(ax_map_shard* shard) {
#ifndef _WIN32
  pthread_mutex_lock(&shard->lock);
#else
  (void)shard;
#endif
}

static void ax_map_unlock(ax_map_shard* shard) {
#ifndef _WIN32
  pthread_mutex_unlock(&shard->lock);
#else
  (void)shard;
#endif
}

static char* ax_map_strdup_len(const char* value, size_t len) {
  char* out = (char*)malloc(len + 1);
  if (out == 0) {
    return 0;
  }
  if (len > 0) {
    memcpy(out, value, len);
  }
  out[len] = 0;
  return out;
}

static size_t ax_map_pow2_at_least(size_t value) {
  size_t out = 1;
  while (out < value) {
    out <<= 1;
  }
  return out;
}

static uint64_t ax_map_hash(const char* data, size_t len) {
  uint64_t h = 1469598103934665603ull;
  for (size_t i = 0; i < len; i++) {
    h ^= (unsigned char)data[i];
    h *= 1099511628211ull;
  }
  h ^= h >> 32;
  h *= 0xd6e8feb86659fd93ull;
  h ^= h >> 32;
  return h ? h : 1;
}

static ax_map_shard* ax_map_shard_for_hash(ax_map* map, uint64_t hash) {
  return &map->shards[(hash >> 58) & (AX_MAP_SHARDS - 1u)];
}

static ax_map_entry* ax_map_find_slot(ax_map_shard* shard,
                                      const char* key,
                                      size_t key_len,
                                      uint64_t hash,
                                      int* found) {
  size_t mask = shard->capacity - 1;
  size_t first_tombstone = (size_t)-1;
  for (size_t probe = 0;; probe++) {
    size_t index = (hash + probe) & mask;
    ax_map_entry* entry = &shard->entries[index];
    if (entry->state == 0) {
      *found = 0;
      return first_tombstone != (size_t)-1 ? &shard->entries[first_tombstone] : entry;
    }
    if (entry->state == 2) {
      if (first_tombstone == (size_t)-1) {
        first_tombstone = index;
      }
      continue;
    }
    if (entry->hash == hash && entry->key_len == key_len &&
        memcmp(entry->key, key, key_len) == 0) {
      *found = 1;
      return entry;
    }
  }
}

static void ax_map_rehash(ax_map_shard* shard, size_t new_capacity) {
  ax_map_entry* old_entries = shard->entries;
  size_t old_capacity = shard->capacity;
  ax_map_entry* new_entries = (ax_map_entry*)calloc(new_capacity, sizeof(ax_map_entry));
  if (new_entries == 0) {
    return;
  }
  shard->entries = new_entries;
  shard->capacity = new_capacity;
  shard->len = 0;
  shard->tombstones = 0;
  for (size_t i = 0; i < old_capacity; i++) {
    ax_map_entry* old = &old_entries[i];
    if (old->state != 1) {
      continue;
    }
    int found = 0;
    ax_map_entry* slot = ax_map_find_slot(shard, old->key, old->key_len, old->hash, &found);
    *slot = *old;
    slot->state = 1;
    shard->len++;
  }
  free(old_entries);
}

static void ax_map_grow_if_needed(ax_map_shard* shard) {
  if ((shard->len + shard->tombstones + 1) * 10 >= shard->capacity * 7) {
    ax_map_rehash(shard, shard->capacity << 1);
  }
}

static int ax_map_shard_init(ax_map_shard* shard, size_t capacity) {
  shard->entries = 0;
  shard->capacity = ax_map_pow2_at_least(capacity < 16 ? 16 : capacity);
  shard->len = 0;
  shard->tombstones = 0;
#ifndef _WIN32
  if (pthread_mutex_init(&shard->lock, 0) != 0) {
    return 0;
  }
#endif
  shard->entries = (ax_map_entry*)calloc(shard->capacity, sizeof(ax_map_entry));
  if (shard->entries == 0) {
#ifndef _WIN32
    pthread_mutex_destroy(&shard->lock);
#endif
    return 0;
  }
  return 1;
}

static void ax_map_shard_destroy(ax_map_shard* shard) {
  if (shard->entries != 0) {
    for (size_t i = 0; i < shard->capacity; i++) {
      ax_map_entry* entry = &shard->entries[i];
      if (entry->state == 1) {
        free(entry->key);
        free(entry->value);
      }
    }
    free(shard->entries);
  }
#ifndef _WIN32
  pthread_mutex_destroy(&shard->lock);
#endif
}

void* ax_map_new(int capacity) {
  size_t cap = capacity <= 0 ? 1024u : (size_t)capacity;
  size_t shard_cap = (cap + AX_MAP_SHARDS - 1u) / AX_MAP_SHARDS;
  ax_map* map = (ax_map*)calloc(1, sizeof(ax_map));
  if (map == 0) {
    return 0;
  }
  for (size_t i = 0; i < AX_MAP_SHARDS; i++) {
    if (!ax_map_shard_init(&map->shards[i], shard_cap)) {
      for (size_t j = 0; j < i; j++) {
        ax_map_shard_destroy(&map->shards[j]);
      }
      free(map);
      return 0;
    }
  }
  return map;
}

void ax_map_set(void* map_ptr, const char* key, const char* value) {
  ax_map* map = (ax_map*)map_ptr;
  const char* k = key == 0 ? "" : key;
  const char* v = value == 0 ? "" : value;
  if (map == 0) {
    return;
  }

  size_t key_len = strlen(k);
  size_t value_len = strlen(v);
  uint64_t hash = ax_map_hash(k, key_len);
  char* next_value = ax_map_strdup_len(v, value_len);
  if (next_value == 0) {
    return;
  }
  char* next_key = ax_map_strdup_len(k, key_len);
  if (next_key == 0) {
    free(next_value);
    return;
  }

  ax_map_shard* shard = ax_map_shard_for_hash(map, hash);
  ax_map_lock(shard);
  ax_map_grow_if_needed(shard);

  int found = 0;
  ax_map_entry* entry = ax_map_find_slot(shard, k, key_len, hash, &found);
  if (found) {
    free(next_key);
    free(entry->value);
    entry->value = next_value;
    entry->value_len = value_len;
    ax_map_unlock(shard);
    return;
  }
  if (entry->state == 2) {
    shard->tombstones--;
  }
  entry->hash = hash;
  entry->key = next_key;
  entry->key_len = key_len;
  entry->value = next_value;
  entry->value_len = value_len;
  entry->state = 1;
  shard->len++;
  ax_map_unlock(shard);
}

char* ax_map_get(void* map_ptr, const char* key) {
  ax_map* map = (ax_map*)map_ptr;
  const char* k = key == 0 ? "" : key;
  if (map == 0) {
    return ax_map_strdup_len("", 0);
  }
  size_t key_len = strlen(k);
  uint64_t hash = ax_map_hash(k, key_len);
  ax_map_shard* shard = ax_map_shard_for_hash(map, hash);

  ax_map_lock(shard);
  int found = 0;
  ax_map_entry* entry = ax_map_find_slot(shard, k, key_len, hash, &found);
  if (!found) {
    ax_map_unlock(shard);
    return ax_map_strdup_len("", 0);
  }
  char* out = ax_map_strdup_len(entry->value, entry->value_len);
  ax_map_unlock(shard);
  return out;
}

int ax_map_has(void* map_ptr, const char* key) {
  ax_map* map = (ax_map*)map_ptr;
  const char* k = key == 0 ? "" : key;
  if (map == 0) {
    return 0;
  }
  size_t key_len = strlen(k);
  uint64_t hash = ax_map_hash(k, key_len);
  ax_map_shard* shard = ax_map_shard_for_hash(map, hash);

  ax_map_lock(shard);
  int found = 0;
  ax_map_find_slot(shard, k, key_len, hash, &found);
  ax_map_unlock(shard);
  return found;
}

int ax_map_del(void* map_ptr, const char* key) {
  ax_map* map = (ax_map*)map_ptr;
  const char* k = key == 0 ? "" : key;
  if (map == 0) {
    return 0;
  }
  size_t key_len = strlen(k);
  uint64_t hash = ax_map_hash(k, key_len);
  ax_map_shard* shard = ax_map_shard_for_hash(map, hash);

  ax_map_lock(shard);
  int found = 0;
  ax_map_entry* entry = ax_map_find_slot(shard, k, key_len, hash, &found);
  if (!found) {
    ax_map_unlock(shard);
    return 0;
  }
  free(entry->key);
  free(entry->value);
  memset(entry, 0, sizeof(*entry));
  entry->state = 2;
  shard->len--;
  shard->tombstones++;
  ax_map_unlock(shard);
  return 1;
}

int ax_map_len(void* map_ptr) {
  ax_map* map = (ax_map*)map_ptr;
  if (map == 0) {
    return 0;
  }
  size_t total = 0;
  for (size_t i = 0; i < AX_MAP_SHARDS; i++) {
    ax_map_shard* shard = &map->shards[i];
    ax_map_lock(shard);
    total += shard->len;
    ax_map_unlock(shard);
  }
  return total > 2147483647u ? 2147483647 : (int)total;
}

void ax_map_clear(void* map_ptr) {
  ax_map* map = (ax_map*)map_ptr;
  if (map == 0) {
    return;
  }
  for (size_t i = 0; i < AX_MAP_SHARDS; i++) {
    ax_map_shard* shard = &map->shards[i];
    ax_map_lock(shard);
    for (size_t j = 0; j < shard->capacity; j++) {
      ax_map_entry* entry = &shard->entries[j];
      if (entry->state == 1) {
        free(entry->key);
        free(entry->value);
      }
    }
    memset(shard->entries, 0, shard->capacity * sizeof(ax_map_entry));
    shard->len = 0;
    shard->tombstones = 0;
    ax_map_unlock(shard);
  }
}
