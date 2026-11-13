## Panel

terminal-dock-bottom = Dock to bottom
terminal-dock-right = Dock to right
terminal-hide-panel = Hide terminal panel
terminal-show-panel = Show terminal panel
terminal-tab-default-name = Terminal
terminal-module-disabled = The terminal module isn’t enabled

## Terminal helper

terminal-helper-missing = Couldn’t find the terminal helper; reinstall Winc
terminal-helper-start-failed = Couldn’t start the terminal helper: { $error }
terminal-helper-exited = The terminal helper exited
terminal-helper-version-mismatch = The terminal helper’s version doesn’t match Winc; reinstall Winc
terminal-process-exited = Process exited (exit code { $code })
terminal-disconnected = Terminal connection lost
terminal-restart = Restart
terminal-restart-confirm = Restart this terminal?
terminal-restart-confirm-detail = The connection to this terminal was lost. Restarting ends any programs still running in it.
terminal-restart-confirm-action = Restart

## Commands and output

terminal-command-fallback = Command
terminal-quoted-command = “{ $command }”
terminal-command-failed = { $name } failed with exit code { $code } ({ $duration })
terminal-command-finished = { $name } finished ({ $duration })
terminal-command-copied = Command copied
terminal-no-failed-commands = No failed commands
terminal-no-error-location = No file locations found in the last failed command’s output
terminal-error-position = Error { $index }/{ $total }: { common-key-enter } Open
terminal-folded-lines-unit = { $count ->
        [one] line
       *[other] lines
    }
terminal-selection-copied = Selection copied
terminal-link-open-failed = Couldn’t open link: { $error }
terminal-notification = { $title }: { $body }

## Find

terminal-find-placeholder = Find in terminal
terminal-find-searching = Searching…
terminal-find-no-results = No results

## Close confirmation

terminal-quit-confirm-one = Quit and end “{ $name }”?
terminal-quit-confirm-many = { $count ->
        [one] Quit and end { $count } running program?
       *[other] Quit and end { $count } running programs?
    }
terminal-quit-confirm-detail = Programs are still running in the terminal. Quitting Winc will end them.
terminal-close-window-confirm-one = Close the window and end “{ $name }”?
terminal-close-window-confirm-many = { $count ->
        [one] Close the window and end { $count } running program?
       *[other] Close the window and end { $count } running programs?
    }
terminal-close-window-confirm-detail = Programs are still running in this window’s terminal. Closing the window will end them.
terminal-switch-folder-confirm-one = Open another folder and end “{ $name }”?
terminal-switch-folder-confirm-many = { $count ->
        [one] Open another folder and end { $count } running program?
       *[other] Open another folder and end { $count } running programs?
    }
terminal-switch-folder-confirm-detail = Programs are still running in the terminal. Opening another folder in this window closes the current folder’s terminals and ends them.
terminal-switch-folder-confirm-button = Open
terminal-close-folder-confirm-one = Close the folder and end “{ $name }”?
terminal-close-folder-confirm-many = { $count ->
        [one] Close the folder and end { $count } running program?
       *[other] Close the folder and end { $count } running programs?
    }
terminal-close-folder-confirm-detail = Programs are still running in the terminal. Closing the folder will end them.
terminal-close-confirm = End “{ $name }”?
terminal-close-confirm-detail = Closing this terminal tab will end the program running in it.
terminal-paste-confirm = { $count ->
        [one] Paste { $count } line into the terminal?
       *[other] Paste { $count } lines into the terminal?
    }
terminal-paste-confirm-detail = The program in the terminal hasn’t enabled bracketed paste, so each line will run as a separate command right away.
terminal-paste-image-failed = Couldn’t save the image: { $error }

## 运行配置。

terminal-shell-select = Select Shell
terminal-run-select = Select Run Configuration
terminal-run-again = Run Again
terminal-run-confirm = Run “{ $name }”?
terminal-run-confirm-detail = This command comes from the project’s .winc/run.json and runs with your user permissions. Run it only if you trust this project.
terminal-run-confirm-detected = Winc generated this command from the project’s files, and it runs code from the project. Run it only if you trust this project.
terminal-run-confirm-command = Command: { $command }
terminal-run-confirm-cwd = Working directory: { $cwd }
terminal-run-confirm-env = Environment variable: { $variable }
terminal-run-confirm-action = Run
terminal-run-config-problem = Run configurations (.winc/run.json): { $problem }
terminal-run-create-failed = Couldn’t create run configurations: { $error }
terminal-run-add = Add Run Configuration
terminal-run-stopped = Stopped
terminal-run-finished = Completed
terminal-run-failed = “{ $name }” failed (exit code { $code })
terminal-run-menu-running = Running · { $command }
