/*
 * jpkg - jonerix package manager
 * toml.h - Minimal TOML parser for package metadata
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#ifndef JPKG_TOML_H
#define JPKG_TOML_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* TOML value types */
typedef enum {
    TOML_STRING,
    TOML_INTEGER,
    TOML_BOOLEAN,
    TOML_ARRAY   /* array of strings only, sufficient for jpkg */
} toml_type_t;

/* A single string in an array */
typedef struct toml_array {
    char **items;
    size_t count;
    size_t capacity;
} toml_array_t;

/* A TOML key-value pair */
typedef struct toml_value {
    char *key;              /* fully qualified: "section.key" */
    toml_type_t type;
    union {
        char *string;
        int64_t integer;
        bool boolean;
        toml_array_t array;
    } v;
    struct toml_value *next;
} toml_value_t;

/* TOML document (linked list of values) */
typedef struct toml_doc {
    toml_value_t *head;
    toml_value_t *tail;
    size_t count;
} toml_doc_t;

/* Parse a TOML string. Returns NULL on error, sets *err_msg. */
toml_doc_t *toml_parse(const char *input, char **err_msg);

/* Free a parsed TOML document */
void toml_free(toml_doc_t *doc);

/* Lookup a string value: "package.name" -> "toybox" */
const char *toml_get_string(const toml_doc_t *doc, const char *key);

/* Lookup an integer value */
bool toml_get_integer(const toml_doc_t *doc, const char *key, int64_t *out);

/* Lookup a boolean value */
bool toml_get_boolean(const toml_doc_t *doc, const char *key, bool *out);

/* Lookup a string array */
const toml_array_t *toml_get_array(const toml_doc_t *doc, const char *key);

/* Check if a key exists */
bool toml_has_key(const toml_doc_t *doc, const char *key);

/* Serialize a TOML document to string (caller must free) */
char *toml_serialize(const toml_doc_t *doc);

/* Builder: create new empty document */
toml_doc_t *toml_new(void);

/* Builder: add a string value */
void toml_set_string(toml_doc_t *doc, const char *key, const char *value);

/* Builder: add an integer value */
void toml_set_integer(toml_doc_t *doc, const char *key, int64_t value);

/* Builder: add a boolean value */
void toml_set_boolean(toml_doc_t *doc, const char *key, bool value);

/* Builder: add a string array */
void toml_set_array(toml_doc_t *doc, const char *key, const char **items, size_t count);

#endif /* JPKG_TOML_H */
