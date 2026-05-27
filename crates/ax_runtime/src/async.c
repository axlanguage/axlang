#include "ax_runtime.h"

#ifdef _WIN32

#include <stdlib.h>

typedef void* (*ax_async_entry)(void*);

typedef struct {
  int kind;
  union {
    int i32;
    long long i64;
    double f64;
    void* ptr;
  } value;
} ax_async_slot;

typedef struct {
  int count;
  ax_async_slot slots[];
} ax_async_context;

typedef struct {
  void* result;
} ax_future;

void* ax_async_spawn_with_context(void* entry, void* context);

void* ax_async_spawn(void* entry) {
  return ax_async_spawn_with_context(entry, 0);
}

void* ax_async_spawn_with_context(void* entry, void* context) {
  ax_future* future = (ax_future*)calloc(1, sizeof(ax_future));
  if (future == 0) {
    free(context);
    return 0;
  }
  if (entry != 0) {
    future->result = ((ax_async_entry)entry)(context);
  }
  free(context);
  return future;
}

void* ax_async_context_new(int count) {
  if (count <= 0) {
    return 0;
  }
  ax_async_context* context =
    (ax_async_context*)calloc(1, sizeof(ax_async_context) + ((size_t)count * sizeof(ax_async_slot)));
  if (context != 0) {
    context->count = count;
  }
  return context;
}

static ax_async_slot* ax_async_context_slot(void* value, int index) {
  ax_async_context* context = (ax_async_context*)value;
  if (context == 0 || index < 0 || index >= context->count) {
    return 0;
  }
  return &context->slots[index];
}

void ax_async_context_set_i32(void* context, int index, int value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 1;
    slot->value.i32 = value;
  }
}

void ax_async_context_set_i64(void* context, int index, long long value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 2;
    slot->value.i64 = value;
  }
}

void ax_async_context_set_f64(void* context, int index, double value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 3;
    slot->value.f64 = value;
  }
}

void ax_async_context_set_ptr(void* context, int index, void* value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 4;
    slot->value.ptr = value;
  }
}

int ax_async_context_get_i32(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.i32;
}

long long ax_async_context_get_i64(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.i64;
}

double ax_async_context_get_f64(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0.0 : slot->value.f64;
}

void* ax_async_context_get_ptr(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.ptr;
}

static void* ax_async_take(void* value) {
  ax_future* future = (ax_future*)value;
  if (future == 0) {
    return 0;
  }
  void* result = future->result;
  free(future);
  return result;
}

void ax_async_cancel(void* value) {
  free(value);
}

void ax_async_detach(void* value) {
  free(value);
}

void ax_async_await_void(void* future) {
  ax_async_take(future);
}

int ax_async_await_i32(void* future) {
  int* result = (int*)ax_async_take(future);
  if (result == 0) {
    return 0;
  }
  int value = *result;
  free(result);
  return value;
}

long long ax_async_await_i64(void* future) {
  long long* result = (long long*)ax_async_take(future);
  if (result == 0) {
    return 0;
  }
  long long value = *result;
  free(result);
  return value;
}

double ax_async_await_f64(void* future) {
  double* result = (double*)ax_async_take(future);
  if (result == 0) {
    return 0.0;
  }
  double value = *result;
  free(result);
  return value;
}

void* ax_async_await_ptr(void* future) {
  return ax_async_take(future);
}

