/*
 * jpkg - jonerix package manager
 * fetch.c - HTTPS downloads via LibreSSL (or plain sockets for HTTP)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * This implements a minimal HTTP/HTTPS client using raw sockets and
 * LibreSSL's TLS API. In production this links against LibreSSL.
 * For portability, we also support a fallback using the system's
 * 'curl' command if LibreSSL is not available at compile time.
 */

#include "fetch.h"
#include "util.h"
#include <string.h>
#include <strings.h>
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
    /* Find curl binary — use absolute path for jonerix FROM-scratch images
     * where PATH may not be set in all contexts (e.g. system() via /bin/sh) */
    const char *curl_bin = NULL;
    if (access("/bin/curl", X_OK) == 0)
        curl_bin = "/bin/curl";
    else if (access("/usr/bin/curl", X_OK) == 0)
        curl_bin = "/usr/bin/curl";

    if (!curl_bin) {
        log_error("curl not found (/bin/curl, /usr/bin/curl) — "
                  "HTTPS requires curl when jpkg is built without LibreSSL");
        return -1;
    }

    char tmppath[256];
    snprintf(tmppath, sizeof(tmppath), "/tmp/jpkg-fetch-%d.tmp", (int)getpid());

    char cmd[4096];
    snprintf(cmd, sizeof(cmd),
             "%s -fsSL --connect-timeout 30 --max-time 3600 -o '%s' '%s' 2>/dev/null",
             curl_bin, tmppath, url);

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
    /* Load system CA certificates from the HOST filesystem.
     * Do NOT use g_rootfs here — CA certs are needed by the running
     * jpkg process, not by the target rootfs. When --root is set,
     * the target rootfs may not have certs installed yet. */
    static const char *ca_paths[] = {
        "/etc/ssl/cert.pem",
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        NULL
    };
    for (int i = 0; ca_paths[i]; i++) {
        if (file_exists(ca_paths[i])) {
            tls_config_set_ca_file(g_tls_config, ca_paths[i]);
            break;
        }
    }
    return 0;
}

void fetch_cleanup(void) {
    if (g_tls_config) {
        tls_config_free(g_tls_config);
        g_tls_config = NULL;
    }
}

/* Write all bytes, retrying on TLS_WANT_POLLIN/TLS_WANT_POLLOUT */
static ssize_t tls_write_all(struct tls *ctx, const void *data, size_t len) {
    size_t off = 0;
    while (off < len) {
        ssize_t w = tls_write(ctx, (const uint8_t *)data + off, len - off);
        if (w == TLS_WANT_POLLIN || w == TLS_WANT_POLLOUT)
            continue;
        if (w < 0)
            return -1;
        off += (size_t)w;
    }
    return (ssize_t)off;
}

/* Read until EOF, retrying on TLS_WANT_POLLIN/TLS_WANT_POLLOUT */
static int tls_read_all(struct tls *ctx, fetch_buf_t *out) {
    uint8_t chunk[8192];
    for (;;) {
        ssize_t n = tls_read(ctx, chunk, sizeof(chunk));
        if (n == TLS_WANT_POLLIN || n == TLS_WANT_POLLOUT)
            continue;
        if (n == 0)
            break;
        if (n < 0)
            return -1;
        buf_append(out, chunk, (size_t)n);
    }
    return 0;
}

/* Case-insensitive header search: extracts value for "Name: value\r\n".
 * Returns pointer into headers (not NUL-terminated) and sets *vlen. */
static const char *find_header_value(const char *headers, size_t hdr_len,
                                     const char *name, size_t *vlen) {
    size_t nlen = strlen(name);
    const char *p = headers;
    const char *end = headers + hdr_len;
    while (p < end) {
        const char *eol = NULL;
        for (const char *s = p; s + 1 < end; s++) {
            if (s[0] == '\r' && s[1] == '\n') { eol = s; break; }
        }
        if (!eol) break;
        /* Check "Name:" prefix (case-insensitive) */
        if ((size_t)(eol - p) > nlen + 1 && p[nlen] == ':' &&
            strncasecmp(p, name, nlen) == 0) {
            const char *v = p + nlen + 1;
            while (v < eol && *v == ' ') v++;
            *vlen = (size_t)(eol - v);
            return v;
        }
        p = eol + 2;
    }
    return NULL;
}

#define MAX_REDIRECTS 5

