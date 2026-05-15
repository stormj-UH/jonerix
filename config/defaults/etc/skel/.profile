# ~/.profile — POSIX-safe per-user login shell init.
#
# Sourced by /bin/sh (mksh) and any other POSIX-only shell at login.
# brash and zsh use their own per-user files (~/.brash_profile,
# ~/.zprofile) which fall back to this if they delegate.

# ── User bin on PATH ────────────────────────────────────────────────────
# ~/bin is the canonical jonerix location for user-installed binaries.
# /usr/local/bin doesn't exist on this distro (merged-usr-flat — see
# /etc/profile comments).
if [ -d "$HOME/bin" ]; then
    case ":$PATH:" in
        *":$HOME/bin:"*) ;;
        *) PATH="$HOME/bin:$PATH" ;;
    esac
    export PATH
fi

# ── History ─────────────────────────────────────────────────────────────
HISTFILE="$HOME/.sh_history"
HISTSIZE=1000
export HISTFILE HISTSIZE
