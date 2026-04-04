/*
 * jpkg - jonerix package manager
 * toml.c - Minimal TOML parser for package metadata
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Supports the subset of TOML used by jpkg:
 *   - [section] headers
 *   - key = "string value"
 *   - key = 12345  (integer)
 *   - key = true/false  (boolean)
 *   - key = ["a", "b", "c"]  (array of strings)
 *   - # comments
 *   - Multiline strings with triple quotes (basic only)
 */

#include "toml.h"
#include "util.h"
#include <ctype.h>
#include <string.h>
#include <stdio.h>

/* ========== Parser State ========== */

typedef struct {
    const char *input;
    const char *pos;
    int line;
    char section[256];   /* current [section] */
    char errbuf[512];
} parser_t;

static void parser_init(parser_t *p, const char *input) {
    p->input = input;
    p->pos = input;
    p->line = 1;
    p->section[0] = '\0';
    p->errbuf[0] = '\0';
}

static void parser_error(parser_t *p, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = snprintf(p->errbuf, sizeof(p->errbuf), "line %d: ", p->line);
    vsnprintf(p->errbuf + n, sizeof(p->errbuf) - (size_t)n, fmt, ap);
    va_end(ap);
}

/* ========== Lexer Helpers ========== */

static void skip_whitespace(parser_t *p) {
    while (*p->pos == ' ' || *p->pos == '\t') p->pos++;
}

static void skip_to_eol(parser_t *p) {
    while (*p->pos && *p->pos != '\n') p->pos++;
}

static void next_line(parser_t *p) {
    if (*p->pos == '\n') { p->pos++; p->line++; }
}

/* Parse an escape sequence in a string */
static int parse_escape(parser_t *p, char *out) {
    p->pos++; /* skip backslash */
    switch (*p->pos) {
        case '"':  *out = '"';  break;
        case '\\': *out = '\\'; break;
        case 'n':  *out = '\n'; break;
        case 't':  *out = '\t'; break;
        case 'r':  *out = '\r'; break;
        case 'b':  *out = '\b'; break;
        case 'f':  *out = '\f'; break;
        default:
            parser_error(p, "unknown escape: \\%c", *p->pos);
            return -1;
    }
    p->pos++;
    return 0;
}

/* Parse a quoted string (starting after the opening quote) */
static char *parse_string(parser_t *p) {
    if (*p->pos != '"') {
        parser_error(p, "expected '\"'");
        return NULL;
    }
    p->pos++;

    /* Check for triple-quote """...""" */
    bool triple = false;
    if (p->pos[0] == '"' && p->pos[1] == '"') {
        triple = true;
        p->pos += 2;
        /* Skip immediate newline after opening triple quote */
        if (*p->pos == '\n') { p->pos++; p->line++; }
    }

    size_t cap = 128, len = 0;
    char *buf = xmalloc(cap);

    while (*p->pos) {
        if (triple) {
            if (p->pos[0] == '"' && p->pos[1] == '"' && p->pos[2] == '"') {
                p->pos += 3;
                buf[len] = '\0';
                return buf;
            }
        } else {
            if (*p->pos == '"') {
                p->pos++;
                buf[len] = '\0';
                return buf;
            }
            if (*p->pos == '\n') {
                parser_error(p, "unterminated string");
                free(buf);
                return NULL;
            }
        }

        if (len + 4 >= cap) {
            cap *= 2;
            buf = xrealloc(buf, cap);
        }

        if (*p->pos == '\\' && !triple) {
            char esc;
            if (parse_escape(p, &esc) != 0) { free(buf); return NULL; }
            buf[len++] = esc;
        } else {
            if (*p->pos == '\n') p->line++;
            buf[len++] = *p->pos++;
        }
    }

    parser_error(p, "unterminated string");
    free(buf);
    return NULL;
}

/* Parse an integer */
static bool parse_integer(parser_t *p, int64_t *out) {
    const char *start = p->pos;
    bool neg = false;
    if (*p->pos == '-') { neg = true; p->pos++; }
    else if (*p->pos == '+') { p->pos++; }

    if (!isdigit((unsigned char)*p->pos)) {
        p->pos = start;
        return false;
    }

    int64_t val = 0;
    while (isdigit((unsigned char)*p->pos)) {
        val = val * 10 + (*p->pos - '0');
        p->pos++;
    }

    *out = neg ? -val : val;
    return true;
}

