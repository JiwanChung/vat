# ===========================================
# Developer Shell Configuration (.bashrc)
# Optimized for: Rust/Python/Node.js development
# ===========================================

# --- Shell Options ---
set -o vi                           # Vi mode editing
shopt -s histappend                 # Append to history, don't overwrite
shopt -s cmdhist                    # Save multiline commands as one entry
shopt -s checkwinsize               # Update LINES/COLUMNS after each command
shopt -s cdspell                    # Autocorrect minor cd typos
shopt -s dirspell                   # Autocorrect directory name typos
shopt -s globstar                   # Enable ** recursive glob

# --- History ---
export HISTSIZE=50000
export HISTFILESIZE=100000
export HISTCONTROL=ignoreboth:erasedups
export HISTTIMEFORMAT="%Y-%m-%d %H:%M:%S  "
export HISTIGNORE="ls:cd:pwd:exit:clear:history"

# --- Path ---
export PATH="$HOME/.cargo/bin:$PATH"
export PATH="$HOME/.local/bin:$PATH"
export PATH="$HOME/.pyenv/bin:$PATH"
export PATH="$HOME/.nvm/versions/node/v20/bin:$PATH"
export PATH="$HOME/go/bin:$PATH"
export PATH="/opt/homebrew/bin:$PATH"
export PATH="$HOME/.krew/bin:$PATH"

# --- Default programs ---
export EDITOR="nvim"
export VISUAL="nvim"
export PAGER="less -R"
export BROWSER="open"
export MANPAGER="sh -c 'col -bx | bat -l man -p'"
export LESS="-iMFXR --mouse"

# --- Locale ---
export LANG="en_US.UTF-8"
export LC_ALL="en_US.UTF-8"

# --- Development ---
export RUST_BACKTRACE=1
export RUST_LOG="info"
export CARGO_INCREMENTAL=1
export SCCACHE_DIR="$HOME/.cache/sccache"
export RUSTC_WRAPPER="sccache"

export PYTHONDONTWRITEBYTECODE=1
export PYTHONUNBUFFERED=1
export PIP_REQUIRE_VIRTUALENV=true
export VIRTUAL_ENV_DISABLE_PROMPT=1

export NODE_OPTIONS="--max-old-space-size=4096"
export NPM_CONFIG_SAVE_EXACT=true

export GOPATH="$HOME/go"
export GOBIN="$GOPATH/bin"
export CGO_ENABLED=0

# --- Cloud & K8s ---
export AWS_PROFILE="acme-dev"
export AWS_DEFAULT_REGION="us-east-1"
export AWS_PAGER=""
export KUBECONFIG="$HOME/.kube/config:$HOME/.kube/acme-staging:$HOME/.kube/acme-prod"
export KUBE_EDITOR="nvim"

# --- Docker ---
export DOCKER_BUILDKIT=1
export COMPOSE_DOCKER_CLI_BUILD=1
export DOCKER_DEFAULT_PLATFORM="linux/amd64"

# --- FZF ---
export FZF_DEFAULT_COMMAND="fd --type f --hidden --follow --exclude .git"
export FZF_DEFAULT_OPTS="--height 40% --layout=reverse --border --preview 'bat --color=always {}'"
export FZF_CTRL_T_COMMAND="$FZF_DEFAULT_COMMAND"
export FZF_ALT_C_COMMAND="fd --type d --hidden --follow --exclude .git"

# --- Tokens & API keys (loaded from secure store) ---
export GITHUB_TOKEN=$(security find-generic-password -a "$USER" -s github-token -w 2>/dev/null)
export OPENAI_API_KEY=$(security find-generic-password -a "$USER" -s openai-key -w 2>/dev/null)

# --- Prompt (PS1) ---
export GIT_PS1_SHOWDIRTYSTATE=1
export GIT_PS1_SHOWSTASHSTATE=1
export GIT_PS1_SHOWUNTRACKEDFILES=1
export GIT_PS1_SHOWUPSTREAM="auto"

parse_git_branch() {
    git branch 2>/dev/null | sed -e '/^[^*]/d' -e 's/* \(.*\)/(\1)/'
}

parse_venv() {
    if [[ -n "$VIRTUAL_ENV" ]]; then
        echo "($(basename "$VIRTUAL_ENV")) "
    fi
}

export PS1='\[\033[0;36m\]$(parse_venv)\[\033[0;33m\]\u@\h\[\033[0m\]:\[\033[0;34m\]\w\[\033[0;32m\] $(parse_git_branch)\[\033[0m\]\n\$ '

# --- Aliases: Navigation ---
alias ..='cd ..'
alias ...='cd ../..'
alias ....='cd ../../..'
alias ll='ls -lahF --color=auto'
alias la='ls -A --color=auto'
alias lt='ls -lahFt --color=auto'
alias tree='tree -C --dirsfirst'

