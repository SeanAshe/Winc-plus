# zsh 的 shell 集成：通过 OSC 133 和 OSC 7 报告命令块与当前目录。
# 首次提示符前按设置处理默认提示符，加载启动目录中的 Python 环境并按需添加名称前缀。
# winc 从 `.zshenv` 读取本文件，启动前按需生成。

[[ -n $_winc_hooked ]] && builtin return
builtin typeset -g _winc_hooked=1 _winc_running= _winc_home=${${(%):-%x}:A:h}

# 百分号转义，按字节：命令行和目录里可能有分号、控制字符、非 ASCII。结果放在 REPLY 里。
_winc_urlencode() {
  builtin emulate -L zsh
  builtin setopt extendedglob
  builtin local LC_ALL=C
  REPLY=${1//(#m)[^A-Za-z0-9\/._~-]/%${(l:2::0:)$(( [##16] #MATCH ))}}
}

_winc_precmd() {
  builtin local ret=$?
  if [[ -n $_winc_running ]]; then
    # 结束标记附带随机数：命令的输出中伪造的结束标记不会被采用。
    builtin print -n -- "\e]133;D;$ret;nonce=$_winc_nonce\a"
    _winc_running=
  fi
  _winc_urlencode "$PWD"
  builtin print -n -- "\e]7;file://$HOST$REPLY\a\e]133;A;redraw=1;signed=1\a"
  # 提示符在哪里结束（用户从哪里开始输入）：PS1 末尾加一个不占宽度的标记。提示符主题每次重设
  # PS1 时再加上。
  [[ $PS1 == *$'\e]133;B\a'* ]] || PS1=$PS1$'%{\e]133;B\a%}'
}

_winc_preexec() {
  _winc_urlencode "$1"
  builtin print -n -- "\e]133;C;nonce=$_winc_nonce;cmdline_url=$REPLY\a"
  _winc_running=1
}

# 当前目录所在的 Git 仓库的分支，给提示符用：直接读 .git/HEAD，不在每个提示符前都启动 git。
# 不在仓库里时为空；分离的 HEAD 写提交号的前 7 位。
_winc_branch() {
  builtin emulate -L zsh
  builtin local dir=$PWD git head
  _winc_git=
  while true; do
    if [[ -f $dir/.git/HEAD ]]; then
      git=$dir/.git
      break
    elif [[ -f $dir/.git ]]; then
      # 工作树、子模块：.git 是一个写着 `gitdir: 路径` 的文件。
      git=$(<$dir/.git)
      git=${git#gitdir: }
      [[ $git == /* ]] || git=$dir/$git
      break
    fi
    [[ $dir == / || -z $dir ]] && return
    dir=${dir:h}
  done
  [[ -r $git/HEAD ]] || return
  head=$(<$git/HEAD)
  if [[ $head == 'ref: refs/heads/'* ]]; then
    head=${head#ref: refs/heads/}
  else
    head=${head[1,7]}
  fi
  # 文件内容直接进入提示符：去掉控制字符，其中的转义序列不会被终端执行。
  head=${head//[[:cntrl:]]/}
  _winc_git=" %F{5}${head//\%/%%}%f"
}

# 用户没有自己的提示符时（还是 macOS 的 /etc/zshrc 或 zsh 自带的那个）换成 winc 的：目录（只留
# 最后三级）、Git 分支、提示符号（上一条命令失败时是红的），不花哨；ls 带颜色；Tab 补全用可以
# 上下选的菜单、不分大小写。用了 oh-my-zsh、p10k、starship 或者自己设了 PS1 的都不动。
_winc_defaults() {
  [[ $_winc_prompt == 0 ]] && builtin return
  [[ $PS1 == '%n@%m %1~ %# ' || $PS1 == '%m%# ' ]] || builtin return
  builtin setopt prompt_subst
  PS1='%F{4}%B%(4~|…/%3~|%~)%b%f${_winc_git} %(?.%F{2}.%F{1})❯%f '
  precmd_functions=(_winc_branch $precmd_functions)
  builtin export CLICOLOR=1
  if (( ! $+functions[compdef] )); then
    builtin autoload -Uz compinit
    compinit -i -d "$_winc_home/zcompdump"
  fi
  builtin zstyle ':completion:*' menu select
  builtin zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}'
  _winc_branch
}

# 第一个提示符出来前才把钩子挂到最后：.zshrc 里的提示符主题（p10k、starship）先挂的钩子先跑，
# 我们看到的是它们设好的 PS1，退出码也还是命令的。
_winc_install() {
  builtin local ret=$?
  precmd_functions=(${precmd_functions:#_winc_install} _winc_precmd)
  _winc_defaults
  [[ $_winc_python == 0 ]] || builtin source -- "$_winc_home/../python.sh"
  builtin unset _winc_python
  preexec_functions+=(_winc_preexec)
  () { builtin return $ret }
  _winc_precmd
}
builtin typeset -ga precmd_functions preexec_functions
precmd_functions+=(_winc_install)