/* Parse a string array: ["a", "b", "c"] */
static toml_array_t parse_array(parser_t *p, bool *ok) {
    toml_array_t arr = {NULL, 0, 0};
    *ok = false;

    if (*p->pos != '[') {
        parser_error(p, "expected '['");
        return arr;
    }
    p->pos++;

    arr.capacity = 8;
    arr.items = xcalloc(arr.capacity, sizeof(char *));

    while (1) {
        skip_whitespace(p);
        /* Handle newlines inside arrays */
        while (*p->pos == '\n' || *p->pos == '#') {
            if (*p->pos == '#') skip_to_eol(p);
            if (*p->pos == '\n') { p->pos++; p->line++; }
            skip_whitespace(p);
        }

        if (*p->pos == ']') {
            p->pos++;
            *ok = true;
            return arr;
        }

        if (arr.count > 0) {
            if (*p->pos != ',') {
                parser_error(p, "expected ',' or ']' in array");
                return arr;
            }
            p->pos++;
            skip_whitespace(p);
            /* Handle newlines after comma */
            while (*p->pos == '\n' || *p->pos == '#') {
                if (*p->pos == '#') skip_to_eol(p);
                if (*p->pos == '\n') { p->pos++; p->line++; }
                skip_whitespace(p);
            }
            /* Allow trailing comma */
            if (*p->pos == ']') {
                p->pos++;
                *ok = true;
                return arr;
            }
        }

        char *item = parse_string(p);
        if (!item) return arr;

        if (arr.count >= arr.capacity) {
            arr.capacity *= 2;
            arr.items = xrealloc(arr.items, arr.capacity * sizeof(char *));
        }
        arr.items[arr.count++] = item;
    }
}

/* ========== Value Management ========== */

static toml_value_t *value_new(const char *section, const char *key) {
    toml_value_t *v = xcalloc(1, sizeof(toml_value_t));
    if (section[0]) {
        size_t slen = strlen(section);
        size_t klen = strlen(key);
        v->key = xmalloc(slen + 1 + klen + 1);
        memcpy(v->key, section, slen);
        v->key[slen] = '.';
        memcpy(v->key + slen + 1, key, klen);
        v->key[slen + 1 + klen] = '\0';
    } else {
        v->key = xstrdup(key);
    }
    return v;
}

static void value_free(toml_value_t *v) {
    if (!v) return;
    free(v->key);
    switch (v->type) {
        case TOML_STRING: free(v->v.string); break;
        case TOML_ARRAY:
            for (size_t i = 0; i < v->v.array.count; i++)
                free(v->v.array.items[i]);
            free(v->v.array.items);
            break;
        default: break;
    }
    free(v);
}

static void doc_add(toml_doc_t *doc, toml_value_t *v) {
    v->next = NULL;
    if (doc->tail) {
        doc->tail->next = v;
    } else {
        doc->head = v;
    }
    doc->tail = v;
    doc->count++;
}

/* ========== Public API ========== */

