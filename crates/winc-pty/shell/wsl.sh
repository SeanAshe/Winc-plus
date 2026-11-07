# 在 WSL 发行版内启动用户的 shell 并注入集成脚本，`$1` 是脚本目录在发行版内的路径。
# 仅设置环境变量和参数，不写入文件，也不修改用户的配置。注入规则与 Unix 上相同。
dir=$1
shell=${SHELL:-$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)}
shell=${shell:-/bin/sh}
case ${shell##*/} in
zsh)
    if [ -n "${ZDOTDIR+x}" ]; then
        export WINC_ZDOTDIR="$ZDOTDIR"
    fi
    export ZDOTDIR="$dir/zsh"
    exec "$shell" -l
    ;;
bash)
    # 登录 shell 不读取 --rcfile，因此以非登录方式启动，由集成脚本读取登录 shell 的配置文件。
    export WINC_BASH_LOGIN=1
    exec "$shell" --rcfile "$dir/bash/winc.bash"
    ;;
fish)
    if [ -n "${XDG_DATA_DIRS+x}" ]; then
        export WINC_XDG_DATA_DIRS="$XDG_DATA_DIRS"
    fi
    export XDG_DATA_DIRS="$dir/fish:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    exec "$shell" -l
    ;;
*)
    unset WINC_NONCE
    exec "$shell" -l
    ;;
esac
