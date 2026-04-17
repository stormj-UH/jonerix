/*
 * /bin/install GNU-compat shim for jonerix builder use.
 *
 * Toybox's install applet supports -m, -d, -o, -g, -D, -p, but NOT -c
 * (GNU's "copy" flag, which is the default behavior of toybox install
 * anyway — just silently ignored on GNU). Autoconf-generated Makefiles
 * emit `install -c -m 644 src dst`, and toybox install parses `-c` as
 * the destination and then complains "Needs 1 argument" when src/dst
 * are not what it expects. Reproduced on Python 3.14.3 build
 * 2026-04-17.
 *
 * This shim strips `-c` from argv and exec's the toybox install applet
 * with the remaining flags. No other behavior change.
 *
 * Installed at /bin/install (replacing the toybox applet symlink) by
 * ci-build-{aarch64,x86_64}.sh so every recipe sees it.
 */

#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    char **new_argv = calloc((size_t)argc + 2, sizeof(char *));
    if (!new_argv) { perror("calloc"); return 1; }
    int ni = 0;
    new_argv[ni++] = (char *)"/bin/toybox";
    new_argv[ni++] = (char *)"install";
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-c") == 0) continue;
        new_argv[ni++] = argv[i];
    }
    new_argv[ni] = NULL;
    execv("/bin/toybox", new_argv);
    /* execv failed */
    perror("execv /bin/toybox install");
    return 127;
}