toml_doc_t *toml_parse(const char *input, char **err_msg) {
    parser_t p;
    parser_init(&p, input);

    toml_doc_t *doc = xcalloc(1, sizeof(toml_doc_t));

    while (*p.pos) {
        skip_whitespace(&p);

        /* Empty line or comment */
        if (*p.pos == '\n') { next_line(&p); continue; }
        if (*p.pos == '#') { skip_to_eol(&p); next_line(&p); continue; }
        if (*p.pos == '\0') break;

        /* Section header: [section] */
        if (*p.pos == '[') {
            p.pos++;
            /* Check for [[ (array of tables) - not supported but skip */
            if (*p.pos == '[') {
                parser_error(&p, "array of tables [[...]] not supported");
                goto error;
            }

            skip_whitespace(&p);
            char *s = p.section;
            size_t slen = 0;
            while (*p.pos && *p.pos != ']' && *p.pos != '\n' &&
                   slen < sizeof(p.section) - 1) {
                if (*p.pos == '.' || isalnum((unsigned char)*p.pos) ||
                    *p.pos == '_' || *p.pos == '-') {
                    s[slen++] = *p.pos++;
                } else {
                    parser_error(&p, "invalid char in section name: '%c'", *p.pos);
                    goto error;
                }
            }
            s[slen] = '\0';

            if (*p.pos != ']') {
                parser_error(&p, "expected ']'");
                goto error;
            }
            p.pos++;
            skip_whitespace(&p);
            if (*p.pos == '#') skip_to_eol(&p);
            if (*p.pos == '\n') next_line(&p);
            continue;
        }

        /* Key = Value */
        char keybuf[256];
        size_t klen = 0;
        while (*p.pos && *p.pos != '=' && *p.pos != ' ' && *p.pos != '\t' &&
               *p.pos != '\n' && klen < sizeof(keybuf) - 1) {
            if (isalnum((unsigned char)*p.pos) || *p.pos == '_' || *p.pos == '-') {
                keybuf[klen++] = *p.pos++;
            } else {
                parser_error(&p, "invalid char in key: '%c'", *p.pos);
                goto error;
            }
        }
        keybuf[klen] = '\0';

        if (klen == 0) {
            parser_error(&p, "empty key");
            goto error;
        }

        skip_whitespace(&p);
        if (*p.pos != '=') {
            parser_error(&p, "expected '=' after key '%s'", keybuf);
            goto error;
        }
        p.pos++;
        skip_whitespace(&p);

        toml_value_t *val = value_new(p.section, keybuf);

        /* Determine value type */
        if (*p.pos == '"') {
            /* String */
            char *s = parse_string(&p);
            if (!s) { value_free(val); goto error; }
            val->type = TOML_STRING;
            val->v.string = s;
        } else if (*p.pos == '[') {
            /* Array */
            bool ok;
            val->v.array = parse_array(&p, &ok);
            if (!ok) { value_free(val); goto error; }
            val->type = TOML_ARRAY;
        } else if (strncmp(p.pos, "true", 4) == 0 &&
                   !isalnum((unsigned char)p.pos[4])) {
            val->type = TOML_BOOLEAN;
            val->v.boolean = true;
            p.pos += 4;
        } else if (strncmp(p.pos, "false", 5) == 0 &&
                   !isalnum((unsigned char)p.pos[5])) {
            val->type = TOML_BOOLEAN;
            val->v.boolean = false;
            p.pos += 5;
        } else if (isdigit((unsigned char)*p.pos) || *p.pos == '-' || *p.pos == '+') {
            int64_t ival;
            if (!parse_integer(&p, &ival)) {
                parser_error(&p, "invalid integer");
                value_free(val);
                goto error;
            }
            val->type = TOML_INTEGER;
            val->v.integer = ival;
        } else {
            parser_error(&p, "unexpected value starting with '%c'", *p.pos);
            value_free(val);
            goto error;
        }

        doc_add(doc, val);

        /* Rest of line must be empty or comment */
        skip_whitespace(&p);
        if (*p.pos == '#') skip_to_eol(&p);
        if (*p.pos == '\n') next_line(&p);
        else if (*p.pos != '\0') {
            parser_error(&p, "unexpected content after value");
            goto error;
        }
    }

    return doc;

error:
    if (err_msg) *err_msg = xstrdup(p.errbuf);
    toml_free(doc);
    return NULL;
}

void toml_free(toml_doc_t *doc) {
    if (!doc) return;
    toml_value_t *v = doc->head;
    while (v) {
        toml_value_t *next = v->next;
        value_free(v);
        v = next;
    }
    free(doc);
}

static toml_value_t *find_value(const toml_doc_t *doc, const char *key) {
    if (!doc || !key) return NULL;
    for (toml_value_t *v = doc->head; v; v = v->next) {
        if (strcmp(v->key, key) == 0) return v;
    }
    return NULL;
}

const char *toml_get_string(const toml_doc_t *doc, const char *key) {
    toml_value_t *v = find_value(doc, key);
    return (v && v->type == TOML_STRING) ? v->v.string : NULL;
}

bool toml_get_integer(const toml_doc_t *doc, const char *key, int64_t *out) {
    toml_value_t *v = find_value(doc, key);
    if (v && v->type == TOML_INTEGER) { *out = v->v.integer; return true; }
    return false;
}

bool toml_get_boolean(const toml_doc_t *doc, const char *key, bool *out) {
    toml_value_t *v = find_value(doc, key);
    if (v && v->type == TOML_BOOLEAN) { *out = v->v.boolean; return true; }
    return false;
}

const toml_array_t *toml_get_array(const toml_doc_t *doc, const char *key) {
    toml_value_t *v = find_value(doc, key);
    return (v && v->type == TOML_ARRAY) ? &v->v.array : NULL;
}

bool toml_has_key(const toml_doc_t *doc, const char *key) {
    return find_value(doc, key) != NULL;
}

/* ========== Serialization ========== */