void* ax_async_box_i32(int value) {
  int* result = (int*)malloc(sizeof(int));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_i64(long long value) {
  long long* result = (long long*)malloc(sizeof(long long));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_f64(double value) {
  double* result = (double*)malloc(sizeof(double));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_bool(int value) {
  int* result = (int*)malloc(sizeof(int));
  if (result != 0) {
    *result = value;
  }
  return result;
}

#else

#include <pthread.h>
#include <stdlib.h>

typedef void* (*ax_async_entry)(void*);

typedef struct {
  int kind;
  union {
    int i32;
    long long i64;
    double f64;
    void* ptr;
  } value;
} ax_async_slot;

typedef struct {
  int count;
  ax_async_slot slots[];
} ax_async_context;

typedef struct ax_future ax_future;

struct ax_future {
  pthread_t thread;
  pthread_mutex_t lock;
  pthread_cond_t ready;
  ax_async_entry entry;
  void* context;
  void* result;
  int started;
  int finished;
  int cancelled;
  int detached;
  ax_future* next;
};

typedef struct {
  pthread_mutex_t lock;
  pthread_cond_t ready;
  pthread_t thread;
  ax_future* head;
  ax_future* tail;
  int running;
} ax_async_scheduler;

static ax_async_scheduler ax_scheduler = {
  PTHREAD_MUTEX_INITIALIZER,
  PTHREAD_COND_INITIALIZER,
  0,
  0,
  0,
  0,
};
static pthread_once_t ax_scheduler_once = PTHREAD_ONCE_INIT;

static void ax_async_future_destroy(ax_future* future);

static void ax_async_context_cleanup(void* data) {
  ax_future* future = (ax_future*)data;
  free(future->context);
  future->context = 0;
}

static void* ax_async_thread_main(void* data) {
  ax_future* future = (ax_future*)data;
  int detached = 0;
  void* result = 0;
  pthread_cleanup_push(ax_async_context_cleanup, future);
  pthread_setcancelstate(PTHREAD_CANCEL_ENABLE, 0);
  pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, 0);
  future->result = future->entry(future->context);
  pthread_mutex_lock(&future->lock);
  future->finished = 1;
  detached = future->detached;
  result = future->result;
  pthread_cond_broadcast(&future->ready);
  pthread_mutex_unlock(&future->lock);
  pthread_cleanup_pop(1);
  if (detached) {
    ax_async_future_destroy(future);
  }
  return result;
}

static void ax_async_future_destroy(ax_future* future) {
  pthread_mutex_destroy(&future->lock);
  pthread_cond_destroy(&future->ready);
  free(future);
}

static int ax_async_future_start_locked(ax_future* future) {
  if (future->cancelled) {
    free(future->context);
    future->context = 0;
    future->finished = 1;
    pthread_cond_broadcast(&future->ready);
    return 0;
  }
  if (pthread_create(&future->thread, 0, ax_async_thread_main, future) != 0) {
    free(future->context);
    future->context = 0;
    future->finished = 1;
    pthread_cond_broadcast(&future->ready);
    return 0;
  }
  future->started = 1;
  if (future->detached) {
    pthread_detach(future->thread);
  }
  pthread_cond_broadcast(&future->ready);
  return 1;
}

static void* ax_async_scheduler_main(void* data) {
  (void)data;
  for (;;) {
    pthread_mutex_lock(&ax_scheduler.lock);
    while (ax_scheduler.head == 0) {
      pthread_cond_wait(&ax_scheduler.ready, &ax_scheduler.lock);
    }
    ax_future* future = ax_scheduler.head;
    ax_scheduler.head = future->next;
    if (ax_scheduler.head == 0) {
      ax_scheduler.tail = 0;
    }
    future->next = 0;
    pthread_mutex_unlock(&ax_scheduler.lock);

    pthread_mutex_lock(&future->lock);
    ax_async_future_start_locked(future);
    int destroy_cancelled = future->cancelled && !future->started;
    pthread_mutex_unlock(&future->lock);
    if (destroy_cancelled) {
      ax_async_future_destroy(future);
    }
  }
  return 0;
}

static void ax_async_scheduler_start(void) {
  if (pthread_create(&ax_scheduler.thread, 0, ax_async_scheduler_main, 0) == 0) {
    pthread_detach(ax_scheduler.thread);
    ax_scheduler.running = 1;
  }
}

static int ax_async_enqueue(ax_future* future) {
  pthread_once(&ax_scheduler_once, ax_async_scheduler_start);
  if (!ax_scheduler.running) {
    pthread_mutex_lock(&future->lock);
    int started = ax_async_future_start_locked(future);
    int failed = !started && !future->cancelled;
    pthread_mutex_unlock(&future->lock);
    return !failed;
  }
  pthread_mutex_lock(&ax_scheduler.lock);
  if (ax_scheduler.tail == 0) {
    ax_scheduler.head = future;
    ax_scheduler.tail = future;
  } else {
    ax_scheduler.tail->next = future;
    ax_scheduler.tail = future;
  }
  pthread_cond_signal(&ax_scheduler.ready);
  pthread_mutex_unlock(&ax_scheduler.lock);
  return 1;
}

void* ax_async_spawn(void* entry) {
  return ax_async_spawn_with_context(entry, 0);
}

void* ax_async_spawn_with_context(void* entry, void* context) {
  ax_future* future = (ax_future*)calloc(1, sizeof(ax_future));
  if (future == 0) {
    free(context);
    return 0;
  }
  future->entry = (ax_async_entry)entry;
  future->context = context;
  if (pthread_mutex_init(&future->lock, 0) != 0) {
    free(context);
    free(future);
    return 0;
  }
  if (pthread_cond_init(&future->ready, 0) != 0) {
    pthread_mutex_destroy(&future->lock);
    free(context);
    free(future);
    return 0;
  }

  pthread_mutex_lock(&future->lock);
  int started = ax_async_future_start_locked(future);
  int failed = !started && !future->cancelled;
  pthread_mutex_unlock(&future->lock);
  if (failed) {
    ax_async_future_destroy(future);
    return 0;
  }
  return future;
}

void* ax_async_context_new(int count) {
  if (count <= 0) {
    return 0;
  }
  ax_async_context* context =
    (ax_async_context*)calloc(1, sizeof(ax_async_context) + ((size_t)count * sizeof(ax_async_slot)));
  if (context != 0) {
    context->count = count;
  }
  return context;
}

static ax_async_slot* ax_async_context_slot(void* value, int index) {
  ax_async_context* context = (ax_async_context*)value;
  if (context == 0 || index < 0) {
    return 0;
  }
  if (index >= context->count) {
    return 0;
  }
  return &context->slots[index];
}

void ax_async_context_set_i32(void* context, int index, int value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 1;
    slot->value.i32 = value;
  }
}

void ax_async_context_set_i64(void* context, int index, long long value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 2;
    slot->value.i64 = value;
  }
}