static int https_fetch_tls(const parsed_url_t *url, fetch_buf_t *buf) {
    parsed_url_t current = *url;

    for (int redir = 0; redir <= MAX_REDIRECTS; redir++) {
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
        if (tls_connect(ctx, current.host, current.port) != 0) {
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
            current.path, current.host);

        if (tls_write_all(ctx, req, (size_t)rlen) < 0) {
            log_error("TLS write failed: %s", tls_error(ctx));
            tls_close(ctx);
            tls_free(ctx);
            return -1;
        }

        /* Read full response */
        fetch_buf_t raw;
        buf_init(&raw);
        if (tls_read_all(ctx, &raw) < 0) {
            log_error("TLS read failed: %s", tls_error(ctx));
            tls_close(ctx);
            tls_free(ctx);
            fetch_buf_free(&raw);
            return -1;
        }

        tls_close(ctx);
        tls_free(ctx);

        /* Find end of headers */
        const char *hdr_end = NULL;
        size_t hdr_len = 0;
        for (size_t i = 0; i + 3 < raw.len; i++) {
            if (raw.data[i] == '\r' && raw.data[i+1] == '\n' &&
                raw.data[i+2] == '\r' && raw.data[i+3] == '\n') {
                hdr_len = i;
                hdr_end = (const char *)(raw.data + i + 4);
                break;
            }
        }

        if (!hdr_end) {
            log_error("malformed HTTPS response (no header end, got %zu bytes)",
                      raw.len);
            fetch_buf_free(&raw);
            return -1;
        }

        /* Parse status code */
        int status = 0;
        if (raw.len >= 12) {
            status = atoi((const char *)raw.data + 9);
        }

        /* Handle redirects */
        if (status == 301 || status == 302 || status == 307 || status == 308) {
            size_t loc_len = 0;
            const char *loc = find_header_value(
                (const char *)raw.data, hdr_len, "Location", &loc_len);
            if (!loc || loc_len == 0) {
                log_error("HTTPS %d redirect with no Location header", status);
                fetch_buf_free(&raw);
                return -1;
            }
            char loc_url[2048];
            if (loc_len >= sizeof(loc_url)) loc_len = sizeof(loc_url) - 1;
            memcpy(loc_url, loc, loc_len);
            loc_url[loc_len] = '\0';

            log_debug("redirect %d -> %s", status, loc_url);
            fetch_buf_free(&raw);

            if (parse_url(loc_url, &current) != 0) {
                log_error("failed to parse redirect URL: %s", loc_url);
                return -1;
            }
            continue;
        }

        if (status < 200 || status >= 400) {
            log_error("HTTPS %d fetching %s%s", status, current.host, current.path);
            fetch_buf_free(&raw);
            return -1;
        }

        /* Extract body */
        size_t body_off = (size_t)((const uint8_t *)hdr_end - raw.data);
        size_t body_len = raw.len - body_off;
        buf_init(buf);
        buf_append(buf, raw.data + body_off, body_len);
        fetch_buf_free(&raw);
        return 0;
    }

    log_error("too many redirects (max %d)", MAX_REDIRECTS);
    return -1;
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
    /* Retry on transient failures. GitHub releases CDN serves the
     * occasional HTTP 502 / 504 / TLS handshake failure under load (we
     * saw both in container-images runs that pulled musl/gitoxide jpkgs
     * at minute marks coinciding with GHA build storms). The current
     * fetch_to_buffer() path returns a single int -1 for both transient
     * and permanent failures, so we retry on any -1 with a short backoff.
     * Permanent failures (404, malformed URL) cost one extra round-trip
     * each -- acceptable. Three attempts with 2s/5s waits cover real
     * transient outages without dragging out a permanent miss. */
    const int max_attempts = 3;
    const unsigned int backoff_secs[] = {2, 5};
    fetch_buf_t buf;
    int rc = -1;

    for (int attempt = 1; attempt <= max_attempts; attempt++) {
        rc = fetch_to_buffer(url, &buf);
        if (rc == 0) break;
        if (attempt < max_attempts) {
            unsigned int wait = backoff_secs[attempt - 1];
            log_warn("fetch attempt %d/%d failed for %s — retrying in %us",
                     attempt, max_attempts, url, wait);
            sleep(wait);
        }
    }
    if (rc != 0) return -1;

    rc = file_write(path, buf.data, buf.len);
    fetch_buf_free(&buf);

    if (rc != 0) {
        log_error("failed to write fetched data to %s", path);
    }
    return rc;
}
