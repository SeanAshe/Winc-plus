## Task names.

format-task-format = Format
format-task-fix = Fix
format-cancelled = Cancelled

## Formatter rows in Settings.

format-tool-custom = Custom
format-tool-from-project = Specified by project settings (.winc/config.json)
format-tool-auto = Auto
format-tool-auto-reason = Auto · based on { $reason }
format-tool-auto-installed = Auto · installed
format-tool-auto-with-reason = { $name } ({ $reason })
format-tool-unset = Not Set
format-tool-fix-unset = The project has no fixer configured; choose a tool to use
format-tool-no-auto = No tool is chosen automatically; choose a tool to use
format-tool-no-preset = No preset tools; choose “Custom Command…”
format-tool-searching = Looking for installed tools…
format-tool-not-installed = Not installed
format-tool-not-installed-names = { $names } not installed
format-tool-list-separator = { " or " }
format-tool-reads-disk = This command reads the file on disk, without unsaved changes
format-tool-not-found = “{ $tool }” not found
format-install-link = How to Install…
format-tool-custom-command = Custom Command…
format-tool-remove = Remove

## Confirmation before running.

format-approval-on-save-what = the project’s “{ $task ->
        [fix] Fix
       *[other] Format
    } on Save” setting
format-approval-on-save-detail = This project’s .winc/config.json asks to { $task ->
        [fix] fix problems
       *[other] format files
    } before every save. The { $task ->
        [fix] fixer
       *[other] formatter
    } reads and runs the project’s configuration, so allow it only if you trust this project.
format-approval-project-command = the { $task ->
        [fix] fix
       *[other] format
    } command in project settings
format-approval-project-command-detail = Command from .winc/config.json: { $command }
format-approval-project-tool = “{ $tool }” in the project
format-approval-project-tool-detail = { $path } is provided by this project. Run it only if you trust this project.
format-approval-allow-question = Allow { $what }?
format-approval-run-question = Allow { $what } to run?
format-approval-allow = Allow
format-approval-run = Run
format-approval-use-system = Use Installed Version
format-approval-deny = Don’t Allow
format-approval-declined = You chose not to run { $what }
format-approval-needed = Confirm whether { $what } may run first

## Formatting results.

format-unsaved-no-language = The file isn’t saved, so its language is unknown; choose a language in the status bar
format-unsaved-reads-disk = The file isn’t saved; the { $language } { $task ->
        [fix] fix
       *[other] format
    } command needs to read the file on disk
format-save-note = { $task ->
        [fix] fixed
       *[other] formatted
    } { $count ->
        [one] { $count } line
       *[other] { $count } lines
    } with { $tool }
format-save-step-failed = didn’t { $task ->
        [fix] fix
       *[other] format
    }: { $error }
format-saved-with-notes = Saved { $name }; { $notes }
format-note-separator = { "; " }
format-clean = Already formatted; no changes needed
format-clean-selection = Selection already formatted; no changes needed
format-clean-named = “{ $name }” is already formatted; no changes needed
format-fix-clean = No problems to fix
format-fix-clean-named = “{ $name }” has no problems to fix
format-modified-during = The file changed while { $task ->
        [fix] fixing
       *[other] formatting
    }; the result wasn’t applied
format-modified-during-named = “{ $name }” changed while { $task ->
        [fix] fixing
       *[other] formatting
    }; the result wasn’t applied
format-applied = { $task ->
        [fix] Fixed
       *[other] Formatted
    } with { $tool }; changed { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
format-applied-selection = { $task ->
        [fix] Fixed
       *[other] Formatted
    } the selection with { $tool }; changed { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
format-applied-named = { $task ->
        [fix] Fixed
       *[other] Formatted
    } “{ $name }” with { $tool }; changed { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
format-failed = Couldn’t { $task ->
        [fix] fix
       *[other] format
    }: { $error }
format-failed-named = Couldn’t { $task ->
        [fix] fix
       *[other] format
    } “{ $name }”: { $error }

## Errors while running tools.

format-unknown-language = Couldn’t recognize the file’s language
format-no-fixer-configured = The project has no { $language } fixer configured ({ $names }); you can specify one in Settings
format-no-auto-tool = { $language } has no { $task ->
        [fix] fixer
       *[other] formatter
    } that can be used automatically; specify a command in Settings
format-command-empty = The command is empty
format-command-unclosed-single-quote = Unclosed single quote in the command
format-command-unclosed-double-quote = Unclosed double quote in the command
format-no-formatter-installed = No { $language } formatter installed ({ $names })
format-no-range = “{ $tool }” can’t format a selection
format-preset-tool-not-found = The project uses { $preset } ({ $reason }), but “{ $tool }” wasn’t found
format-no-permission = “{ $tool }” isn’t executable
format-spawn-failed = “{ $tool }” failed to start ({ $error })
format-timeout = “{ $tool }” didn’t finish within { $count ->
        [one] { $count } second
       *[other] { $count } seconds
    }. Newly installed tools can be slow the first time; try again
format-output-too-large = Output from “{ $tool }” exceeds { $size } MB
format-exit-code = exit code { $code }
format-killed-by-signal = terminated by a signal
format-transcript-note = (Working directory: the file’s folder; { $status })
format-tool-error = “{ $tool }” failed (exit code { $code })
format-tool-exited = “{ $tool }” quit unexpectedly
format-no-output = “{ $tool }” returned no output
format-output-not-utf8 = Output from “{ $tool }” isn’t UTF-8 text
format-eslint-unreadable = Couldn’t read the result from “{ $tool }”
format-eslint-parse-error = Parsing error
format-eslint-fatal = { $message } ({ $line }:{ $column })
format-error-position = { " " }({ $line }:{ $column })
format-file-too-large = The file is too large

## Installation guide.

format-install-title = Install Formatter
format-install-not-found-one = “{ $tool }” wasn’t found.
format-install-not-found-many = No formatter found. Install any of the following; Winc uses the first one installed: { $tools }.
format-install-quoted = “{ $name }”
format-install-list-separator = { ", " }
format-install-intro =
    Winc has no built-in formatters or fixers; it uses tools installed
    on your system or in the project. After installing, just try again;
    there’s no need to restart Winc.
format-install-run-any = Run any of these commands in Terminal (install the package manager first):
format-install-run-here = Run in Terminal (available on this computer):
format-install-other = Other ways:
format-install-docs = Documentation: { $url }
format-install-project-declared = The project’s package.json already declares this dependency; install the project’s dependencies:
format-install-add-to-project = Add it as a dev dependency of the project (same version as your team):
format-install-note-rustfmt = rustfmt comes with the Rust toolchain; if it’s missing, install it with rustup.
format-install-note-gofmt = gofmt comes with Go.
format-install-note-eslint = ESLint is usually installed in the project; this command installs ESLint and creates a configuration file.
format-install-note-clang-format = Xcode and the Command Line Developer Tools on macOS include clang-format.
format-install-note-swift-format = Xcode 16 or later and the Command Line Developer Tools include swift-format.
format-install-note-xmllint = xmllint comes preinstalled on macOS.