# --- Aliases: Git ---
alias g='git'
alias gs='git status -sb'
alias ga='git add'
alias gc='git commit'
alias gca='git commit --amend'
alias gco='git checkout'
alias gsw='git switch'
alias gb='git branch -vv'
alias gd='git diff'
alias gds='git diff --staged'
alias gl='git log --oneline --graph --decorate -20'
alias gla='git log --oneline --graph --decorate --all'
alias gp='git push'
alias gpf='git push --force-with-lease'
alias gpl='git pull --rebase'
alias gst='git stash'
alias gstp='git stash pop'
alias gcp='git cherry-pick'
alias grb='git rebase'
alias grbi='git rebase -i'

# --- Aliases: Rust ---
alias cr='cargo run'
alias cb='cargo build'
alias ct='cargo test'
alias cc='cargo check'
alias ccl='cargo clippy -- -D warnings'
alias cf='cargo fmt'
alias cw='cargo watch -x check -x test'
alias cdoc='cargo doc --open --no-deps'

# --- Aliases: Python ---
alias py='python3'
alias pip='python3 -m pip'
alias venv='python3 -m venv .venv && source .venv/bin/activate'
alias act='source .venv/bin/activate'
alias deact='deactivate'
alias pytest='python3 -m pytest -v'
alias ruff='python3 -m ruff check .'
alias mypy='python3 -m mypy .'

# --- Aliases: Docker ---
alias d='docker'
alias dc='docker compose'
alias dps='docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"'
alias dlog='docker logs -f'
alias dex='docker exec -it'
alias dprune='docker system prune -af --volumes'

# --- Aliases: Kubernetes ---
alias k='kubectl'
alias kx='kubectx'
alias kn='kubens'
alias kgp='kubectl get pods'
alias kgs='kubectl get svc'
alias kgd='kubectl get deploy'
alias kge='kubectl get events --sort-by=.lastTimestamp'
alias klog='kubectl logs -f'
alias kexec='kubectl exec -it'
alias kpf='kubectl port-forward'

# --- Aliases: Misc ---
alias cat='bat --paging=never'
alias grep='rg'
alias find='fd'
alias top='btop'
alias du='dust'
alias df='duf'
alias diff='delta'
alias curl='curlie'
alias http='xh'
alias dig='dog'
alias watch='viddy'
alias j='just'
alias tf='terraform'
alias tg='terragrunt'
alias lg='lazygit'
alias ld='lazydocker'

# --- Functions ---

# Create and enter directory
mkcd() { mkdir -p "$1" && cd "$1"; }

# Find process by name
psg() { ps aux | rg -i "$1" | rg -v "rg -i"; }

# Quick HTTP server
serve() { python3 -m http.server "${1:-8000}"; }

# Extract any archive
extract() {
    if [ -f "$1" ]; then
        case "$1" in
            *.tar.bz2) tar xjf "$1"    ;;
            *.tar.gz)  tar xzf "$1"    ;;
            *.tar.xz)  tar xJf "$1"    ;;
            *.tar.zst) tar --zstd -xf "$1" ;;
            *.bz2)     bunzip2 "$1"    ;;
            *.gz)      gunzip "$1"     ;;
            *.tar)     tar xf "$1"     ;;
            *.tbz2)    tar xjf "$1"    ;;
            *.tgz)     tar xzf "$1"    ;;
            *.zip)     unzip "$1"      ;;
            *.7z)      7z x "$1"       ;;
            *.rar)     unrar x "$1"    ;;
            *)         echo "Cannot extract '$1'" ;;
        esac
    else
        echo "'$1' is not a valid file"
    fi
}

# Git commit browser (requires fzf)
fshow() {
    git log --graph --color=always \
        --format="%C(auto)%h%d %s %C(black)%C(bold)%cr" "$@" |
    fzf --ansi --no-sort --reverse --tiebreak=index \
        --bind "ctrl-m:execute:(grep -o '[a-f0-9]\{7\}' | head -1 | xargs -I % sh -c 'git show --color=always % | less -R') <<< {}"
}

# Switch to project directory (requires fzf)
proj() {
    local dir
    dir=$(fd --type d --max-depth 2 . ~/projects | fzf --preview 'ls -la {}') && cd "$dir"
}

# --- Tool Initialization ---

# pyenv
if command -v pyenv &>/dev/null; then
    eval "$(pyenv init -)"
    eval "$(pyenv virtualenv-init -)"
fi

# nvm
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && source "$NVM_DIR/nvm.sh" --no-use
[ -s "$NVM_DIR/bash_completion" ] && source "$NVM_DIR/bash_completion"

# direnv
if command -v direnv &>/dev/null; then
    eval "$(direnv hook bash)"
fi

# starship prompt (overrides PS1 above)
if command -v starship &>/dev/null; then
    eval "$(starship init bash)"
fi

# fzf keybindings
[ -f ~/.fzf.bash ] && source ~/.fzf.bash

# zoxide (smarter cd)
if command -v zoxide &>/dev/null; then
    eval "$(zoxide init bash)"
fi

# completions
[ -f /opt/homebrew/etc/bash_completion ] && source /opt/homebrew/etc/bash_completion
complete -C /usr/local/bin/aws_completer aws
source <(kubectl completion bash 2>/dev/null)
source <(helm completion bash 2>/dev/null)
