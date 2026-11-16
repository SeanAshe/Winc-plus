# bash 的 shell 集成：读取用户启动文件，加载启动目录中的 Python 环境，报告命令块和当前目录。
# 使用 OSC 133 和 OSC 7，按 `terminal.prompt` 处理默认提示符，环境加载可添加名称前缀。
# winc 通过 `--rcfile` 读取本文件，启动前按需生成。

# `WINC_PROMPT=0` 时保留默认提示符，由 `terminal.prompt` 控制。
_winc_prompt_on=${WINC_PROMPT-1}
# 记录启动目录，用户配置中的 `cd` 不改变项目环境的选择。
_winc_python_root=$PWD
_winc_python_script=${BASH_SOURCE[0]%/*}/../python.sh
# `WINC_PYTHON=0` 时不加载启动目录中的 Python 环境：用户没有允许这个目录。
_winc_python=${WINC_PYTHON-1}
builtin unset WINC_PROMPT WINC_PYTHON
# 命令行报告附带的随机数，保存在未导出的变量中，供 OSC 133 验证。
_winc_nonce=${WINC_NONCE-}
builtin unset WINC_NONCE

# 本来要作为登录 shell 启动时（--rcfile 对登录 shell 不起作用），按登录 shell 的顺序读。
if [[ -n $WINC_BASH_LOGIN ]]; then
  builtin unset WINC_BASH_LOGIN
  [[ -r /etc/profile ]] && builtin source /etc/profile
  if [[ -r ~/.bash_profile ]]; then
    builtin source ~/.bash_profile
  elif [[ -r ~/.bash_login ]]; then
    builtin source ~/.bash_login
  elif [[ -r ~/.profile ]]; then
    builtin source ~/.profile
  fi
else
  [[ -r ~/.bashrc ]] && builtin source ~/.bashrc
fi

if [[ -z $_winc_hooked ]]; then
  _winc_hooked=1
  _winc_running=
  _winc_at_prompt=
  _winc_histnum=$(HISTTIMEFORMAT= builtin history 1)
  [[ $_winc_histnum =~ ^[[:space:]]*([0-9]+) ]] && _winc_histnum=${BASH_REMATCH[1]}

  # 百分号转义，按字节。结果放在 _winc_reply 里（不开子 shell）。
  _winc_urlencode() {
    builtin local LC_ALL=C
    builtin local s="$1" out= c i
    for ((i = 0; i < ${#s}; i++)); do
      c=${s:i:1}
      case $c in
        [A-Za-z0-9/._~-]) out+=$c ;;
        *)
          # 老的 bash 把 0x80 以上的字节当负数，取低 8 位。
          builtin printf -v c '%d' "'$c"
          builtin printf -v c '%%%02X' $((c & 255))
          out+=$c
          ;;
      esac
    done
    _winc_reply=$out
  }

  # 当前目录所在的 Git 仓库的分支：直接读 .git/HEAD，不启动 git。结果（连同颜色）放在 _winc_git。
  _winc_branch() {
    builtin local dir="$PWD" git head
    _winc_git=
    while :; do
      if [[ -f $dir/.git/HEAD ]]; then
        git=$dir/.git
        break
      elif [[ -f $dir/.git ]]; then
        builtin read -r git <"$dir/.git"
        git=${git#gitdir: }
        [[ $git == /* ]] || git=$dir/$git
        break
      fi
      [[ $dir == / || -z $dir ]] && builtin return
      dir=${dir%/*}
      [[ -z $dir ]] && dir=/
    done
    [[ -r $git/HEAD ]] || builtin return
    builtin read -r head <"$git/HEAD"
    if [[ $head == 'ref: refs/heads/'* ]]; then
      head=${head#ref: refs/heads/}
    else
      head=${head:0:7}
    fi
    # 文件内容直接进入提示符：去掉控制字符，其中的转义序列不会被终端执行。
    head=${head//[[:cntrl:]]/}
    # 变量里的颜色序列用 \001、\002 包起来，readline 才不把它们算进提示符的宽度。
    _winc_git=$' \001\e[35m\002'"$head"$'\001\e[0m\002'
  }

  # 用户没有自己的提示符时（还是系统或 bash 自带的那个）换成 winc 的：目录、Git 分支、提示符号
  # （上一条命令失败时是红的）；ls 带颜色。自己设了 PS1 的不动。下面依次是 macOS 的
  # /etc/bashrc、bash 自带的、部分 Linux 发行版的 /etc/bash.bashrc 和 ~/.bashrc 模板（无颜色、
  # 有颜色）、另一些发行版的 /etc/bashrc（两种转义写法）中的默认值。
  _winc_defaults=
  case $PS1 in
    '\h:\W \u\$ ' | '\s-\v\$ ' \
    | '${debian_chroot:+($debian_chroot)}\u@\h:\w\$ ' \
    | '${debian_chroot:+($debian_chroot)}\[\033[01;32m\]\u@\h\[\033[00m\]:\[\033[01;34m\]\w\[\033[00m\]\$ ' \
    | '[\u@\h \W]\$ ' | '[\u@\h \W]\\$ ')
      [[ $_winc_prompt_on != 0 ]] && _winc_defaults=1 ;;
  esac
  if [[ -n $_winc_defaults ]]; then
    PS1='\[\e[1;34m\]\w\[\e[0m\]${_winc_git} ${_winc_mark} '
    builtin export CLICOLOR=1
  fi
  [[ $_winc_python == 0 ]] || builtin source "$_winc_python_script"
  builtin unset _winc_python_script _winc_python

  # PROMPT_COMMAND 的第一条：记下命令的退出码，原样返回，后面用户自己的 PROMPT_COMMAND 看到的
  # 还是命令的退出码。
  _winc_status() {
    _winc_last=$?
    builtin return $_winc_last
  }

  # PROMPT_COMMAND 的最后一条：提示符主题（starship 之类）已经设好了 PS1。
  _winc_prompt() {
    if [[ -n $_winc_running ]]; then
      # 结束标记附带随机数：命令的输出中伪造的结束标记不会被采用。
      builtin printf '\e]133;D;%s;nonce=%s\a' "$_winc_last" "$_winc_nonce"
      _winc_running=
    fi
    if [[ -n $_winc_defaults ]]; then
      _winc_branch
      if [[ $_winc_last == 0 ]]; then
        _winc_mark=$'\001\e[32m\002❯\001\e[0m\002'
      else
        _winc_mark=$'\001\e[31m\002❯\001\e[0m\002'
      fi
    fi
    _winc_urlencode "$PWD"
    builtin printf '\e]7;file://%s%s\a\e]133;A;signed=1\a' "$HOSTNAME" "$_winc_reply"
    case $PS1 in
      *'\[\e]133;B\a\]'*) ;;
      *) PS1=$PS1'\[\e]133;B\a\]' ;;
    esac
    _winc_at_prompt=1
    # 后面没有别的 PROMPT_COMMAND 了，PS1 里的 `$?` 要看到的是命令的退出码。
    builtin return $_winc_last
  }

  # 用户敲的命令开始运行：DEBUG 陷阱在每条简单命令前都跑，只认提示符之后的第一条。陷阱里传进来
  # 的 `$_` 原样作为最后一个参数，陷阱跑完 `$_` 还是用户的（`mkdir x && cd $_`）。
  _winc_preexec() {
    [[ -n $_winc_at_prompt ]] || builtin return 0
    # 行编辑中由按键执行的命令（`bind -x`）和补全函数不是用户提交的命令，提示符仍在等待输入。
    [[ -n ${READLINE_POINT+set} || -n ${COMP_LINE+set} ]] && builtin return 0
    _winc_at_prompt=
    # 空回车、⌃C：PROMPT_COMMAND 又开始跑了，没有用户的命令。
    [[ $BASH_COMMAND == _winc_status ]] && builtin return 0
    builtin local cmd
    cmd=$(HISTTIMEFORMAT= builtin history 1)
    # 历史的编号没变（HISTCONTROL 没记下这一条、关了历史）时取的是上一条。上一条以这条命令的
    # 第一段开头时是同一条命令重复执行（`ignoredups`），仍取历史里的整行；否则改用 BASH_COMMAND。
    if [[ $cmd =~ ^[[:space:]]*([0-9]+)[*[:space:]]+(.*)$ ]]; then
      if [[ ${BASH_REMATCH[1]} != "$_winc_histnum" ||
        ${BASH_REMATCH[2]} == "$BASH_COMMAND"* ]]; then
        _winc_histnum=${BASH_REMATCH[1]}
        cmd=${BASH_REMATCH[2]}
      else
        cmd=$BASH_COMMAND
      fi
    else
      cmd=$BASH_COMMAND
    fi
    _winc_urlencode "$cmd"
    builtin printf '\e]133;C;nonce=%s;cmdline_url=%s\a' "$_winc_nonce" "$_winc_reply"
    _winc_running=1
  }

  # bash 5.1 起 PROMPT_COMMAND 可以是数组，一项一条命令：这时插在最前、最后两项，只改第一项的话
  # 后面那几项排在 `_winc_prompt` 之后，DEBUG 陷阱把它们当成用户敲的命令。更早的 bash 只跑
  # 第一项，照字符串处理。
  if ((BASH_VERSINFO[0] * 100 + BASH_VERSINFO[1] >= 501)) &&
    [[ $(builtin declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
    PROMPT_COMMAND=(_winc_status "${PROMPT_COMMAND[@]}" _winc_prompt)
  else
    PROMPT_COMMAND=$'_winc_status\n'"${PROMPT_COMMAND}"$'\n_winc_prompt'
  fi
  # 用户已经有自己的 DEBUG 陷阱时不抢：没有「开始运行」的标记，块就不出现，shell 照常工作。
  if [[ -z $(builtin trap -p DEBUG) ]]; then
    builtin trap '_winc_preexec "$_"' DEBUG
  fi
fi
