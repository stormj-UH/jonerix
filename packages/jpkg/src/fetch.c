/*
 * jpkg - jonerix package manager
 * fetch.c - HTTPS downloads via LibreSSL (or plain sockets for HTTP)
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 *
 * This implements a minimal HTTP/HTTPS client using raw sockets and
 * LibreSSL's TLS API. In production this links against LibreSSL.
 * For portability, we also support a fallback using the system's
 * 'curl' command if LibreSSL is not available at compile time.
 */

#include "fetch.h"
#include "util.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <netdb.h>
#include <errno.h>
#include <fcntl.h>

/*
 * We use a compile-time switch: if JPKG_USE_LIBRESSL is defined,
 * we use LibreSSL's tls(3) API. Otherwise, we shell out to curl
 * (which is always available on jonerix).
 */

/* ========== Buffer Management ========== */

static void buf_init(fetch_buf_t *buf) {
    buf->data = NULL;
    buf->len = 0;
    buf->capacity = 0;
}

static void buf_append(fetch_buf_t *buf, const uint8_t *data, size_t len) {
    if (buf->len + len > buf->capacity) {
        size_t new_cap = buf->capacity ? buf->capacity * 2 : 4096;
        while (new_cap < buf->len + len) new_cap *= 2;
        buf->data = xrealloc(buf->data, new_cap);
        buf->capacity = new_cap;
    }
    memcpy(buf->data + buf->len, data, len);
    buf->len += len;
}

void fetch_buf_free(fetch_buf_t *buf) {
    if (buf) {
        free(buf->data);
        buf->data = NULL;
        buf->len = 0;
        buf->capacity = 0;
    }
}

/* ========== URL Parsing ========== */

typedef struct {
    char scheme[16];
    char host[256];
    char port[8];
    char path[2048];
} parsed_url_t;

static int parse_url(const char *url, parsed_url_t *out) {
    memset(out, 0, sizeof(*out));

    /* Scheme */
    const char *p = strstr(url, "://");
    if (!p) {
        log_error("invalid URL (no scheme): %s", url);
        return -1;
    }
    size_t slen = (size_t)(p - url);
    if (slen >= sizeof(out->scheme)) slen = sizeof(out->scheme) - 1;
    memcpy(out->scheme, url, slen);
    out->scheme[slen] = '\0';
    p += 3;

    /* Default port */
    if (strcmp(out->scheme, "https") == 0)
        strcpy(out->port, "443");
    else
        strcpy(out->port, "80");

    /* Host (possibly with :port) */
    const char *slash = strchr(p, '/');
    const char *colon = strchr(p, ':');
    if (colon && (!slash || colon < slash)) {
        /* Explicit port */
        size_t hlen = (size_t)(colon - p);
        if (hlen >= sizeof(out->host)) hlen = sizeof(out->host) - 1;
        memcpy(out->host, p, hlen);
        out->host[hlen] = '\0';

        colon++;
        size_t plen = slash ? (size_t)(slash - colon) : strlen(colon);
        if (plen >= sizeof(out->port)) plen = sizeof(out->port) - 1;
        memcpy(out->port, colon, plen);
        out->port[plen] = '\0';
    } else {
        size_t hlen = slash ? (size_t)(slash - p) : strlen(p);
        if (hlen >= sizeof(out->host)) hlen = sizeof(out->host) - 1;
        memcpy(out->host, p, hlen);
        out->host[hlen] = '\0';
    }

    /* Path */
    if (slash) {
        strncpy(out->path, slash, sizeof(out->path) - 1);
        out->path[sizeof(out->path) - 1] = '\0';
    } else {
        strcpy(out->path, "/");
    }

    return 0;
}

/* ========== Plain Socket Helpers ========== */

static int tcp_connect(const char *host, const char *port) {
    struct addrinfo hints = {0}, *res, *rp;
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;

    int err = getaddrinfo(host, port, &hints, &res);
    if (err != 0) {
        log_error("DNS resolution failed for %s: %s", host, gai_strerror(err));
        return -1;
    }

    int fd = -1;
    for (rp = res; rp; rp = rp->ai_next) {
        fd = socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (fd < 0) continue;
        if (connect(fd, rp->ai_addr, rp->ai_addrlen) == 0) break;
        close(fd);
        fd = -1;
    }
    freeaddrinfo(res);

    if (fd < 0) {
        log_error("failed to connect to %s:%s", host, port);
    }
    return fd;
}

