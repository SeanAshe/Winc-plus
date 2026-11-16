# zsh 的启动环境：记录启动目录，恢复用户的 `ZDOTDIR` 并读取用户的 `.zshenv`。
# 交互式 shell 从 `winc.zsh` 安装命令块钩子，后续配置仍按用户的 `ZDOTDIR` 读取。
# winc 通过 `ZDOTDIR` 指向本文件所在目录，启动前按需生成。

builtin typeset -g _winc_dir=${${(%):-%x}:A:h}
# 记录启动目录，用户配置中的 `cd` 不改变项目环境的选择。
builtin typeset -g _winc_python_root=$PWD
# `WINC_PYTHON=0` 时不加载启动目录中的 Python 环境：用户没有允许这个目录。
builtin typeset -g _winc_python=${WINC_PYTHON-1}
builtin unset WINC_PYTHON
# `WINC_PROMPT=0` 时保留默认提示符，由 `terminal.prompt` 控制。
builtin typeset -g _winc_prompt=${WINC_PROMPT-1}
builtin unset WINC_PROMPT
# 命令行报告附带的随机数，保存在未导出的变量中，供 OSC 133 验证。
builtin typeset -g _winc_nonce=${WINC_NONCE-}
builtin unset WINC_NONCE
if [[ -n ${WINC_ZDOTDIR+x} ]]; then
  builtin export ZDOTDIR=$WINC_ZDOTDIR
  builtin unset WINC_ZDOTDIR
else
  builtin unset ZDOTDIR
fi

{
  [[ -f ${ZDOTDIR:-$HOME}/.zshenv ]] && builtin source -- "${ZDOTDIR:-$HOME}/.zshenv"
} always {
  [[ -o interactive ]] && builtin source -- "$_winc_dir/winc.zsh"
  builtin unset _winc_dir
}
