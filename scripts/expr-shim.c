/*
 * Minimal /bin/expr shim for jonerix builder use.
 *
 * toybox doesn't ship expr, and GNU coreutils is GPL. autoconf vintage
 * configure scripts (libffi, openntpd, sudo, jq, pico, tzdata, libevent)
 * make heavy use of `expr $x + 1` etc. for integer-feature probes. This
 * shim is good enough for those probes.
 *
 * Supported forms:
 *   expr length STRING              -> integer
 *   expr STRING : REGEX             -> integer (match length) or \1 match
 *   expr NUM OP NUM                 -> integer (+ - * / % = != < > <= >=)
 *   expr STRING                     -> STRING (pass-through)
 *
 * Exit status follows POSIX: 0 on a nonempty/nonzero result, 1 otherwise,
 * 2 on bad invocation. Keep it SIMPLE so autoconf doesn't discover
 * unsupported features and bail.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <regex.h>

static long tl(const char *s, int *ok) {
    char *e;
    long v = strtol(s, &e, 10);
    *ok = (*e == 0 && e != s);
    return v;
}

int main(int c, char **v) {
    /* POSIX expr supports `(expression)` grouping. Ruby's tool/ifchange
     * (and other autoconf-vintage scripts) use it for argument extraction:
     *   expr "(" "$1" : '[^=]*=\(.*\)' ")"
     * Strip one level of surrounding parens so the remaining args fall
     * into the normal 3/4-arg dispatch below. Nested parens aren't
     * supported — none of our build recipes need them. */
    if (c >= 4 && !strcmp(v[1], "(") && !strcmp(v[c-1], ")")) {
        for (int i = 1; i < c - 2; i++) v[i] = v[i+1];
        c -= 2;
    }

    if (c == 3 && !strcmp(v[1], "length")) {
        printf("%zu\n", strlen(v[2]));
        return 0;
    }
    if (c == 4 && !strcmp(v[2], ":")) {
        regex_t r;
        regmatch_t m[2];
        int has_group = !!strstr(v[3], "\\(");
        if (regcomp(&r, v[3], 0)) return 2;
        if (regexec(&r, v[1], 2, m, 0) == 0) {
            if (has_group && m[1].rm_so >= 0)
                printf("%.*s\n", (int)(m[1].rm_eo - m[1].rm_so), v[1] + m[1].rm_so);
            else
                printf("%d\n", (int)(m[0].rm_eo - m[0].rm_so));
            regfree(&r);
            return 0;
        }
        regfree(&r);
        printf(has_group ? "\n" : "0\n");
        return 1;
    }
    if (c == 4) {
        int o1, o2;
        long a = tl(v[1], &o1), b = tl(v[3], &o2);
        if (o1 && o2) {
            long r = 0;
            const char *o = v[2];
            if      (!strcmp(o, "+"))  r = a + b;
            else if (!strcmp(o, "-"))  r = a - b;
            else if (!strcmp(o, "*"))  r = a * b;
            else if (!strcmp(o, "/"))  r = a / b;
            else if (!strcmp(o, "%"))  r = a % b;
            else if (!strcmp(o, "="))  r = (a == b);
            else if (!strcmp(o, "!=")) r = (a != b);
            else if (!strcmp(o, "<"))  r = (a <  b);
            else if (!strcmp(o, ">"))  r = (a >  b);
            else if (!strcmp(o, "<=")) r = (a <= b);
            else if (!strcmp(o, ">=")) r = (a >= b);
            else return 2;
            printf("%ld\n", r);
            return r == 0 ? 1 : 0;
        }
    }
    if (c == 2) {
        printf("%s\n", v[1]);
        return 0;
    }
    return 2;
}
