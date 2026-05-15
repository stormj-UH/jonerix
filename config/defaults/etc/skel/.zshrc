# ~/.zshrc — per-user interactive zsh config.
#
# System-wide defaults are in /etc/zshrc.  This file is sourced
# AFTER that for interactive shells; override or add to taste.

# ── History ─────────────────────────────────────────────────────────────
HISTFILE=$HOME/.zsh_history
HISTSIZE=10000
SAVEHIST=10000
setopt APPEND_HISTORY HIST_IGNORE_DUPS HIST_IGNORE_SPACE INTERACTIVE_COMMENTS SHARE_HISTORY

# ── Editing ─────────────────────────────────────────────────────────────
bindkey -e        # emacs keybindings; -v for vi
bindkey '^[[A' history-search-backward
bindkey '^[[B' history-search-forward

# ── Completion ─────────────────────────────────────────────────────────
autoload -Uz compinit
compinit -D -i    # -D skips dumpfile (musl ppoll wedge); -i ignores insecure files

# Case-insensitive + partial-word completion.
zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}' 'r:|=*' 'l:|=* r:|=*'

# ── Aliases ─────────────────────────────────────────────────────────────
alias ls='ls --color=auto'
alias ll='ls -lh'
alias la='ls -lhA'
alias grep='grep --color=auto'
alias ..='cd ..'
alias ...='cd ../..'
