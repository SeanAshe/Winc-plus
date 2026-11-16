# 在首次提示符前加载启动目录中的 Python 环境，只读取文件属性，不执行项目中的激活脚本。
# 由 fish 的集成脚本读取；本文件由 winc 生成。

function __winc_activate_python
    test -z "$VIRTUAL_ENV"; and test -z "$CONDA_PREFIX"; or return
    for name in .venv venv
        set -l root "$__winc_python_root/$name"
        test -f "$root/pyvenv.cfg"; and test -f "$root/bin/python"; and test -x "$root/bin/python"; or continue
        set -g _OLD_VIRTUAL_PATH $PATH
        if set -q PYTHONHOME
            set -g _OLD_VIRTUAL_PYTHONHOME "$PYTHONHOME"
            set -e PYTHONHOME
        end
        set -gx VIRTUAL_ENV "$root"
        set -gx VIRTUAL_ENV_PROMPT "($name) "
        set -gx PATH "$root/bin" $PATH
        if test -z "$VIRTUAL_ENV_DISABLE_PROMPT"
            functions -c fish_prompt _old_virtual_fish_prompt
            function fish_prompt
                printf '%s' "$VIRTUAL_ENV_PROMPT"
                _old_virtual_fish_prompt
            end
        end
        function deactivate
            set -gx PATH $_OLD_VIRTUAL_PATH
            set -e _OLD_VIRTUAL_PATH
            if set -q _OLD_VIRTUAL_PYTHONHOME
                set -gx PYTHONHOME "$_OLD_VIRTUAL_PYTHONHOME"
                set -e _OLD_VIRTUAL_PYTHONHOME
            end
            if functions -q _old_virtual_fish_prompt
                functions -e fish_prompt
                functions -c _old_virtual_fish_prompt fish_prompt
                functions -e _old_virtual_fish_prompt
            end
            set -e VIRTUAL_ENV VIRTUAL_ENV_PROMPT
            if test "$argv[1]" != nondestructive
                functions -e deactivate
            end
        end
        break
    end
end
__winc_activate_python
functions -e __winc_activate_python
set -e __winc_python_root
