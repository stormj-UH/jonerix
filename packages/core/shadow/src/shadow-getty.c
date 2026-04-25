/*
 * shadow-getty — minimal getty-to-login bridge for the jonerix console.
 *
 * Invoked by the shadow-login OpenRC service (supervise-daemon handles
 * respawn). For each invocation:
 *   1. Become a session leader (setsid) so TIOCSCTTY can succeed.
 *   2. Open /dev/ttyN read-write.
 *   3. Claim it as our controlling terminal.
 *   4. Wire fd 0/1/2 to it.
 *   5. Reset termios to sane Linux-VC defaults (ECHO on, erase=DEL,
 *      ICANON+ISIG, no fancy line noise).
 *   6. Set TERM=linux.
 *   7. execv /bin/login -p.
 *
 * Exists because the previous mksh wrapper (+ toybox stty + `setsid -c`)
 * had too many failure modes: stty rejected one of its knobs and the
 * wrapper exited, supervise-daemon respawned once per second, login
 * saw EOF on a pipe-stdin and died immediately, the user's HDMI
 * console became an unusable reprint loop. A 100-line C program
 * avoids the shell entirely — see jonerix-tormenta debugging, 2026-04-24.
 *
 * Usage:  shadow-getty [tty-path]        (default: /dev/tty1)
 *
 * Part of shadow (BSD-3-Clause jonerix core jpkg). SPDX-License-Identifier: BSD-3-Clause
 */

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

static void setup_termios(int fd) {
    struct termios t;
    if (tcgetattr(fd, &t) != 0) {
        /* Non-fatal: if the kernel can't hand us current attrs, we
         * plow on. login(1) will work; the user's backspace may not. */
        return;
    }

    /* Input: CR→LF, honour XON/XOFF, strip high bit off only if the
     * terminal asks. BRKINT lets ^C trigger a break interrupt. */
    t.c_iflag = BRKINT | ICRNL | IXON | IUTF8;

    /* Output: post-process, NL→CRLF. The Linux VC needs ONLCR. */
    t.c_oflag = OPOST | ONLCR;

    /* Control: keep whatever baud the VC set (CBAUD mask), 8 bits, no
     * parity, 1 stop bit, enable receiver, hang up on last close. */
    t.c_cflag = (t.c_cflag & CBAUD) | CS8 | CREAD | HUPCL;

    /* Local: canonical line editing, echo on, signal chars, extended
     * processing. IEXTEN is required for ECHOCTL to work. */
    t.c_lflag = ISIG | ICANON | ECHO | ECHOE | ECHOK
              | ECHOCTL | ECHOKE | IEXTEN;

    /* Control characters. VERASE = 0x7F (DEL) because the Linux kernel
     * virtual console sends DEL on Backspace and zsh's default emacs
     * keymap binds DEL to backward-delete-char. If we left this at
     * ^H, the user's Backspace would insert a literal tilde. */
    memset(t.c_cc, 0, sizeof(t.c_cc));
    t.c_cc[VINTR]  = 0x03; /* ^C */
    t.c_cc[VQUIT]  = 0x1c; /* ^\ */
    t.c_cc[VERASE] = 0x7f; /* ^? (DEL) — Backspace */
    t.c_cc[VKILL]  = 0x15; /* ^U */
    t.c_cc[VEOF]   = 0x04; /* ^D */
    t.c_cc[VSTART] = 0x11; /* ^Q */
    t.c_cc[VSTOP]  = 0x13; /* ^S */
    t.c_cc[VSUSP]  = 0x1a; /* ^Z */
    t.c_cc[VMIN]   = 1;
    t.c_cc[VTIME]  = 0;

    /* Speed: 38400 is the Linux-VC convention. The kernel ignores it,
     * but programs that inspect termios (e.g. login) expect a sane
     * value. */
    cfsetispeed(&t, B38400);
    cfsetospeed(&t, B38400);

    /* TCSAFLUSH: drain output, discard input, then apply. Avoids any
     * residual garbage from whatever used the tty before us. */
    tcsetattr(fd, TCSAFLUSH, &t);

    tcflush(fd, TCIOFLUSH);
}

int main(int argc, char **argv) {
    const char *tty = (argc > 1) ? argv[1] : "/dev/tty1";

    /* Detach from any controlling tty the supervisor inherited (stdin
     * may be a pipe from supervise-daemon). Becoming our own session
     * leader is the precondition for TIOCSCTTY below. */
    if (setsid() < 0 && errno != EPERM) {
        /* EPERM is fine: we're already a pgrp leader (harmless). */
        perror("setsid");
    }

    /* Open the target tty. O_NOCTTY here — we'll claim it explicitly
     * with TIOCSCTTY so we know it stuck. */
    int fd = open(tty, O_RDWR | O_NOCTTY);
    if (fd < 0) {
        fprintf(stderr, "shadow-getty: open %s: %s\n", tty, strerror(errno));
        return 1;
    }

    /* Claim it as our controlling tty. The `1` forces the claim even
     * if another session already has it. Normally it shouldn't, but
     * during a crash-restart the previous getty's session might still
     * hold it. */
    if (ioctl(fd, TIOCSCTTY, 1) < 0) {
        fprintf(stderr, "shadow-getty: TIOCSCTTY %s: %s (continuing)\n",
                tty, strerror(errno));
        /* Non-fatal: login will still read and write on the fd; the
         * ctty bit mostly matters for job control. */
    }

    /* Wire fd 0/1/2 to the tty. We do this AFTER TIOCSCTTY so login's
     * attempts to manipulate its ctty hit the right fd. */
    if (dup2(fd, 0) < 0 || dup2(fd, 1) < 0 || dup2(fd, 2) < 0) {
        perror("shadow-getty: dup2");
        return 1;
    }
    if (fd > 2) close(fd);

    setup_termios(0);

    /* TERM for the Linux VC. login -p (below) preserves this into the
     * user's shell; without it the shell comes up at TERM=dumb. */
    setenv("TERM", "linux", 1);

    /* Hand off to login. -p: preserve the environment we just set. */
    execl("/bin/login", "login", "-p", (char *)NULL);

    /* execl only returns on failure. */
    perror("shadow-getty: exec /bin/login");
    return 127;
}
