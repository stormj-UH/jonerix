/*
 * jpkg - jonerix package manager
 * fetch.h - HTTPS downloads via LibreSSL
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#ifndef JPKG_FETCH_H
#define JPKG_FETCH_H

#include <stddef.h>
#include <stdint.h>

/* Downloaded data buffer */
typedef struct fetch_buf {
    uint8_t *data;
    size_t len;
    size_t capacity;
} fetch_buf_t;

/* Initialize the fetch subsystem (TLS context) */
int fetch_init(void);

/* Cleanup the fetch subsystem */
void fetch_cleanup(void);

/* Download a URL to memory. Returns 0 on success.
 * Caller must free buf->data. */
int fetch_to_buffer(const char *url, fetch_buf_t *buf);

/* Download a URL to a file. Returns 0 on success. */
int fetch_to_file(const char *url, const char *path);

/* Free a fetch buffer */
void fetch_buf_free(fetch_buf_t *buf);

#endif /* JPKG_FETCH_H */
