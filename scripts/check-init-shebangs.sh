#!/bin/sh
# Fail when an OpenRC service script that the repo ships or generates
# does not start with exactly `#!/bin/openrc-run`.
#
# Why exactly that line:
#   - OpenRC's gendepends.sh only adds a script to the deptree when its
#     first line ends in /openrc-run (or /runscript). Any other shebang
#     (openrc 0.54-r5..r7 shipped /etc/init.d/sysctl with a mksh one)
#     leaves the service out of every runlevel without a message.
#   - jonerix folds sbin into /bin and has no /sbin/openrc-run, so a
#     /sbin shebang passes gendepends but the exec fails with ENOENT.
#
# What is checked (under packages/, config/, image/, install/, scripts/
# and the top-level Dockerfiles):
#   1. every file in an init.d/ directory, and every *.initd file: the
#      first line (symlinks, dotfiles and *.sh helpers are skipped);
#   2. every shell heredoc that writes into an init.d/ path, e.g.
#      `cat > "$DESTDIR/etc/init.d/foo" <<'EOF'`: the heredoc's first
#      line;
#   3. every `#!.../openrc-run` or `#!.../runscript` anywhere in those
#      files (Python writers, recipes, scripts): it must be the exact
#      string above.
#
# Usage: sh scripts/check-init-shebangs.sh [REPO_ROOT]
# Exit status: 0 if clean, 1 if any bad shebang was found.
#
# SPDX-License-Identifier: MIT

set -eu

ROOT=${1:-.}
WANT='#!/bin/openrc-run'
SELF=scripts/check-init-shebangs.sh

cd "$ROOT"

tmp=${TMPDIR:-/tmp}/check-init-shebangs.$$
trap 'rm -f "$tmp"' EXIT INT TERM

# Candidate files: anything mentioning init.d/, openrc-run or runscript.
# Source code and patches against upstream trees (.c/.h/.rs/.go, .patch,
# .diff) and documentation are left out: they quote shebangs rather than
# ship them (upstream .in templates use `#!@SBINDIR@/openrc-run`).
{
    for top in packages config image install scripts; do
        [ -d "$top" ] || continue
        find "$top" -type f \
            ! -name '.*' \
            ! -name '*.c' ! -name '*.h' ! -name '*.rs' ! -name '*.go' \
            ! -name '*.patch' ! -name '*.diff' \
            ! -name '*.md' ! -name '*.txt' ! -name '*.json' ! -name '*.lock' \
            -exec grep -l -e 'init\.d/' -e 'openrc-run' -e 'runscript' {} + || true
    done
    for f in Dockerfile*; do
        [ -f "$f" ] || continue
        grep -l -e 'init\.d/' -e 'openrc-run' -e 'runscript' "$f" || true
    done
    # Files in init.d/ directories are checked even when they mention
    # none of the above (e.g. a script with a plain sh shebang).
    for top in packages config image install; do
        [ -d "$top" ] || continue
        find "$top" -type f -path '*/init.d/*' ! -name '.*' ! -name '*.sh' || true
        find "$top" -type f -name '*.initd' || true
    done
} | sort -u > "$tmp"

status=0

report() {
    printf '%s:%s: %s\n' "$1" "$2" "$3"
    status=1
}

# Plain POSIX sh on purpose: the jonerix builder image ships no awk.
check_file() {
    f=$1 initd=$2 n=0 expect=0
    while IFS= read -r line || [ -n "$line" ]; do
        n=$((n + 1))
        if [ "$n" -eq 1 ] && [ "$initd" -eq 1 ] && [ "$line" != "$WANT" ]; then
            report "$f" "$n" "init script starts with \"$line\", want \"$WANT\""
        fi
        if [ "$expect" -ne 0 ]; then
            body=$line
            # <<- strips leading tabs from the heredoc body.
            [ "$expect" -eq 2 ] && body=${body#"${body%%[!	]*}"}
            if [ "$body" != "$WANT" ]; then
                report "$f" "$n" "heredoc into init.d starts with \"$body\", want \"$WANT\""
            fi
            expect=0
        fi
        # A heredoc whose target lies in an init.d/ directory:
        #   cat > .../init.d/NAME <<EOF      cat <<EOF > .../init.d/NAME
        #   tee .../init.d/NAME <<EOF        install ... /dev/stdin .../init.d/NAME <<EOF
        # (Cheap single-star tests first: multi-star patterns are slow
        # on long lines in some shells.)
        case "$line" in
            *'<<'*)
                case "$line" in
                    *init.d/*)
                        case "$line" in
                            *'>'*init.d/*'<<'*|*'<<'*'>'*init.d/*|*tee*init.d/*'<<'*|*/dev/stdin*init.d/*'<<'*)
                                case "$line" in
                                    *'<<-'*) expect=2 ;;
                                    *) expect=1 ;;
                                esac
                                ;;
                        esac
                        ;;
                esac
                ;;
        esac
        # Any shebang naming openrc-run or runscript must be exactly $WANT.
        case "$line" in
            *'#!'*) ;;
            *) continue ;;
        esac
        rest=$line
        while :; do
            case "$rest" in
                *'#!'*) ;;
                *) break ;;
            esac
            rest=${rest#*'#!'}
            tok=${rest#"${rest%%[! 	]*}"}
            pre=${rest%"$tok"}
            tok=${tok%%[ 	\"\'\`\;]*}
            case "$tok" in
                */openrc-run|*/runscript)
                    if [ "#!$pre$tok" != "$WANT" ]; then
                        report "$f" "$n" "shebang \"#!$pre$tok\", want \"$WANT\""
                    fi
                    ;;
            esac
        done
    done < "$f"
}

while IFS= read -r f; do
    [ "$f" = "$SELF" ] && continue
    [ -L "$f" ] && continue
    case "$f" in
        */init.d/*.sh) initd=0 ;;
        */init.d/*|*.initd) initd=1 ;;
        *) initd=0 ;;
    esac
    check_file "$f" "$initd"
done < "$tmp"

if [ "$status" -ne 0 ]; then
    printf 'check-init-shebangs: OpenRC services must start with exactly %s\n' "$WANT" >&2
else
    printf 'check-init-shebangs: OK\n'
fi
exit "$status"