/* ========== HTTP Client (plain, no TLS) ========== */

static int http_fetch(const parsed_url_t *url, fetch_buf_t *buf) {
    int fd = tcp_connect(url->host, url->port);
    if (fd < 0) return -1;

    /* Send request */
    char req[4096];
    int rlen = snprintf(req, sizeof(req),
        "GET %s HTTP/1.0\r\n"
        "Host: %s\r\n"
        "User-Agent: jpkg/1.0\r\n"
        "Connection: close\r\n"
        "\r\n",
        url->path, url->host);

    if (write(fd, req, (size_t)rlen) != rlen) {
        log_error("failed to send HTTP request");
        close(fd);
        return -1;
    }

    /* Read response */
    fetch_buf_t raw;
    buf_init(&raw);
    uint8_t chunk[8192];
    ssize_t n;
    while ((n = read(fd, chunk, sizeof(chunk))) > 0) {
        buf_append(&raw, chunk, (size_t)n);
    }
    close(fd);

    /* Parse HTTP response: find \r\n\r\n */
    const char *hdr_end = NULL;
    for (size_t i = 0; i + 3 < raw.len; i++) {
        if (raw.data[i] == '\r' && raw.data[i+1] == '\n' &&
            raw.data[i+2] == '\r' && raw.data[i+3] == '\n') {
            hdr_end = (const char *)(raw.data + i + 4);
            break;
        }
    }

    if (!hdr_end) {
        log_error("malformed HTTP response (no header end)");
        fetch_buf_free(&raw);
        return -1;
    }

    /* Check status line */
    if (raw.len < 12 || memcmp(raw.data, "HTTP/", 5) != 0) {
        log_error("malformed HTTP response");
        fetch_buf_free(&raw);
        return -1;
    }

    /* Extract status code */
    const char *status_str = (const char *)raw.data + 9;
    int status = atoi(status_str);
    if (status < 200 || status >= 400) {
        log_error("HTTP %d fetching %s%s", status, url->host, url->path);
        fetch_buf_free(&raw);
        return -1;
    }

    /* Copy body to output buffer */
    size_t body_off = (size_t)((const uint8_t *)hdr_end - raw.data);
    size_t body_len = raw.len - body_off;
    buf_init(buf);
    buf_append(buf, raw.data + body_off, body_len);
    fetch_buf_free(&raw);

    return 0;
}

/* ========== HTTPS via curl fallback ========== */

/*
 * When LibreSSL is not linked at compile time (e.g., during development
 * or on non-jonerix hosts), we fall back to invoking curl.
 * On jonerix itself, curl is always available as a core package.
 */
static int https_fetch_curl(const char *url, fetch_buf_t *buf) {
    char tmppath[256];
    snprintf(tmppath, sizeof(tmppath), "/tmp/jpkg-fetch-%d.tmp", (int)getpid());

    char cmd[4096];
    snprintf(cmd, sizeof(cmd),
             "curl -fsSL --connect-timeout 30 --max-time 300 -o '%s' '%s' 2>/dev/null",
             tmppath, url);

    int rc = system(cmd);
    if (rc != 0) {
        log_error("curl failed for %s (exit %d)", url, rc);
        unlink(tmppath);
        return -1;
    }

    uint8_t *data;
    ssize_t len = file_read(tmppath, &data);
    unlink(tmppath);

    if (len < 0) {
        log_error("failed to read downloaded file");
        return -1;
    }

    buf->data = data;
    buf->len = (size_t)len;
    buf->capacity = (size_t)len;
    return 0;
}

#ifdef JPKG_USE_LIBRESSL
#include <tls.h>

static struct tls_config *g_tls_config = NULL;

int fetch_init(void) {
    if (tls_init() != 0) {
        log_error("TLS initialization failed");
        return -1;
    }
    g_tls_config = tls_config_new();
    if (!g_tls_config) {
        log_error("failed to create TLS config");
        return -1;
    }
    /* Load system CA certificates */
    char cafile[512];
    snprintf(cafile, sizeof(cafile), "%s/etc/ssl/cert.pem", g_rootfs);
    if (file_exists(cafile)) {
        tls_config_set_ca_file(g_tls_config, cafile);
    }
    return 0;
}

void fetch_cleanup(void) {
    if (g_tls_config) {
        tls_config_free(g_tls_config);
        g_tls_config = NULL;
    }
}

