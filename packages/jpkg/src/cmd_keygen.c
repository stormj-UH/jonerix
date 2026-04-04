/*
 * jpkg - jonerix package manager
 * cmd_keygen.c - jpkg keygen: generate Ed25519 signing keypair
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "sign.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>

int cmd_keygen(int argc, char **argv) {
    const char *prefix = (argc > 0) ? argv[0] : "jpkg-sign";

    uint8_t pubkey[ED25519_PUBKEY_LEN];
    uint8_t seckey[ED25519_SECKEY_LEN];

    if (sign_keygen(pubkey, seckey) != 0) {
        log_error("key generation failed");
        return 1;
    }

    char pub_path[512], sec_path[512];
    snprintf(pub_path, sizeof(pub_path), "%s.pub", prefix);
    snprintf(sec_path, sizeof(sec_path), "%s.sec", prefix);

    if (file_write(pub_path, pubkey, ED25519_PUBKEY_LEN) != 0) {
        log_error("failed to write %s", pub_path);
        return 1;
    }
    if (file_write(sec_path, seckey, ED25519_SECKEY_LEN) != 0) {
        log_error("failed to write %s", sec_path);
        return 1;
    }
    chmod(sec_path, 0600);

    log_info("public key : %s  (%d bytes)", pub_path, ED25519_PUBKEY_LEN);
    log_info("secret key : %s  (%d bytes, keep private!)", sec_path, ED25519_SECKEY_LEN);
    printf("\n# Copy the secret key as a GitHub Actions secret (JPKG_SIGNING_KEY):\n");
    printf("base64 %s\n", sec_path);
    printf("\n# Install the public key into an image or rootfs:\n");
    printf("install -Dm644 %s /etc/jpkg/keys/jonerix.pub\n", pub_path);

    return 0;
}
