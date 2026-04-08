/*
 * jpkg - jonerix package manager
 * main.c - Entry point, command dispatch
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

#include "util.h"
#include "fetch.h"
#include "sign.h"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>

#define JPKG_VERSION "1.0.0"

/* Command handlers (defined in cmd_*.c) */
extern int cmd_update(int argc, char **argv);
extern int cmd_install(int argc, char **argv);
extern int cmd_remove(int argc, char **argv);
extern int cmd_upgrade(int argc, char **argv);
extern int cmd_search(int argc, char **argv);
extern int cmd_info(int argc, char **argv);
extern int cmd_build(int argc, char **argv);
extern int cmd_build_world(int argc, char **argv);
extern int cmd_verify(int argc, char **argv);
extern int cmd_license_audit(int argc, char **argv);
extern int cmd_keygen(int argc, char **argv);
extern int cmd_sign(int argc, char **argv);

/* ========== Command Table ========== */

typedef struct {
    const char *name;
    const char *alias;
    const char *description;
    int (*handler)(int argc, char **argv);
    bool needs_root;
} command_t;

static const command_t commands[] = {
    { "update",        NULL,  "Fetch package index from mirrors",        cmd_update,        true  },
    { "install",       "add", "Install package(s) and dependencies",     cmd_install,       true  },
    { "remove",        "del", "Remove installed package(s)",             cmd_remove,        true  },
    { "upgrade",       NULL,  "Upgrade all installed packages",          cmd_upgrade,       true  },
    { "search",        NULL,  "Search package names and descriptions",   cmd_search,        false },
    { "info",          NULL,  "Show package metadata",                   cmd_info,          false },
    { "build",         NULL,  "Build package from source recipe",        cmd_build,         false },
    { "build-world",   NULL,  "Rebuild entire system from source",       cmd_build_world,   false },
    { "verify",        NULL,  "Verify installed files against manifests", cmd_verify,       false },
    { "license-audit", NULL,  "Audit licenses of installed packages",    cmd_license_audit, false },
    { "keygen",        NULL,  "Generate Ed25519 signing keypair",        cmd_keygen,        false },
    { "sign",          NULL,  "Sign a file with an Ed25519 secret key",  cmd_sign,          false },
    { NULL, NULL, NULL, NULL, false }
};

/* ========== Usage ========== */

static void print_version(void) {
    printf("jpkg %s - jonerix package manager\n", JPKG_VERSION);
    printf("MIT License - Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software\n");
}

static void print_usage(void) {
    print_version();
    printf("\nUsage: jpkg [options] <command> [arguments]\n\n");
    printf("Commands:\n");
    for (int i = 0; commands[i].name; i++) {
        if (commands[i].alias) {
            printf("  %-16s (%-3s) %s\n",
                   commands[i].name, commands[i].alias, commands[i].description);
        } else {
            printf("  %-22s %s\n", commands[i].name, commands[i].description);
        }
    }
    printf("\nOptions:\n");
    printf("  -v, --verbose        Increase verbosity\n");
    printf("  -q, --quiet          Suppress non-error output\n");
    printf("  -r, --root <path>    Use alternative root filesystem\n");
    printf("  -V, --version        Show version\n");
    printf("  -h, --help           Show this help\n");
}

/* ========== Main ========== */

int main(int argc, char **argv) {
    if (argc < 2) {
        print_usage();
        return 1;
    }

    /* Parse global options before the command */
    int cmd_idx = 1;
    while (cmd_idx < argc && argv[cmd_idx][0] == '-') {
        if (strcmp(argv[cmd_idx], "-v") == 0 || strcmp(argv[cmd_idx], "--verbose") == 0) {
            log_set_level(LOG_DEBUG);
            cmd_idx++;
        } else if (strcmp(argv[cmd_idx], "-q") == 0 || strcmp(argv[cmd_idx], "--quiet") == 0) {
            log_set_level(LOG_ERROR);
            cmd_idx++;
        } else if (strcmp(argv[cmd_idx], "-r") == 0 || strcmp(argv[cmd_idx], "--root") == 0) {
            if (cmd_idx + 1 >= argc) {
                fprintf(stderr, "jpkg: --root requires an argument\n");
                return 1;
            }
            set_rootfs(argv[cmd_idx + 1]);
            cmd_idx += 2;
        } else if (strcmp(argv[cmd_idx], "-V") == 0 || strcmp(argv[cmd_idx], "--version") == 0) {
            print_version();
            return 0;
        } else if (strcmp(argv[cmd_idx], "-h") == 0 || strcmp(argv[cmd_idx], "--help") == 0) {
            print_usage();
            return 0;
        } else {
            /* Unknown option - might be a command */
            break;
        }
    }

    if (cmd_idx >= argc) {
        print_usage();
        return 1;
    }

    const char *cmd_name = argv[cmd_idx];

    /* Find command */
    const command_t *cmd = NULL;
    for (int i = 0; commands[i].name; i++) {
        if (strcmp(cmd_name, commands[i].name) == 0 ||
            (commands[i].alias && strcmp(cmd_name, commands[i].alias) == 0)) {
            cmd = &commands[i];
            break;
        }
    }

    if (!cmd) {
        /* Try external subcommand: jpkg-<cmd> */
        char subcmd[256];
        int n = snprintf(subcmd, sizeof(subcmd), "jpkg-%s", cmd_name);
        if (n > 0 && (size_t)n < sizeof(subcmd)) {
            /* Check if the binary exists somewhere in PATH */
            static const char *path_dirs[] = {
                "/bin", "/usr/bin", "/usr/local/bin", "/sbin", "/usr/sbin", NULL
            };
            bool found = false;
            for (int i = 0; path_dirs[i]; i++) {
                char full[512];
                snprintf(full, sizeof(full), "%s/%s", path_dirs[i], subcmd);
                if (access(full, X_OK) == 0) { found = true; break; }
            }
            if (found) {
                /* Shift argv so that subcmd gets: argv[0]=subcmd, rest=original args after cmd */
                argv[cmd_idx] = subcmd;
                execvp(subcmd, argv + cmd_idx);
                /* execvp only returns on failure */
                fprintf(stderr, "jpkg: failed to run %s: %s\n", subcmd, strerror(errno));
                return 1;
            }
        }
        fprintf(stderr, "jpkg: unknown command: %s\n", cmd_name);
        fprintf(stderr, "Run 'jpkg --help' for usage.\n");
        return 1;
    }

    /* Check for root privileges if needed */
    if (cmd->needs_root && geteuid() != 0 && g_rootfs[0] == '\0') {
        log_warn("this operation typically requires root privileges");
        /* Continue anyway - let it fail naturally if permissions are denied */
    }

    /* Initialize TLS for HTTPS fetches */
    if (fetch_init() != 0) {
        fprintf(stderr, "jpkg: TLS initialization failed\n");
        return 1;
    }

    /* Dispatch to command handler */
    int sub_argc = argc - cmd_idx - 1;
    char **sub_argv = argv + cmd_idx + 1;

    int rc = cmd->handler(sub_argc, sub_argv);

    /* Cleanup */
    fetch_cleanup();
    sign_cleanup();

    return rc;
}
