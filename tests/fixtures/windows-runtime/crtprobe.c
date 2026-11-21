/* CRT-A probe (P18 M0d): an ordinary C program built as .NET's native
 * binaries are (MSVC, the hybrid CRT: vcruntime linked statically, the
 * Universal CRT from ucrtbase.dll through the api-ms-win-crt-* contracts):
 *
 *   crtprobe.exe raw | check | guest
 *
 * Every step prints what the CRT computed; `check` compares with what
 * Windows 11 printed (`raw` on the owner's PC; `crtprobe.expected.h`).
 * `files` (guest only) writes, reads, seeks and removes a file in %TEMP%
 * and compares with what the C standard and Windows' text mode give
 * (each "\n" written as CR LF: 29 bytes). */
#define _CRT_SECURE_NO_WARNINGS
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <ctype.h>
#include <math.h>
#include <time.h>
#include <errno.h>
#include <locale.h>
#include <stdarg.h>

static char captured[16384];
static int captured_length, capturing;
static void out(const char *s) {
    if (capturing) {
        size_t n = strlen(s);
        if (captured_length + n < sizeof captured) {
            memcpy(captured + captured_length, s, n);
            captured_length += (int)n;
            captured[captured_length] = 0;
        }
        return;
    }
    fputs(s, stdout);
}
static void line(const char *format, ...) {
    char b[1024];
    va_list args;
    va_start(args, format);
    vsnprintf(b, sizeof b, format, args);
    va_end(args);
    out(b);
    out("\n");
}

static void printf_step(void) {
    char b[256];
    wchar_t w[128];
    int n;
    line("ints [%d] [%5d] [%-5d] [%05d] [%+d] [%x] [%#X] [%o] [%lld] [%llu] [%hd]", 42, -42, 7, -7, 3, 255, 255, 8,
         -5LL, 18446744073709551615ULL, 70000);
    line("floats [%f] [%.2f] [%e] [%.3E] [%g] [%g] [%G] [%.0f] [%.0f] [%a]", 3.14159, 2.005, 12345.678, 0.000123,
         100000.0, 1e-5, 1.5e-10, 0.5, 1.5, 1.0);
    line("strings [%s] [%8s] [%-8s] [%.3s] [%c] [%%]", "hi", "hi", "hi", "hello", 'Z');
    line("special [%f] [%f] [%e]", HUGE_VAL, -HUGE_VAL, sqrt(-1.0));
    n = snprintf(b, 8, "%s", "truncated output");
    line("snprintf %d [%s]", n, b);
    n = sprintf_s(b, sizeof b, "%d-%s", 7, "x");
    line("sprintf_s %d [%s]", n, b);
    n = _snprintf_s(b, 6, _TRUNCATE, "%s", "abcdefgh");
    line("_snprintf_s %d [%s]", n, b);
    n = swprintf(w, 128, L"%ls %d %.1f", L"wide", 5, 2.5);
    line("swprintf %d %d %d", n, w[0], w[n - 1]);
    n = _scprintf("%s%d", "count", 12345);
    line("_scprintf %d", n);
}

static void scan_step(void) {
    int a = 0, c = 0;
    unsigned x = 0;
    double d = 0;
    float f = 0;
    char s[16] = {0}, set[16] = {0};
    long long ll = 0;
    int n = sscanf("  -12 ff hello 2.5 1e3 abcx 99999999999", "%d %x %15s %lf %f %15[a-c]%*s %lld", &a, &x, s, &d, &f,
                   set, &ll);
    line("sscanf %d %d %u %s %.1f %.1f %s %lld", n, a, x, s, d, f, set, ll);
    n = sscanf("abc", "%d", &c);
    line("sscanf mismatch %d", n);
    n = sscanf("", "%d", &c);
    line("sscanf empty %d", n);
    n = sscanf_s("word 7", "%s %d", s, (unsigned)sizeof s, &c);
    line("sscanf_s %d %s %d", n, s, c);
}

