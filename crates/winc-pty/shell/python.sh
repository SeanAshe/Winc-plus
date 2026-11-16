# 在首次提示符前加载启动目录中的 Python 环境，只读取文件属性，不执行项目中的激活脚本。
# 由 bash、zsh 的集成脚本读取；本文件由 winc 生成。

_winc_activate_python() {
    [[ -z ${VIRTUAL_ENV-} && -z ${CONDA_PREFIX-} ]] || builtin return 0
    builtin local name root
    for name in .venv venv; do
        root=$_winc_python_root/$name
        [[ -f $root/pyvenv.cfg && -f $root/bin/python && -x $root/bin/python ]] || continue
        _OLD_VIRTUAL_PATH=$PATH
        if [[ -n ${PYTHONHOME+x} ]]; then
            _OLD_VIRTUAL_PYTHONHOME=$PYTHONHOME
            builtin unset PYTHONHOME
        fi
        builtin export VIRTUAL_ENV="$root" VIRTUAL_ENV_PROMPT="($name) " PATH="$root/bin:$PATH"
        if [[ -z ${VIRTUAL_ENV_DISABLE_PROMPT-} ]]; then
            _OLD_VIRTUAL_PS1=${PS1-}
            PS1="($name) ${PS1-}"
        fi
        deactivate() {
            builtin export PATH="$_OLD_VIRTUAL_PATH"
            builtin unset _OLD_VIRTUAL_PATH
            if [[ -n ${_OLD_VIRTUAL_PYTHONHOME+x} ]]; then
                builtin export PYTHONHOME="$_OLD_VIRTUAL_PYTHONHOME"
                builtin unset _OLD_VIRTUAL_PYTHONHOME
            fi
            if [[ -n ${_OLD_VIRTUAL_PS1+x} ]]; then
                PS1=$_OLD_VIRTUAL_PS1
                builtin unset _OLD_VIRTUAL_PS1
            fi
            builtin unset VIRTUAL_ENV VIRTUAL_ENV_PROMPT
            builtin hash -r 2>/dev/null
            [[ ${1-} == nondestructive ]] || builtin unset -f deactivate
        }
        builtin hash -r 2>/dev/null
        break
    done
}
_winc_activate_python
builtin unset -f _winc_activate_python
builtin unset _winc_python_root
