/*
 * jpkg - jonerix package manager
 * cmd_sign.c - jpkg sign: sign a file with an Ed25519 secret key
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "sign.h"
#include "util.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

int cmd_sign(int argc, char **argv) {
    const char *file = NULL;
    const char *keyfile = NULL;

    for (int i = 0; i < argc; i++) {
        if ((strcmp(argv[i], "--key") == 0 || strcmp(argv[i], "-k") == 0) && i + 1 < argc) {
            keyfile = argv[++i];
        } else {
            file = argv[i];
        }
    }

    if (!file) {
        fprintf(stderr, "usage: jpkg sign <file> [--key <keyfile>]\n");
        fprintf(stderr, "  Key can also be provided via JPKG_SIGNING_KEY env (base64).\n");
        return 1;
    }

    uint8_t seckey[ED25519_SECKEY_LEN];

    if (keyfile) {
        if (sign_load_seckey_file(keyfile, seckey) != 0) {
            log_error("failed to load key: %s", keyfile);
            return 1;
        }
    } else {
        log_error("no signing key: provide --key <file>");
        return 1;
    }

    uint8_t *data = NULL;
    ssize_t len = file_read(file, &data);
    if (len <= 0 || !data) {
        log_error("failed to read: %s", file);
        free(data);
        return 1;
    }

    uint8_t sig[ED25519_SIG_LEN];
    if (sign_create(data, (size_t)len, seckey, sig) != 0) {
        log_error("signing failed");
        free(data);
        return 1;
    }
    free(data);

    char sig_path[512];
    snprintf(sig_path, sizeof(sig_path), "%s.sig", file);
    if (file_write(sig_path, sig, ED25519_SIG_LEN) != 0) {
        log_error("failed to write %s", sig_path);
        return 1;
    }

    log_info("signed %s → %s", file, sig_path);
    return 0;
}
