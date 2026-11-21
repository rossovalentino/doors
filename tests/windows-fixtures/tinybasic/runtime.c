/* Original narrow C platform for the pinned, unchanged Tiny Basic application.
   Public Win32 imports only; no Microsoft runtime implementation is
   distributed. */
#include "platform.h"
#define API __declspec(dllimport)
API void *__stdcall GetStdHandle(unsigned);
API int __stdcall WriteFile(void *, const void *, unsigned, unsigned *, void *);
API int __stdcall ReadFile(void *, void *, unsigned, unsigned *, void *);
API void *__stdcall CreateFileW(const unsigned short *, unsigned, unsigned,
                                void *, unsigned, unsigned, void *);
API int __stdcall CloseHandle(void *);
API unsigned short *__stdcall GetCommandLineW(void);
API unsigned long long __stdcall GetTickCount64(void);
API __declspec(noreturn) void __stdcall ExitProcess(unsigned);
int _fltused;
void *memset(void *p, int value, size_t count) {
  volatile unsigned char *out = p;
  while (count--)
    *out++ = (unsigned char)value;
  return p;
}
static FILE streams[5];
FILE *stdin = &streams[0];
static union {
  unsigned long long alignment;
  unsigned char bytes[384 * 1024];
} arena;
typedef struct Block {
  size_t bytes;
  int used;
  struct Block *next;
  unsigned long long padding;
} Block;
static Block *blocks;
size_t strlen(const char *s) {
  size_t n = 0;
  while (s[n])
    ++n;
  return n;
}
int strcmp(const char *a, const char *b) {
  while (*a && *a == *b) {
    ++a;
    ++b;
  }
  return (unsigned char)*a - (unsigned char)*b;
}
char *strcpy(char *a, const char *b) {
  char *out = a;
  while ((*a++ = *b++) != 0) {
  }
  return out;
}
char *strcat(char *a, const char *b) {
  strcpy(a + strlen(a), b);
  return a;
}
char *strchr(const char *s, int c) {
  do {
    if (*s == c)
      return (char *)s;
  } while (*s++);
  return NULL;
}
void *malloc(size_t bytes) {
  Block *b;
  size_t n;
  if (bytes > sizeof(arena.bytes) - sizeof(Block) - 15)
    return NULL;
  n = (bytes + 15) & ~(size_t)15;
  if (!n)
    n = 16;
  if (!blocks) {
    blocks = (Block *)arena.bytes;
    blocks->bytes = sizeof(arena.bytes) - sizeof(Block);
    blocks->used = 0;
    blocks->next = NULL;
  }
  for (b = blocks; b; b = b->next)
    if (!b->used && b->bytes >= n) {
      if (b->bytes >= n + sizeof(Block) + 16) {
        Block *tail = (Block *)((char *)(b + 1) + n);
        tail->bytes = b->bytes - n - sizeof(Block);
        tail->used = 0;
        tail->next = b->next;
        b->next = tail;
        b->bytes = n;
      }
      b->used = 1;
      return b + 1;
    }
  return NULL;
}
void free(void *p) {
  Block *b;
  if (!p)
    return;
  for (b = blocks; b; b = b->next)
    if (b + 1 == p) {
      b->used = 0;
      break;
    }
  for (b = blocks; b && b->next;)
    if (!b->used && !b->next->used) {
      b->bytes += sizeof(Block) + b->next->bytes;
      b->next = b->next->next;
    } else
      b = b->next;
}
void *realloc(void *p, size_t bytes) {
  Block *b;
  void *q;
  size_t i;
  if (!p)
    return malloc(bytes);
  if (!bytes) {
    free(p);
    return NULL;
  }
  for (b = blocks; b && b + 1 != p; b = b->next) {
  }
  if (!b)
    return NULL;
  if (bytes <= b->bytes)
    return p;
  q = malloc(bytes);
  if (!q)
    return NULL;
  for (i = 0; i < b->bytes; ++i)
    ((char *)q)[i] = ((char *)p)[i];
  free(p);
  return q;
}
char *strdup(const char *s) {
  char *p = malloc(strlen(s) + 1);
  if (p)
    strcpy(p, s);
  return p;
}
int isdigit(int c) { return c >= '0' && c <= '9'; }
int isalpha(int c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isspace(int c) { return c == ' ' || (c >= 9 && c <= 13); }
int tolower(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
int abs(int n) { return n < 0 ? -n : n; }
int rand(void) {
  static unsigned state = 1;
  state = state * 1103515245u + 12345u;
  return (int)((state >> 16) & 32767);
}
long strtol(const char *s, char **end, int base) {
  unsigned long n = 0;
  int negative = 0;
  while (isspace(*s))
    ++s;
  if (*s == '-' || *s == '+')
    negative = *s++ == '-';
  if (base != 10) {
    if (end)
      *end = (char *)s;
    return 0;
  }
  while (isdigit(*s))
    n = n * 10 + (unsigned)(*s++ - '0');
  if (end)
    *end = (char *)s;
  return negative ? -(long)n : (long)n;
}
__declspec(noreturn) void exit(int code) { ExitProcess((unsigned)code); }
clock_t clock(void) { return (clock_t)GetTickCount64(); }
/* Tiny Basic's exponent operands are signed integers. */
double pow(double base, double exponent) {
  int signed_power = (int)exponent;
  unsigned power =
      signed_power < 0 ? 0u - (unsigned)signed_power : (unsigned)signed_power;
  double value = 1;
  while (power) {
    if (power & 1)
      value *= base;
    base *= base;
    power >>= 1;
  }
  return signed_power < 0 ? 1 / value : value;
}
static int emit(void *handle, const char *p, size_t n) {
  while (n) {
    unsigned count = 0, take = n > 192 ? 192 : (unsigned)n;
    if (!WriteFile(handle, p, take, &count, NULL) || !count)
      return -1;
    p += count;
    n -= count;
  }
  return 0;
}
FILE *fopen(const char *path, const char *mode) {
  unsigned short wide[192];
  size_t n = strlen(path), i;
  void *h;
  if (n >= 192)
    return NULL;
  for (i = 0; i <= n; ++i) {
    if ((unsigned char)path[i] > 127)
      return NULL;
    wide[i] = (unsigned short)path[i];
  }
  h = CreateFileW(wide, mode[0] == 'w' ? 0x40000000u : 0x80000000u, 7, NULL,
                  mode[0] == 'w' ? 2 : 3, 0, NULL);
  if (h == (void *)-1)
    return NULL;
  for (i = 1; i < 5; ++i)
    if (!streams[i].used) {
      streams[i].used = 1;
      streams[i].terminal = 0;
      streams[i].handle = h;
      streams[i].writing = mode[0] == 'w';
      streams[i].position = streams[i].count = 0;
      return &streams[i];
    }
  CloseHandle(h);
  return NULL;
}
static int flush_stream(FILE *f) {
  unsigned position = 0;
  if (!f->writing)
    return 0;
  while (position < f->count) {
    unsigned n = 0;
    if (!WriteFile(f->handle, f->buffer + position, f->count - position, &n,
                   NULL) ||
        !n)
      return EOF;
    position += n;
  }
  f->count = 0;
  return 0;
}
int fclose(FILE *f) {
  int ok;
  if (!f || !f->used)
    return EOF;
  ok = flush_stream(f) == 0;
  if (!CloseHandle(f->handle))
    ok = 0;
  f->used = 0;
  return ok ? 0 : EOF;
}
int fgetc(FILE *f) {
  if (!f || !f->used || f->writing)
    return EOF;
  if (f->position == f->count) {
    f->position = f->count = 0;
    if (!ReadFile(f->handle, f->buffer, sizeof(f->buffer), &f->count, NULL) ||
        !f->count)
      return EOF;
  }
  return f->buffer[f->position++];
}
/* The upstream formats are %s, %d, %c, %*s, %*d, and %.2f. */
static int format(char *out, size_t cap, const char *fmt, va_list args) {
  size_t n = 0;
  while (*fmt) {
    char scratch[48], *text = scratch;
    int width = 0, len = 0;
    if (*fmt != '%') {
      if (n + 1 >= cap)
        return -1;
      out[n++] = *fmt++;
      continue;
    }
    ++fmt;
    if (*fmt == '*') {
      width = va_arg(args, int);
      ++fmt;
      if (width < 0 || width > 1024)
        return -1;
    }
    if (fmt[0] == '.' && fmt[1] == '2' && fmt[2] == 'f') {
      double value = va_arg(args, double);
      unsigned whole;
      fmt += 3;
      if (value < 0) {
        scratch[len++] = '-';
        value = -value;
      }
      whole = (unsigned)(value * 100 + 0.5);
      {
        unsigned a = whole / 100;
        char reverse[16];
        int k = 0;
        do {
          reverse[k++] = (char)('0' + a % 10);
          a /= 10;
        } while (a);
        while (k)
          scratch[len++] = reverse[--k];
      }
      scratch[len++] = '.';
      scratch[len++] = (char)('0' + whole / 10 % 10);
      scratch[len++] = (char)('0' + whole % 10);
      scratch[len] = 0;
    } else if (*fmt == 's') {
      text = va_arg(args, char *);
      if (!text)
        text = "(null)";
      len = (int)strlen(text);
      ++fmt;
    } else if (*fmt == 'd') {
      int v = va_arg(args, int);
      unsigned a = v < 0 ? 0u - (unsigned)v : (unsigned)v;
      char reverse[16];
      int k = 0;
      if (v < 0)
        scratch[len++] = '-';
      do {
        reverse[k++] = (char)('0' + a % 10);
        a /= 10;
      } while (a);
      while (k)
        scratch[len++] = reverse[--k];
      scratch[len] = 0;
      ++fmt;
    } else if (*fmt == 'c') {
      scratch[0] = (char)va_arg(args, int);
      len = 1;
      ++fmt;
    } else if (*fmt == '%') {
      scratch[0] = '%';
      len = 1;
      ++fmt;
    } else
      return -1;
    while (width > len) {
      if (n + 1 >= cap)
        return -1;
      out[n++] = ' ';
      --width;
    }
    if (n + (size_t)len >= cap)
      return -1;
    while (len--)
      out[n++] = *text++;
  }
  out[n] = 0;
  return (int)n;
}
int printf(const char *fmt, ...) {
  char out[2048];
  int n;
  va_list a;
  va_start(a, fmt);
  n = format(out, sizeof(out), fmt, a);
  va_end(a);
  if (n >= 0 && emit(GetStdHandle((unsigned)-11), out, (size_t)n) != 0)
    return -1;
  return n;
}
int fprintf(FILE *f, const char *fmt, ...) {
  char out[2048];
  int n;
  va_list a;
  va_start(a, fmt);
  n = format(out, sizeof(out), fmt, a);
  va_end(a);
  if (n >= 0) {
    int i;
    if (!f || !f->used || !f->writing)
      return -1;
    for (i = 0; i < n; ++i) {
      if (f->count == sizeof(f->buffer) && flush_stream(f) != 0)
        return -1;
      f->buffer[f->count++] = (unsigned char)out[i];
    }
  }
  return n;
}
int sprintf(char *out, const char *fmt, ...) {
  int n;
  va_list a;
  va_start(a, fmt);
  n = format(out, 1024, fmt, a);
  va_end(a);
  return n;
}
int puts(const char *s) {
  void *h = GetStdHandle((unsigned)-11);
  return emit(h, s, strlen(s)) || emit(h, "\n", 1) ? EOF : 0;
}
int main(int, char **);
void entry(void) {
  char command[512], *argv[16];
  int argc = 0;
  size_t i = 0;
  unsigned short *wide = GetCommandLineW();
  while (i < sizeof(command) - 1 && wide[i]) {
    if (wide[i] > 127)
      exit(64);
    command[i] = (char)wide[i];
    ++i;
  }
  command[i] = 0;
  {
    char *p = command;
    while (*p && argc < 16) {
      int quoted = 0;
      char *out;
      while (*p == ' ')
        ++p;
      if (!*p)
        break;
      argv[argc++] = out = p;
      while (*p && (quoted || *p != ' ')) {
        if (*p == '"') {
          quoted = !quoted;
          ++p;
        } else
          *out++ = *p++;
      }
      if (*p)
        ++p;
      *out = 0;
    }
  }
  streams[0].used = 1;
  streams[0].terminal = 1;
  streams[0].handle = GetStdHandle((unsigned)-10);
  exit(main(argc, argv));
}