static void convert_step(void) {
    char *end;
    errno = 0;
    line("strtol %ld %ld %ld", strtol("  -0x1fz", &end, 0), strtol("0755", NULL, 0), strtol("zz", NULL, 36));
    line("end %d", (int)(end - "  -0x1fz"));
    errno = 0;
    line("overflow %ld %d", strtol("99999999999999999999", NULL, 10), errno == ERANGE);
    line("strtoul %lu %llu", strtoul("-1", NULL, 10), strtoull("18446744073709551615", NULL, 10));
    line("strtod %.6f %.6f %g %g", strtod("3.25xyz", &end), strtod("-1e3", NULL), strtod("0x1.8p1", NULL),
         strtod("1e-320", NULL));
    line("atoi %d %ld %lld %.2f", atoi("  123abc"), atol("-77"), _atoi64("9876543210"), atof("2.75"));
    {
        char b[40];
        _itoa_s(-255, b, sizeof b, 16);
        line("_itoa_s %s", b);
        _ui64toa_s(18446744073709551615ULL, b, sizeof b, 10);
        line("_ui64toa_s %s", b);
    }
}

static int compare(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}
static void library_step(void) {
    int v[12] = {5, 3, 9, 1, 12, 7, 2, 8, 11, 4, 10, 6}, i, key = 8, *found;
    char b[64];
    char text[] = "a,b,,c";
    char *context = NULL, *token;
    qsort(v, 12, sizeof v[0], compare);
    b[0] = 0;
    for (i = 0; i < 12; i++) sprintf(b + strlen(b), "%d ", v[i]);
    line("qsort %s", b);
    found = (int *)bsearch(&key, v, 12, sizeof v[0], compare);
    line("bsearch %d", found ? *found : -1);
    b[0] = 0;
    for (token = strtok_s(text, ",", &context); token; token = strtok_s(NULL, ",", &context)) strcat(b, token);
    line("strtok_s %s", b);
    line("ctype %d %d %d %d %d", !!isalpha('a'), !!isdigit('7'), !!isspace('\t'), toupper('q'), !!iswupper(L'A'));
    line("cmp %d %d %d %d", strcmp("a", "b") < 0, _stricmp("ABC", "abc"), strncmp("abcd", "abce", 3),
         _wcsicmp(L"X", L"x"));
    line("math %.6f %.6f %.6f %.6f %.6f %.6f", sin(1.0), cos(1.0), exp(1.0), log(10.0), pow(2.0, 0.5), atan2(1.0, 2.0));
    line("math2 %.1f %.1f %.1f %.1f %.6f %.6f", floor(-1.5), ceil(-1.5), round(2.5), trunc(-2.7), fmod(7.5, 2.0),
         cbrt(27.0));
    line("div %d %d abs %d", div(17, 5).quot, div(17, 5).rem, abs(-9));
    srand(1);
    line("rand %d %d", rand(), rand());
    line("errno text [%s]", strerror(ENOENT));
}

static void environment_step(void) {
    char *value;
    size_t length = 0;
    _putenv_s("CRTPROBE_VALUE", "door 26");
    value = getenv("CRTPROBE_VALUE");
    line("getenv [%s]", value ? value : "(null)");
    line("getenv case [%s]", getenv("crtprobe_value") ? "found" : "missing");
    _dupenv_s(&value, &length, "CRTPROBE_VALUE");
    line("_dupenv_s [%s] %d", value ? value : "(null)", (int)length);
    free(value);
    _putenv("CRTPROBE_VALUE=");
    line("removed [%s]", getenv("CRTPROBE_VALUE") ? "present" : "gone");
    line("locale [%s]", setlocale(LC_ALL, NULL));
}

static void time_step(void) {
    struct tm tm;
    time_t t = 1700000000;
    char b[128];
    gmtime_s(&tm, &t);
    strftime(b, sizeof b, "%a %b %d %Y %H:%M:%S %j %p", &tm);
    line("gmtime %d %d %d %d [%s]", tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_wday, b);
    line("mkgmtime %lld", (long long)_mkgmtime(&tm));
    line("time sane %d", time(NULL) > 1700000000);
}

