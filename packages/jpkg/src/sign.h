/*
 * jpkg - jonerix package manager
 * sign.h - Ed25519 signature verification
 *
 * MIT License
 * Copyright (c) 2026 jonerix contributors
 */

#ifndef JPKG_SIGN_H
#define JPKG_SIGN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define ED25519_PUBKEY_LEN  32
#define ED25519_SECKEY_LEN  64
#define ED25519_SIG_LEN     64

/* Load the distribution's public key(s) from /etc/jpkg/keys/ */
int sign_load_keys(void);

/* Verify an Ed25519 signature.
 * Returns true if valid. */
bool sign_verify(const uint8_t *msg, size_t msg_len,
                 const uint8_t sig[ED25519_SIG_LEN],
                 const uint8_t pubkey[ED25519_PUBKEY_LEN]);

/* Verify a detached signature file against data.
 * Tries all loaded public keys. Returns true if any key validates. */
bool sign_verify_detached(const uint8_t *data, size_t data_len,
                          const uint8_t *sig_data, size_t sig_len);

/* Sign data with a secret key (for build tooling) */
int sign_create(const uint8_t *msg, size_t msg_len,
                const uint8_t seckey[ED25519_SECKEY_LEN],
                uint8_t sig_out[ED25519_SIG_LEN]);

/* Generate an Ed25519 keypair */
int sign_keygen(uint8_t pubkey[ED25519_PUBKEY_LEN],
                uint8_t seckey[ED25519_SECKEY_LEN]);

/* Load a key from file (raw 32/64 byte binary) */
int sign_load_pubkey_file(const char *path, uint8_t pubkey[ED25519_PUBKEY_LEN]);
int sign_load_seckey_file(const char *path, uint8_t seckey[ED25519_SECKEY_LEN]);

/* Cleanup loaded keys */
void sign_cleanup(void);

#endif /* JPKG_SIGN_H */