static int https_fetch_tls(const parsed_url_t *url, fetch_buf_t *buf) {
    struct tls *ctx = tls_client();
    if (!ctx) {
        log_error("failed to create TLS client");
        return -1;
    }
    if (tls_configure(ctx, g_tls_config) != 0) {
        log_error("TLS configure failed: %s", tls_error(ctx));
        tls_free(ctx);
        return -1;
    }
    if (tls_connect(ctx, url->host, url->port) != 0) {
        log_error("TLS connect failed: %s", tls_error(ctx));
        tls_free(ctx);
        return -1;
    }

    /* Send HTTP request */
    char req[4096];
    int rlen = snprintf(req, sizeof(req),
        "GET %s HTTP/1.0\r\n"
        "Host: %s\r\n"
        "User-Agent: jpkg/1.0\r\n"
        "Connection: close\r\n"
        "\r\n",
        url->path, url->host);

    ssize_t w = tls_write(ctx, req, (size_t)rlen);
    if (w < 0) {
        log_error("TLS write failed: %s", tls_error(ctx));
        tls_close(ctx);
        tls_free(ctx);
        return -1;
    }

    /* Read response */
    fetch_buf_t raw;
    buf_init(&raw);
    uint8_t chunk[8192];
    ssize_t n;
    while ((n = tls_read(ctx, chunk, sizeof(chunk))) > 0) {
        buf_append(&raw, chunk, (size_t)n);
    }
    if (n == TLS_WANT_POLLIN || n == TLS_WANT_POLLOUT) {
        /* Non-blocking retry needed - simplified for blocking mode */
    }

    tls_close(ctx);
    tls_free(ctx);

    /* Parse HTTP response */
    const char *hdr_end = NULL;
    for (size_t i = 0; i + 3 < raw.len; i++) {
        if (raw.data[i] == '\r' && raw.data[i+1] == '\n' &&
            raw.data[i+2] == '\r' && raw.data[i+3] == '\n') {
            hdr_end = (const char *)(raw.data + i + 4);
            break;
        }
    }

    if (!hdr_end) {
        log_error("malformed HTTPS response");
        fetch_buf_free(&raw);
        return -1;
    }

    /* Check status code */
    int status = 0;
    if (raw.len >= 12) {
        status = atoi((const char *)raw.data + 9);
    }
    if (status < 200 || status >= 400) {
        log_error("HTTPS %d fetching %s%s", status, url->host, url->path);
        fetch_buf_free(&raw);
        return -1;
    }

    size_t body_off = (size_t)((const uint8_t *)hdr_end - raw.data);
    size_t body_len = raw.len - body_off;
    buf_init(buf);
    buf_append(buf, raw.data + body_off, body_len);
    fetch_buf_free(&raw);

    return 0;
}

#else /* !JPKG_USE_LIBRESSL */

int fetch_init(void) {
    log_debug("fetch: using curl fallback (no LibreSSL)");
    return 0;
}

void fetch_cleanup(void) {
    /* Nothing to do without LibreSSL */
}

#endif /* JPKG_USE_LIBRESSL */

/* ========== Public API ========== */

int fetch_to_buffer(const char *url, fetch_buf_t *buf) {
    if (!url || !buf) return -1;

    buf_init(buf);
    log_debug("fetching: %s", url);

    parsed_url_t purl;
    if (parse_url(url, &purl) != 0) return -1;

    if (strcmp(purl.scheme, "http") == 0) {
        return http_fetch(&purl, buf);
    } else if (strcmp(purl.scheme, "https") == 0) {
#ifdef JPKG_USE_LIBRESSL
        return https_fetch_tls(&purl, buf);
#else
        /* Reconstruct URL for curl */
        return https_fetch_curl(url, buf);
#endif
    } else if (strcmp(purl.scheme, "file") == 0) {
        /* Local file URL */
        uint8_t *data;
        ssize_t len = file_read(purl.path, &data);
        if (len < 0) return -1;
        buf->data = data;
        buf->len = (size_t)len;
        buf->capacity = (size_t)len;
        return 0;
    }

    log_error("unsupported URL scheme: %s", purl.scheme);
    return -1;
}

int fetch_to_file(const char *url, const char *path) {
    fetch_buf_t buf;
    int rc = fetch_to_buffer(url, &buf);
    if (rc != 0) return -1;

    rc = file_write(path, buf.data, buf.len);
    fetch_buf_free(&buf);

    if (rc != 0) {
        log_error("failed to write fetched data to %s", path);
    }
    return rc;
}