void ax_async_context_set_f64(void* context, int index, double value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 3;
    slot->value.f64 = value;
  }
}

void ax_async_context_set_ptr(void* context, int index, void* value) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  if (slot != 0) {
    slot->kind = 4;
    slot->value.ptr = value;
  }
}

int ax_async_context_get_i32(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.i32;
}

long long ax_async_context_get_i64(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.i64;
}

double ax_async_context_get_f64(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0.0 : slot->value.f64;
}

void* ax_async_context_get_ptr(void* context, int index) {
  ax_async_slot* slot = ax_async_context_slot(context, index);
  return slot == 0 ? 0 : slot->value.ptr;
}

static void* ax_async_join(void* value) {
  ax_future* future = (ax_future*)value;
  if (future == 0) {
    return 0;
  }
  pthread_mutex_lock(&future->lock);
  while (!future->started && !future->finished) {
    pthread_cond_wait(&future->ready, &future->lock);
  }
  if (!future->started) {
    void* result = future->result;
    pthread_mutex_unlock(&future->lock);
    ax_async_future_destroy(future);
    return result;
  }
  pthread_t thread = future->thread;
  pthread_mutex_unlock(&future->lock);

  void* result = 0;
  pthread_join(thread, &result);
  if (result == PTHREAD_CANCELED) {
    result = 0;
  }
  if (result == 0) {
    result = future->result;
  }
  ax_async_future_destroy(future);
  return result;
}

void ax_async_cancel(void* value) {
  ax_future* future = (ax_future*)value;
  if (future == 0) {
    return;
  }
  pthread_mutex_lock(&future->lock);
  if (!future->started && !future->finished) {
    future->cancelled = 1;
    pthread_cond_broadcast(&future->ready);
    pthread_mutex_unlock(&future->lock);
    return;
  }
  if (!future->started) {
    pthread_mutex_unlock(&future->lock);
    ax_async_future_destroy(future);
    return;
  }
  pthread_t thread = future->thread;
  pthread_mutex_unlock(&future->lock);

  void* result = 0;
  pthread_cancel(thread);
  pthread_join(thread, &result);
  ax_async_future_destroy(future);
}

void ax_async_detach(void* value) {
  ax_future* future = (ax_future*)value;
  if (future == 0) {
    return;
  }
  pthread_t thread = 0;
  int should_detach_thread = 0;
  int should_destroy = 0;

  pthread_mutex_lock(&future->lock);
  future->detached = 1;
  if (future->started) {
    thread = future->thread;
    should_detach_thread = 1;
    should_destroy = future->finished;
  }
  pthread_mutex_unlock(&future->lock);

  if (should_detach_thread) {
    pthread_detach(thread);
  }
  if (should_destroy) {
    ax_async_future_destroy(future);
  }
}

void ax_async_await_void(void* future) {
  ax_async_join(future);
}

int ax_async_await_i32(void* future) {
  int* result = (int*)ax_async_join(future);
  if (result == 0) {
    return 0;
  }
  int value = *result;
  free(result);
  return value;
}

long long ax_async_await_i64(void* future) {
  long long* result = (long long*)ax_async_join(future);
  if (result == 0) {
    return 0;
  }
  long long value = *result;
  free(result);
  return value;
}

double ax_async_await_f64(void* future) {
  double* result = (double*)ax_async_join(future);
  if (result == 0) {
    return 0.0;
  }
  double value = *result;
  free(result);
  return value;
}

void* ax_async_await_ptr(void* future) {
  return ax_async_join(future);
}

void* ax_async_box_i32(int value) {
  int* result = (int*)malloc(sizeof(int));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_i64(long long value) {
  long long* result = (long long*)malloc(sizeof(long long));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_f64(double value) {
  double* result = (double*)malloc(sizeof(double));
  if (result != 0) {
    *result = value;
  }
  return result;
}

void* ax_async_box_bool(int value) {
  int* result = (int*)malloc(sizeof(int));
  if (result != 0) {
    *result = value;
  }
  return result;
}

#endif
