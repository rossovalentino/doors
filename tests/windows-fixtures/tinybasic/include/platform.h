#ifndef DOOR_TINYBASIC_PLATFORM_H
#define DOOR_TINYBASIC_PLATFORM_H
#include <stdarg.h>
#include <stddef.h>
#define bool _Bool
#define true 1
#define false 0
#define EOF (-1)
#define BUFSIZ 1024
#define CLOCKS_PER_SEC 1000
typedef long long clock_t;
typedef struct {
  void *handle;
  int used;
  int terminal;
  int writing;
  unsigned position, count;
  unsigned char buffer[1024];
} FILE;
extern FILE *stdin;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
int sprintf(char *, const char *, ...);
int puts(const char *);
FILE *fopen(const char *, const char *);
int fclose(FILE *);
int fgetc(FILE *);
void *malloc(size_t);
void *realloc(void *, size_t);
void free(void *);
__declspec(noreturn) void exit(int);
int abs(int);
int rand(void);
long strtol(const char *, char **, int);
size_t strlen(const char *);
int strcmp(const char *, const char *);
char *strcpy(char *, const char *);
char *strcat(char *, const char *);
char *strchr(const char *, int);
char *strdup(const char *);
int isalpha(int);
int isalnum(int);
int isdigit(int);
int isspace(int);
int tolower(int);
clock_t clock(void);
double pow(double, double);
#endif