char *toml_serialize(const toml_doc_t *doc) {
    if (!doc) return xstrdup("");

    size_t cap = 1024, len = 0;
    char *buf = xmalloc(cap);
    buf[0] = '\0';

    char last_section[256] = "";

    for (toml_value_t *v = doc->head; v; v = v->next) {
        /* Determine section */
        char section[256] = "";
        const char *dot = strrchr(v->key, '.');
        const char *key_part;

        if (dot) {
            size_t slen = (size_t)(dot - v->key);
            if (slen >= sizeof(section)) slen = sizeof(section) - 1;
            memcpy(section, v->key, slen);
            section[slen] = '\0';
            key_part = dot + 1;
        } else {
            key_part = v->key;
        }

        /* Emit section header if changed */
        if (strcmp(section, last_section) != 0) {
            if (section[0]) {
                size_t needed = strlen(section) + 8;
                while (len + needed >= cap) { cap *= 2; buf = xrealloc(buf, cap); }
                len += (size_t)snprintf(buf + len, cap - len, "\n[%s]\n", section);
            }
            strncpy(last_section, section, sizeof(last_section) - 1);
            last_section[sizeof(last_section) - 1] = '\0';
        }

        /* Emit key = value */
        size_t needed = strlen(key_part) + 256;
        while (len + needed >= cap) { cap *= 2; buf = xrealloc(buf, cap); }

        switch (v->type) {
            case TOML_STRING:
                len += (size_t)snprintf(buf + len, cap - len, "%s = \"%s\"\n",
                                        key_part, v->v.string ? v->v.string : "");
                break;
            case TOML_INTEGER:
                len += (size_t)snprintf(buf + len, cap - len, "%s = %lld\n",
                                        key_part, (long long)v->v.integer);
                break;
            case TOML_BOOLEAN:
                len += (size_t)snprintf(buf + len, cap - len, "%s = %s\n",
                                        key_part, v->v.boolean ? "true" : "false");
                break;
            case TOML_ARRAY: {
                len += (size_t)snprintf(buf + len, cap - len, "%s = [", key_part);
                for (size_t i = 0; i < v->v.array.count; i++) {
                    needed = strlen(v->v.array.items[i]) + 8;
                    while (len + needed >= cap) { cap *= 2; buf = xrealloc(buf, cap); }
                    if (i > 0) {
                        len += (size_t)snprintf(buf + len, cap - len, ", ");
                    }
                    len += (size_t)snprintf(buf + len, cap - len, "\"%s\"",
                                            v->v.array.items[i]);
                }
                while (len + 4 >= cap) { cap *= 2; buf = xrealloc(buf, cap); }
                len += (size_t)snprintf(buf + len, cap - len, "]\n");
                break;
            }
        }
    }

    return buf;
}

/* ========== Builder ========== */

toml_doc_t *toml_new(void) {
    return xcalloc(1, sizeof(toml_doc_t));
}

void toml_set_string(toml_doc_t *doc, const char *key, const char *value) {
    /* Check if key already exists */
    toml_value_t *v = find_value(doc, key);
    if (v) {
        if (v->type == TOML_STRING) free(v->v.string);
        v->type = TOML_STRING;
        v->v.string = xstrdup(value);
        return;
    }
    v = xcalloc(1, sizeof(toml_value_t));
    v->key = xstrdup(key);
    v->type = TOML_STRING;
    v->v.string = xstrdup(value);
    doc_add(doc, v);
}

void toml_set_integer(toml_doc_t *doc, const char *key, int64_t value) {
    toml_value_t *v = find_value(doc, key);
    if (v) {
        v->type = TOML_INTEGER;
        v->v.integer = value;
        return;
    }
    v = xcalloc(1, sizeof(toml_value_t));
    v->key = xstrdup(key);
    v->type = TOML_INTEGER;
    v->v.integer = value;
    doc_add(doc, v);
}

void toml_set_boolean(toml_doc_t *doc, const char *key, bool value) {
    toml_value_t *v = find_value(doc, key);
    if (v) {
        v->type = TOML_BOOLEAN;
        v->v.boolean = value;
        return;
    }
    v = xcalloc(1, sizeof(toml_value_t));
    v->key = xstrdup(key);
    v->type = TOML_BOOLEAN;
    v->v.boolean = value;
    doc_add(doc, v);
}

void toml_set_array(toml_doc_t *doc, const char *key, const char **items, size_t count) {
    toml_value_t *v = find_value(doc, key);
    if (v && v->type == TOML_ARRAY) {
        for (size_t i = 0; i < v->v.array.count; i++) free(v->v.array.items[i]);
        free(v->v.array.items);
    } else if (v) {
        /* Type mismatch - just overwrite */
    } else {
        v = xcalloc(1, sizeof(toml_value_t));
        v->key = xstrdup(key);
        doc_add(doc, v);
    }
    v->type = TOML_ARRAY;
    v->v.array.count = count;
    v->v.array.capacity = count > 0 ? count : 1;
    v->v.array.items = xcalloc(v->v.array.capacity, sizeof(char *));
    for (size_t i = 0; i < count; i++) {
        v->v.array.items[i] = xstrdup(items[i]);
    }
}
