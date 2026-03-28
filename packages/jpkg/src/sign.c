/*
 * jpkg - jonerix package manager
 * sign.c - Ed25519 signature verification using tweetnacl
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 *
 * This file embeds a minimal Ed25519 implementation derived from
 * tweetnacl (public domain, Daniel J. Bernstein et al.).
 */

#include "sign.h"
#include "util.h"
#include <string.h>
#include <stdio.h>
#include <dirent.h>
#include <fcntl.h>
#include <unistd.h>

/* ========== TweetNaCl Ed25519 (public domain) ========== */

/*
 * Minimal Ed25519 sign/verify from tweetnacl.
 * This is a complete, standalone implementation.
 * Original code by Daniel J. Bernstein, Tanja Lange, Peter Schwabe.
 * Public domain.
 *
 * We only need: crypto_sign_open (verify) and crypto_sign (sign).
 * The SHA-512 used internally is also included.
 */

typedef int64_t gf[16];
typedef uint64_t u64;
typedef int64_t i64;

static const gf gf0 = {0};
static const gf gf1 = {1};
static const gf D = {0x78a3, 0x1359, 0x4dca, 0x75eb, 0xd8ab, 0x4141, 0x0a4d, 0x0070,
                     0xe898, 0x7779, 0x4079, 0x8cc7, 0xfe73, 0x2b6f, 0x6cee, 0x5203};
static const gf D2 = {0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0,
                      0xd130, 0xeef3, 0x80f2, 0x198e, 0xfce7, 0x56df, 0xd9dc, 0x2406};
static const gf X = {0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c,
                     0xdc5c, 0xfdd6, 0xe231, 0xc0a4, 0x53fe, 0xcd6e, 0x36d3, 0x2169};
static const gf Y = {0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666,
                     0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666};
static const gf I = {0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43,
                     0xd7a7, 0x3dfb, 0x0099, 0x2b4d, 0xdf0b, 0x4fc1, 0x2480, 0x2b83};

static u64 dl64(const uint8_t *x) {
    u64 u = 0;
    for (int i = 0; i < 8; i++) u = (u << 8) | x[i];
    return u;
}

static void ts64(uint8_t *x, u64 u) {
    for (int i = 7; i >= 0; i--) { x[i] = (uint8_t)(u & 0xff); u >>= 8; }
}

static int vn(const uint8_t *x, const uint8_t *y, int n) {
    uint32_t d = 0;
    for (int i = 0; i < n; i++) d |= x[i] ^ y[i];
    return (1 & ((d - 1) >> 8)) - 1;
}

static int crypto_verify_32(const uint8_t *x, const uint8_t *y) {
    return vn(x, y, 32);
}

static void set25519(gf r, const gf a) {
    for (int i = 0; i < 16; i++) r[i] = a[i];
}

static void car25519(gf o) {
    i64 c;
    for (int i = 0; i < 16; i++) {
        o[i] += (1LL << 16);
        c = o[i] >> 16;
        o[(i+1) * (i < 15)] += c - 1 + 37 * (c - 1) * (i == 15);
        o[i] -= c << 16;
    }
}