/* Guest only: a file in %TEMP% through stdio. */
static void files_step(void) {
    char path[512], b[64];
    FILE *f;
    long size;
    const char *temp = getenv("TEMP");
    snprintf(path, sizeof path, "%s\\crtprobe.txt", temp ? temp : ".");
    f = fopen(path, "w");
    if (!f) {
        line("fopen failed %d", errno);
        return;
    }
    fprintf(f, "line one %d\nline two\n", 1);
    fputs("third\n", f);
    fclose(f);
    f = fopen(path, "rb");
    fseek(f, 0, SEEK_END);
    size = ftell(f);
    rewind(f);
    line("size %ld", size);
    fclose(f);
    f = fopen(path, "r");
    fgets(b, sizeof b, f);
    line("first [%.*s]", (int)strcspn(b, "\n"), b);
    fgets(b, sizeof b, f);
    line("second [%.*s]", (int)strcspn(b, "\n"), b);
    line("eof %d", fgets(b, sizeof b, f) != NULL && fgets(b, sizeof b, f) == NULL && feof(f));
    fclose(f);
    line("remove %d", remove(path));
}

struct step {
    const char *name;
    void (*run)(void);
    int guest_only;
};
static const struct step steps[] = {{"printf", printf_step, 0},   {"scan", scan_step, 0},
                                    {"convert", convert_step, 0}, {"library", library_step, 0},
                                    {"environment", environment_step, 0}, {"time", time_step, 0},
                                    {"files", files_step, 1}};
static const char *const expected[] = {
#include "crtprobe.expected.h"
};
static const char files_expected[] = "size 29\nfirst [line one 1]\nsecond [line two]\neof 1\nremove 0\n";
static int line_length(const char *s) {
    int n = 0;
    while (s[n] && s[n] != '\r' && s[n] != '\n') n++;
    return n;
}
static int compare_text(const char *got, const char *want, const char **g, const char **w) {
    int at = 1;
    for (;;) {
        int a = line_length(got), b = line_length(want);
        if (!*got && !*want) return 0;
        if (a != b || memcmp(got, want, a) != 0) {
            *g = got;
            *w = want;
            return at;
        }
        got += a;
        want += b;
        while (*got == '\r' || *got == '\n') got++;
        while (*want == '\r' || *want == '\n') want++;
        at++;
    }
}
int main(int argc, char **argv) {
    int raw = argc > 1 && strcmp(argv[argc - 1], "raw") == 0, guest = argc > 1 && strcmp(argv[argc - 1], "guest") == 0;
    int failed = 0, i;
    for (i = 0; i < (int)(sizeof steps / sizeof steps[0]); i++) {
        if (steps[i].guest_only) {
            const char *g = 0, *w = 0;
            int at;
            if (!guest) continue;
            captured_length = 0;
            captured[0] = 0;
            capturing = 1;
            steps[i].run();
            capturing = 0;
            at = compare_text(captured, files_expected, &g, &w);
            if (at == 0) {
                printf("CRT-A %d %s ok\n", i + 1, steps[i].name);
            } else {
                printf("CRT-A %d %s FAILED line %d\ngot: %.*s\nexpected: %.*s\n", i + 1, steps[i].name, at,
                       line_length(g), g, line_length(w), w);
                failed = 1;
            }
            continue;
        }
        if (raw) {
            printf("== %s\n", steps[i].name);
            steps[i].run();
            continue;
        }
        {
            const char *g = 0, *w = 0;
            int at;
            captured_length = 0;
            captured[0] = 0;
            capturing = 1;
            steps[i].run();
            capturing = 0;
            at = compare_text(captured, expected[i], &g, &w);
            if (at == 0) {
                printf("CRT-A %d %s ok\n", i + 1, steps[i].name);
            } else {
                printf("CRT-A %d %s FAILED line %d\ngot: %.*s\nexpected: %.*s\n", i + 1, steps[i].name, at,
                       line_length(g), g, line_length(w), w);
                failed = 1;
            }
        }
    }
    if (raw) return 0;
    printf(failed ? "CRT-A FAILED\n" : "CRT-A PASSED\n");
    return failed;
}