static void sel25519(gf p, gf q, int b) {
    i64 t, c = ~(b - 1);
    for (int i = 0; i < 16; i++) {
        t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

static void pack25519(uint8_t *o, const gf n) {
    gf m, t;
    set25519(t, n);
    car25519(t);
    car25519(t);
    car25519(t);
    for (int j = 0; j < 2; j++) {
        m[0] = t[0] - 0xffed;
        for (int i = 1; i < 15; i++) {
            m[i] = t[i] - 0xffff - ((m[i-1] >> 16) & 1);
            m[i-1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        int b = (int)((m[15] >> 63) & 1);
        m[14] &= 0xffff;
        sel25519(t, m, 1 - b);
    }
    for (int i = 0; i < 16; i++) {
        o[2*i] = (uint8_t)(t[i] & 0xff);
        o[2*i+1] = (uint8_t)(t[i] >> 8);
    }
}

static int neq25519(const gf a, const gf b) {
    uint8_t c[32], d[32];
    pack25519(c, a);
    pack25519(d, b);
    return crypto_verify_32(c, d);
}

static uint8_t par25519(const gf a) {
    uint8_t d[32];
    pack25519(d, a);
    return d[0] & 1;
}

static void unpack25519(gf o, const uint8_t *n) {
    for (int i = 0; i < 16; i++) o[i] = n[2*i] + ((i64)n[2*i+1] << 8);
    o[15] &= 0x7fff;
}

static void A(gf o, const gf a, const gf b) {
    for (int i = 0; i < 16; i++) o[i] = a[i] + b[i];
}

static void Z(gf o, const gf a, const gf b) {
    for (int i = 0; i < 16; i++) o[i] = a[i] - b[i];
}

static void M(gf o, const gf a, const gf b) {
    i64 t[31] = {0};
    for (int i = 0; i < 16; i++)
        for (int j = 0; j < 16; j++)
            t[i+j] += a[i] * b[j];
    for (int i = 0; i < 15; i++) t[i] += 38 * t[i+16];
    for (int i = 0; i < 16; i++) o[i] = t[i];
    car25519(o);
    car25519(o);
}

static void S(gf o, const gf a) {
    M(o, a, a);
}

static void inv25519(gf o, const gf a) {
    gf c;
    set25519(c, a);
    for (int i = 253; i >= 0; i--) {
        S(c, c);
        if (i != 2 && i != 4) M(c, c, a);
    }
    set25519(o, c);
}

static void pow2523(gf o, const gf i_val) {
    gf c;
    set25519(c, i_val);
    for (int a = 250; a >= 0; a--) {
        S(c, c);
        if (a != 1) M(c, c, i_val);
    }
    set25519(o, c);
}

/* SHA-512 (needed internally by Ed25519) */
static const u64 K512[80] = {
    0x428a2f98d728ae22ULL, 0x7137449123ef65cdULL, 0xb5c0fbcfec4d3b2fULL, 0xe9b5dba58189dbbcULL,
    0x3956c25bf348b538ULL, 0x59f111f1b605d019ULL, 0x923f82a4af194f9bULL, 0xab1c5ed5da6d8118ULL,
    0xd807aa98a3030242ULL, 0x12835b0145706fbeULL, 0x243185be4ee4b28cULL, 0x550c7dc3d5ffb4e2ULL,
    0x72be5d74f27b896fULL, 0x80deb1fe3b1696b1ULL, 0x9bdc06a725c71235ULL, 0xc19bf174cf692694ULL,
    0xe49b69c19ef14ad2ULL, 0xefbe4786384f25e3ULL, 0x0fc19dc68b8cd5b5ULL, 0x240ca1cc77ac9c65ULL,
    0x2de92c6f592b0275ULL, 0x4a7484aa6ea6e483ULL, 0x5cb0a9dcbd41fbd4ULL, 0x76f988da831153b5ULL,
    0x983e5152ee66dfabULL, 0xa831c66d2db43210ULL, 0xb00327c898fb213fULL, 0xbf597fc7beef0ee4ULL,
    0xc6e00bf33da88fc2ULL, 0xd5a79147930aa725ULL, 0x06ca6351e003826fULL, 0x142929670a0e6e70ULL,
    0x27b70a8546d22ffcULL, 0x2e1b21385c26c926ULL, 0x4d2c6dfc5ac42aedULL, 0x53380d139d95b3dfULL,
    0x650a73548baf63deULL, 0x766a0abb3c77b2a8ULL, 0x81c2c92e47edaee6ULL, 0x92722c851482353bULL,
    0xa2bfe8a14cf10364ULL, 0xa81a664bbc423001ULL, 0xc24b8b70d0f89791ULL, 0xc76c51a30654be30ULL,
    0xd192e819d6ef5218ULL, 0xd69906245565a910ULL, 0xf40e35855771202aULL, 0x106aa07032bbd1b8ULL,
    0x19a4c116b8d2d0c8ULL, 0x1e376c085141ab53ULL, 0x2748774cdf8eeb99ULL, 0x34b0bcb5e19b48a8ULL,
    0x391c0cb3c5c95a63ULL, 0x4ed8aa4ae3418acbULL, 0x5b9cca4f7763e373ULL, 0x682e6ff3d6b2b8a3ULL,
    0x748f82ee5defb2fcULL, 0x78a5636f43172f60ULL, 0x84c87814a1f0ab72ULL, 0x8cc702081a6439ecULL,
    0x90befffa23631e28ULL, 0xa4506cebde82bde9ULL, 0xbef9a3f7b2c67915ULL, 0xc67178f2e372532bULL,
    0xca273eceea26619cULL, 0xd186b8c721c0c207ULL, 0xeada7dd6cde0eb1eULL, 0xf57d4f7fee6ed178ULL,
    0x06f067aa72176fbaULL, 0x0a637dc5a2c898a6ULL, 0x113f9804bef90daeULL, 0x1b710b35131c471bULL,
    0x28db77f523047d84ULL, 0x32caab7b40c72493ULL, 0x3c9ebe0a15c9bebcULL, 0x431d67c49c100d4cULL,
    0x4cc5d4becb3e42b6ULL, 0x597f299cfc657e2aULL, 0x5fcb6fab3ad6faecULL, 0x6c44198c4a475817ULL
};

#define R512(x,c) (((x) >> (c)) | ((x) << (64 - (c))))
#define Ch512(x,y,z) ((x & y) ^ (~x & z))
#define Maj512(x,y,z) ((x & y) ^ (x & z) ^ (y & z))
#define Sigma0(x) (R512(x,28) ^ R512(x,34) ^ R512(x,39))
#define Sigma1(x) (R512(x,14) ^ R512(x,18) ^ R512(x,41))
#define sigma0(x) (R512(x,1) ^ R512(x,8) ^ (x >> 7))
#define sigma1(x) (R512(x,19) ^ R512(x,61) ^ (x >> 6))

static int crypto_hashblocks(uint8_t *x, const uint8_t *m, u64 n) {
    u64 z[8], b[8], a[8], w[16];
    for (int i = 0; i < 8; i++) z[i] = a[i] = dl64(x + 8*i);
    while (n >= 128) {
        for (int i = 0; i < 16; i++) w[i] = dl64(m + 8*i);
        for (int i = 0; i < 80; i++) {
            for (int j = 0; j < 8; j++) b[j] = a[j];
            u64 t = a[7] + Sigma1(a[4]) + Ch512(a[4],a[5],a[6]) + K512[i] + w[i%16];
            b[7] = t + Sigma0(a[0]) + Maj512(a[0],a[1],a[2]);
            b[3] += t;
            for (int j = 0; j < 8; j++) a[(j+1)%8] = b[j];
            if (i%16 == 15)
                for (int j = 0; j < 16; j++)
                    w[j] += w[(j+9)%16] + sigma0(w[(j+1)%16]) + sigma1(w[(j+14)%16]);
        }
        for (int i = 0; i < 8; i++) { a[i] += z[i]; z[i] = a[i]; }
        m += 128; n -= 128;
    }
    for (int i = 0; i < 8; i++) ts64(x + 8*i, z[i]);
    return (int)n;
}

static const uint8_t iv512[64] = {
    0x6a,0x09,0xe6,0x67,0xf3,0xbc,0xc9,0x08,
    0xbb,0x67,0xae,0x85,0x84,0xca,0xa7,0x3b,
    0x3c,0x6e,0xf3,0x72,0xfe,0x94,0xf8,0x2b,
    0xa5,0x4f,0xf5,0x3a,0x5f,0x1d,0x36,0xf1,
    0x51,0x0e,0x52,0x7f,0xad,0xe6,0x82,0xd1,
    0x9b,0x05,0x68,0x8c,0x2b,0x3e,0x6c,0x1f,
    0x1f,0x83,0xd9,0xab,0xfb,0x41,0xbd,0x6b,
    0x5b,0xe0,0xcd,0x19,0x13,0x7e,0x21,0x79
};

static int crypto_hash(uint8_t *out, const uint8_t *m, u64 n) {
    uint8_t h[64], x[256];
    u64 b = n;

    memcpy(h, iv512, 64);

    crypto_hashblocks(h, m, n);
    m += n;
    n &= 127;
    m -= n;

    memset(x, 0, 256);
    memcpy(x, m, (size_t)n);
    x[n] = 128;

    n = 256 - 128 * (n < 112);
    x[n-9] = (uint8_t)(b >> 61);
    ts64(x + n - 8, b << 3);
    crypto_hashblocks(h, x, n);

    memcpy(out, h, 64);
    return 0;
}

static void add(gf p[4], gf q[4]) {
    gf a, b, c, d, t, e, f, g, h;
    Z(a, p[1], p[0]);
    Z(t, q[1], q[0]);
    M(a, a, t);
    A(b, p[0], p[1]);
    A(t, q[0], q[1]);
    M(b, b, t);
    M(c, p[3], q[3]);
    M(c, c, D2);
    M(d, p[2], q[2]);
    A(d, d, d);
    Z(e, b, a);
    Z(f, d, c);
    A(g, d, c);
    A(h, b, a);

    M(p[0], e, f);
    M(p[1], h, g);
    M(p[2], g, f);
    M(p[3], e, h);
}

static void cswap(gf p[4], gf q[4], uint8_t b) {
    for (int i = 0; i < 4; i++) sel25519(p[i], q[i], b);
}

static void pack(uint8_t *r, gf p[4]) {
    gf tx, ty, zi;
    inv25519(zi, p[2]);
    M(tx, p[0], zi);
    M(ty, p[1], zi);
    pack25519(r, ty);
    r[31] ^= par25519(tx) << 7;
}

static void scalarmult(gf p[4], gf q[4], const uint8_t *s) {
    set25519(p[0], gf0);
    set25519(p[1], gf1);
    set25519(p[2], gf1);
    set25519(p[3], gf0);
    for (int i = 255; i >= 0; --i) {
        uint8_t b = (s[i/8] >> (i & 7)) & 1;
        cswap(p, q, b);
        add(q, p);
        add(p, p);
        cswap(p, q, b);
    }
}

static void scalarbase(gf p[4], const uint8_t *s) {
    gf q[4];
    set25519(q[0], X);
    set25519(q[1], Y);
    set25519(q[2], gf1);
    M(q[3], X, Y);
    scalarmult(p, q, s);
}

static int unpackneg(gf r[4], const uint8_t p[32]) {
    gf t, chk, num, den, den2, den4, den6;
    set25519(r[2], gf1);
    unpack25519(r[1], p);
    S(num, r[1]);
    M(den, num, D);
    Z(num, num, r[2]);
    A(den, r[2], den);

    S(den2, den);
    S(den4, den2);
    M(den6, den4, den2);
    M(t, den6, num);
    M(t, t, den);

    pow2523(t, t);
    M(t, t, num);
    M(t, t, den);
    M(t, t, den);
    M(r[0], t, den);

    S(chk, r[0]);
    M(chk, chk, den);
    if (neq25519(chk, num)) M(r[0], r[0], I);

    S(chk, r[0]);
    M(chk, chk, den);
    if (neq25519(chk, num)) return -1;

    if (par25519(r[0]) == (p[31] >> 7)) Z(r[0], gf0, r[0]);

    M(r[3], r[0], r[1]);
    return 0;
}

static const i64 L[32] = {
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58,
    0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0,    0,    0,    0,    0,    0,    0,    0,
    0,    0,    0,    0,    0,    0,    0,    0x10
};

static void modL(uint8_t *r, i64 x[64]) {
    i64 carry;
    for (int i = 63; i >= 32; --i) {
        carry = 0;
        for (int j = i - 32; j < i - 12; ++j) {
            x[j] += carry - 16 * x[i] * L[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= carry << 8;
        }
        x[i - 12] += carry;
        x[i] = 0;
    }
    carry = 0;
    for (int j = 0; j < 32; ++j) {
        x[j] += carry - (x[31] >> 4) * L[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for (int j = 0; j < 32; ++j) x[j] -= carry * L[j];
    for (int i = 0; i < 32; ++i) {
        x[i+1] += x[i] >> 8;
        r[i] = (uint8_t)(x[i] & 255);
    }
}

static void reduce(uint8_t *r) {
    i64 x[64];
    for (int i = 0; i < 64; ++i) x[i] = (u64)r[i];
    for (int i = 0; i < 64; ++i) r[i] = 0;
    modL(r, x);
}

static int crypto_sign_open(uint8_t *m, i64 *mlen,
                            const uint8_t *sm, u64 n,
                            const uint8_t *pk) {
    uint8_t t[32], h[64];
    gf p[4], q[4];

    *mlen = -1;
    if (n < 64) return -1;
    if (unpackneg(q, pk)) return -1;

    memcpy(m, sm, (size_t)n);
    memcpy(m + 32, pk, 32);
    crypto_hash(h, m, n);
    reduce(h);
    scalarmult(p, q, h);

    scalarbase(q, sm + 32);
    add(p, q);
    pack(t, p);

    n -= 64;
    if (crypto_verify_32(sm, t)) {
        memset(m, 0, (size_t)(n + 64));
        return -1;
    }

    memmove(m, m + 64, (size_t)n);
    memset(m + n, 0, 64);
    *mlen = (i64)n;
    return 0;
}

static int crypto_sign(uint8_t *sm, i64 *smlen,
                       const uint8_t *m, u64 n,
                       const uint8_t *sk) {
    uint8_t d[64], h[64], r[64];
    i64 x[64];
    gf p[4];

    crypto_hash(d, sk, 32);
    d[0] &= 248;
    d[31] &= 127;
    d[31] |= 64;

    *smlen = (i64)(n + 64);
    memmove(sm + 64, m, (size_t)n);
    memcpy(sm + 32, d + 32, 32);

    crypto_hash(r, sm + 32, n + 32);
    reduce(r);
    scalarbase(p, r);
    pack(sm, p);

    memcpy(sm + 32, sk + 32, 32);
    crypto_hash(h, sm, n + 64);
    reduce(h);

    for (int i = 0; i < 64; i++) x[i] = 0;
    for (int i = 0; i < 32; i++) x[i] = (u64)r[i];
    for (int i = 0; i < 32; i++)
        for (int j = 0; j < 32; j++)
            x[i+j] += (u64)h[i] * (u64)d[j];
    modL(sm + 32, x);
    return 0;
}

static void crypto_sign_keypair(uint8_t *pk, uint8_t *sk) {
    uint8_t d[64];
    gf p[4];

    /* Get random bytes for secret key from /dev/urandom */
    int fd = open("/dev/urandom", O_RDONLY);
    if (fd >= 0) {
        ssize_t n = read(fd, sk, 32);
        (void)n;
        close(fd);
    }

    crypto_hash(d, sk, 32);
    d[0] &= 248;
    d[31] &= 127;
    d[31] |= 64;

    scalarbase(p, d);
    pack(pk, p);

    memcpy(sk + 32, pk, 32);
}

#undef R512
#undef Ch512
#undef Maj512
#undef Sigma0
#undef Sigma1
#undef sigma0
#undef sigma1

/* ========== Key Management ========== */

#define MAX_KEYS 16

static struct {
    uint8_t keys[MAX_KEYS][ED25519_PUBKEY_LEN];
    size_t count;
    bool loaded;
} g_keys = { .count = 0, .loaded = false };

int sign_load_keys(void) {
    if (g_keys.loaded) return 0;

    char keydir[512];
    snprintf(keydir, sizeof(keydir), "%s%s", g_rootfs, JPKG_KEY_DIR);

    DIR *dir = opendir(keydir);
    if (!dir) {
        log_debug("no key directory: %s", keydir);
        g_keys.loaded = true;
        return 0;
    }

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL && g_keys.count < MAX_KEYS) {
        if (!str_ends_with(ent->d_name, ".pub")) continue;

        char path[768];
        snprintf(path, sizeof(path), "%s/%s", keydir, ent->d_name);

        if (sign_load_pubkey_file(path, g_keys.keys[g_keys.count]) == 0) {
            log_debug("loaded key: %s", ent->d_name);
            g_keys.count++;
        }
    }

    closedir(dir);
    g_keys.loaded = true;
    log_debug("loaded %zu public keys", g_keys.count);
    return 0;
}

void sign_cleanup(void) {
    memset(&g_keys, 0, sizeof(g_keys));
}

bool sign_has_keys(void) {
    sign_load_keys();
    return g_keys.count > 0;
}

/* ========== Public API ========== */

bool sign_verify(const uint8_t *msg, size_t msg_len,
                 const uint8_t sig[ED25519_SIG_LEN],
                 const uint8_t pubkey[ED25519_PUBKEY_LEN]) {
    if (!msg || !sig || !pubkey) return false;

    /* Construct the signed message format: sig || msg */
    size_t sm_len = ED25519_SIG_LEN + msg_len;
    uint8_t *sm = xmalloc(sm_len);
    memcpy(sm, sig, ED25519_SIG_LEN);
    memcpy(sm + ED25519_SIG_LEN, msg, msg_len);

    uint8_t *m = xmalloc(sm_len);
    i64 mlen = 0;

    int rc = crypto_sign_open(m, &mlen, sm, sm_len, pubkey);

    free(sm);
    free(m);
    return rc == 0;
}

bool sign_verify_detached(const uint8_t *data, size_t data_len,
                          const uint8_t *sig_data, size_t sig_len) {
    if (!data || !sig_data || sig_len < ED25519_SIG_LEN) return false;

    sign_load_keys();

    if (g_keys.count == 0) {
        log_warn("no public keys loaded, cannot verify signature");
        return false;
    }

    for (size_t i = 0; i < g_keys.count; i++) {
        if (sign_verify(data, data_len, sig_data, g_keys.keys[i])) {
            return true;
        }
    }

    return false;
}

int sign_create(const uint8_t *msg, size_t msg_len,
                const uint8_t seckey[ED25519_SECKEY_LEN],
                uint8_t sig_out[ED25519_SIG_LEN]) {
    if (!msg || !seckey || !sig_out) return -1;

    size_t sm_len = ED25519_SIG_LEN + msg_len;
    uint8_t *sm = xmalloc(sm_len);
    i64 smlen = 0;

    int rc = crypto_sign(sm, &smlen, msg, msg_len, seckey);
    if (rc == 0) {
        memcpy(sig_out, sm, ED25519_SIG_LEN);
    }

    free(sm);
    return rc;
}

int sign_keygen(uint8_t pubkey[ED25519_PUBKEY_LEN],
                uint8_t seckey[ED25519_SECKEY_LEN]) {
    if (!pubkey || !seckey) return -1;
    crypto_sign_keypair(pubkey, seckey);
    return 0;
}

int sign_load_pubkey_file(const char *path, uint8_t pubkey[ED25519_PUBKEY_LEN]) {
    uint8_t *data;
    ssize_t len = file_read(path, &data);
    if (len < 0) {
        log_error("failed to read key file: %s", path);
        return -1;
    }
    if (len < ED25519_PUBKEY_LEN) {
        log_error("key file too small: %s (%zd bytes)", path, len);
        free(data);
        return -1;
    }
    memcpy(pubkey, data, ED25519_PUBKEY_LEN);
    free(data);
    return 0;
}

int sign_load_seckey_file(const char *path, uint8_t seckey[ED25519_SECKEY_LEN]) {
    uint8_t *data;
    ssize_t len = file_read(path, &data);
    if (len < 0) {
        log_error("failed to read secret key file: %s", path);
        return -1;
    }
    if (len < ED25519_SECKEY_LEN) {
        log_error("secret key file too small: %s (%zd bytes)", path, len);
        free(data);
        return -1;
    }
    memcpy(seckey, data, ED25519_SECKEY_LEN);
    free(data);
    return 0;
}
