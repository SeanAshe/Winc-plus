## Common

workspace-open-folder-first = Open a folder first
workspace-no-open-file = No file is open
workspace-list-separator = { ", " }
workspace-quoted-name = “{ $name }”

## File tree and sidebar

workspace-tree-expand-limit = Too many folders. Only the first { $count } were expanded.
workspace-tree-invalid-name = Invalid name
workspace-tree-name-taken = An item with this name already exists
workspace-tree-cannot-rename = This item can’t be renamed
workspace-tree-cannot-move = This item can’t be moved
workspace-tree-move-into-itself = Can’t move an item into itself
workspace-tree-name-taken-in = “{ $folder }” already contains an item with this name
workspace-tree-target-outside = The destination isn’t in the current folder
workspace-tree-copy-into-itself = Can’t copy an item into itself
workspace-tree-cannot-copy = This item can’t be copied
workspace-tree-tool-new-file = New file
workspace-tree-tool-new-folder = New folder
workspace-tree-tool-refresh = Refresh
workspace-tree-tool-collapse-all = Collapse all folders
workspace-tree-tool-locate = Reveal active file
workspace-sidebar-files = Files
workspace-sidebar-search = Search
workspace-sidebar-commit = Commit
workspace-sidebar-no-folder = No Folder Open
workspace-tree-open-hint-key = { $key } to open a file or folder
workspace-tree-open-hint = Open a file or folder
workspace-tree-open-folder-hint-key = { $key } to open a folder
workspace-tree-open-folder-hint = Open a folder
workspace-tree-empty = This folder is empty
workspace-tree-empty-hint = Double-click here to create a file
workspace-tree-rename-placeholder = New name
workspace-tree-create-placeholder = Name (use / to create subfolders)
workspace-tree-copy-failed = Couldn’t copy “{ $name }”: { $error }
workspace-tree-move-confirm-one = Move “{ $name }” to “{ $folder }”?
workspace-tree-move-confirm-many = { $count ->
        [one] Move { $count } item to “{ $folder }”?
       *[other] Move { $count } items to “{ $folder }”?
    }
workspace-tree-move = Move
workspace-tree-dont-ask = Don’t ask again
workspace-tree-move-failed = Couldn’t move “{ $name }”: { $error }
workspace-tree-drag-items = { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
workspace-tree-drag-copy = Copy { $name }

## Open folder

workspace-open-folder-where = Where do you want to open “{ $name }”?
workspace-open-folder-where-detail = Opening it in this window closes “{ $current }” and its tabs and terminals.
workspace-open-folder-this-window = This Window
workspace-open-folder-new-window = New Window

## Start page

workspace-start-settings = Settings
workspace-start-new-file = New File
workspace-start-open = Open File or Folder
workspace-start-open-file = Open File
workspace-start-open-folder = Open Folder
workspace-start-version = Version { $version }
workspace-start-section-start = Start
workspace-start-section-recent = Recent

## Global search

workspace-search-placeholder = Search
workspace-search-replace-placeholder = Replace
workspace-search-include = Include
workspace-search-include-example = e.g. src, *.rs
workspace-search-exclude = Exclude
workspace-search-include-ignored = Search Ignored Files (excluded by .gitignore and similar rules)
workspace-search-exclude-example = e.g. *.min.js, docs
workspace-search-searching = Searching…
workspace-search-summary = { $total ->
        [one] { $total }{ $more } result
       *[other] { $total }{ $more } results
    } in { $files ->
        [one] { $files } file
       *[other] { $files } files
    }
workspace-search-summary-searching = { $total ->
        [one] { $total }{ $more } result
       *[other] { $total }{ $more } results
    } in { $files ->
        [one] { $files } file
       *[other] { $files } files
    }, searching…
workspace-search-no-results = No results found
workspace-search-no-results-hint = Try different keywords, or adjust the match options and filters
workspace-search-no-results-hint-ignored = Try other keywords or match options. Ignored files aren’t searched; turn them on next to Exclude.
workspace-search-not-utf8 = Not UTF-8 encoded; can’t replace
workspace-search-tool-refresh = Search again
workspace-search-tool-clear = Clear search results
workspace-search-tool-expand-all = Expand all
workspace-search-tool-collapse-all = Collapse all
workspace-search-hide-filters = Hide filters
workspace-search-filter-files = Filter files
workspace-search-row-replace-all = Replace all
workspace-search-row-replace = Replace
workspace-search-row-dismiss = Remove from results
workspace-search-regex-error = Invalid regular expression: { $error }
workspace-search-include-error = Invalid include pattern: { $error }
workspace-search-exclude-error = Invalid exclude pattern: { $error }
workspace-search-line-copied = Line copied

## Global replace

workspace-replace-all-busy = The previous Replace All hasn’t finished; try again later
workspace-replace-all-searching = The search hasn’t finished; try Replace All again later
workspace-replace-all-too-many = More than { $max } results; narrow the search before replacing all
workspace-replace-all-nothing = No results to replace
workspace-replace-all-confirm-delete = Delete { $total ->
        [one] { $total } match
       *[other] { $total } matches
    } in { $files ->
        [one] { $files } file
       *[other] { $files } files
    }?
workspace-replace-all-confirm = Replace { $total ->
        [one] { $total } match
       *[other] { $total } matches
    } in { $files ->
        [one] { $files } file
       *[other] { $files } files
    } with “{ $replacement }”?
workspace-replace-all-button = Replace
workspace-replace-all-warning = Files that aren’t open will be modified directly. This can’t be undone.
workspace-replace-all-changed = The search results have changed; run Replace All again
workspace-replace-all-done = Replaced { $count ->
        [one] { $count } occurrence
       *[other] { $count } occurrences
    } in { $files ->
        [one] { $files } file
       *[other] { $files } files
    }
workspace-replace-all-unsaved = , { $opened } of them open and not yet saved
workspace-replace-all-stale = { $stale ->
        [one] ; { $stale } changed after the search and wasn’t replaced
       *[other] ; { $stale } changed after the search and weren’t replaced
    }
workspace-replace-all-failed = { $count ->
        [one] ; { $count } file wasn’t replaced ({ $error })
       *[other] ; { $count } files weren’t replaced ({ $error })
    }
workspace-replace-stale = The file changed after the search and this no longer matches; search again
workspace-replace-failed = Couldn’t replace: { $error }
workspace-replace-failed-file = { $name }: { $error }

## Find bar and prompt bar

workspace-find-regex-invalid = Invalid regex
workspace-find-searching = Searching…
workspace-find-no-results = No results
workspace-find-count = { $count ->
        [one] { $count }{ $more } match
       *[other] { $count }{ $more } matches
    }
workspace-find-placeholder = Find
workspace-find-replace-placeholder = Replace
workspace-find-toggle-replace = Toggle replace
workspace-find-match-case = Match case
workspace-find-whole-word = Match whole word
workspace-find-use-regex = Use regular expression
workspace-find-previous = Previous match
workspace-find-next = Next match
workspace-find-filter-lines = Show only matching lines
workspace-find-close = Close
workspace-find-replace = Replace
workspace-find-replace-all = Replace all
workspace-goto-title = Go to Line
workspace-goto-placeholder = line:column
workspace-goto-total = { $count ->
        [one] { $count } line
       *[other] { $count } lines
    }
workspace-stash-title = Stash Changes
workspace-stash-placeholder = Message (optional)
workspace-stash-hint = Includes untracked files · ↩ Stash
workspace-tag-title = New Tag
workspace-tag-placeholder = Tag name
workspace-tag-hint = From { $commit } · ↩ Create

## Quick open

workspace-picker-commands = { $count ->
        [one] { $count } command
       *[other] { $count } commands
    }
workspace-picker-symbols = { $count ->
        [one] { $count } symbol
       *[other] { $count } symbols
    }
workspace-picker-files-matched = { $total ->
        [one] { $matched } / { $total } file
       *[other] { $matched } / { $total } files
    }
workspace-picker-files = { $count ->
        [one] { $count } file
       *[other] { $count } files
    }
workspace-picker-placeholder = Search files by name…
workspace-picker-no-folder = Open a folder first, or type > to search commands
workspace-picker-indexing = Indexing files…
workspace-picker-goto-line-column = Go to line { $line }, column { $column }
workspace-picker-goto-line = Go to line { $line }
workspace-picker-folder-empty = No files in this folder
workspace-picker-no-matching-files = No matching files. Ignored files aren’t searched.
workspace-picker-recently-opened = Recently opened
workspace-picker-no-matching-commands = No matching commands
workspace-picker-recently-used = Recently used
workspace-picker-symbols-too-large = File too large to list symbols
workspace-picker-no-symbols = No symbols found in this file
workspace-picker-no-matching-symbols = No matching symbols
workspace-conflict-modified = “{ $name }” was changed by another app

## Tooltips

workspace-tip-continue-rebase = Continue rebase
workspace-tip-finish-operation = Complete { $operation }
workspace-tip-amend = Amend last commit
workspace-tip-commit-to = Commit to { $branch }
workspace-tip-commit = Commit
workspace-tip-open-file = Open file
workspace-tip-revert-change = Revert change
workspace-tip-commit-staged-only = Commits only the staged part; the rest stays in the working tree
workspace-tip-commit-menu = More commit options
workspace-tip-git-log = Git log
workspace-tip-more-actions = More actions
workspace-tip-show-as-tree = Show as Tree
workspace-tip-show-as-list = Show as List
workspace-tip-previous-conflict = Previous conflict
workspace-tip-next-conflict = Next conflict
workspace-tip-worktree-of = Worktree of “{ $name }” · { $path }
workspace-tip-previous-change = Previous change
workspace-tip-next-change = Next change
workspace-tip-previous-file = Previous file
workspace-tip-next-file = Next file
workspace-tip-diff-inline = Switch to inline view
workspace-tip-diff-side-by-side = Switch to side-by-side view
workspace-tip-open-diff = Open in diff view
workspace-tip-close-esc = Close (Esc)
workspace-tip-revert-to-left = Revert change (restore left side)
workspace-tip-include-change = Include this change in the commit
workspace-tip-expand-unchanged = Expand all unchanged regions
workspace-tip-collapse-unchanged = Collapse unchanged regions
workspace-tip-close-tab = Close
workspace-tip-all-tabs = Show all tabs
workspace-tip-commit-changes = Commit · Changes: { $count }
workspace-tip-log-go-to = Go to { $commit } { $subject }
workspace-tip-log-target-not-loaded = The target commit hasn’t loaded yet

## Status bar

workspace-status-spaces = Spaces: { $size }
workspace-status-update = Update Available: { $version }
workspace-status-update-downloading = Downloading Update…
workspace-status-update-ready = Restart to Update
workspace-status-not-shown = Not shown: { $features }
workspace-status-position-selected = { $position } ({ $count } selected)
workspace-status-selections = { $count ->
        [one] { $count } selection
       *[other] { $count } selections
    }
workspace-status-selections-chars = { $count ->
        [one] { $count } selection ({ $chars } selected)
       *[other] { $count } selections ({ $chars } selected)
    }
workspace-status-readonly = Read-Only
workspace-status-large-file = Large File
workspace-status-branch-running = { $task }…
workspace-status-branch-operation = { $operation } in progress
workspace-status-formatting = Formatting…
workspace-status-fixing = Fixing…
workspace-reopen-discard = Discard Changes and Reopen
workspace-reopen-confirm = Discard changes and reopen with a different encoding?
workspace-reopen-confirm-detail = All unsaved changes will be lost.
workspace-reopen-dirty = There are unsaved changes; save or undo them before reopening
workspace-reopen-failed = Couldn’t reopen: { $error }
workspace-reopen-lossy = Some content couldn’t be decoded as { $encoding }; opened as read-only
workspace-indent-detected = Detected indentation from content: { $indent }

## Breadcrumbs

workspace-breadcrumb-read-failed = Couldn’t read: { $error }
workspace-breadcrumb-empty = Empty folder
workspace-breadcrumb-more = { $count ->
        [one] { $count } more item not shown
       *[other] { $count } more items not shown
    }
workspace-key-path-copied = Copied { $path }
workspace-key-path-none = The cursor isn’t in a JSON, YAML, or TOML key

## Compare files

workspace-compare-read-failed = Couldn’t read “{ $name }”: { $error }
workspace-compare-too-large = “{ $name }” is too large to compare
workspace-compare-binary = “{ $name }” is a binary file and can’t be compared
workspace-compare-identical = “{ $left }” and “{ $right }” are identical

## Opening and saving files

workspace-open-default-app-failed = Couldn’t open with the default app: { $error }
workspace-reveal-failed = { $action } failed: { $error }
workspace-terminal-app-fallback = Terminal
workspace-open-app-failed = Couldn’t open “{ $app }”: { $error }
workspace-deleted-on-disk-key = “{ $name }” was deleted from disk and wasn’t auto-saved; press { $key } to recreate it
workspace-deleted-on-disk = “{ $name }” was deleted from disk and wasn’t auto-saved; save it to recreate it
workspace-save-as-dirty-elsewhere = Couldn’t save as “{ $name }”: that file has unsaved changes in another tab
workspace-save-conflict = Couldn’t save “{ $name }”: the file was changed by another app; choose which version to keep first
workspace-save-failed = Couldn’t save “{ $name }”: { $error }
workspace-saved = Saved { $name }
workspace-drop-to-open = Drag a file or folder into the window to open it
workspace-picker-unavailable = No file chooser is available. Install xdg-desktop-portal, or drag files and folders into the window to open them
workspace-picker-failed = Couldn’t open the file chooser: { $error }
workspace-open-folder-failed = Couldn’t open folder “{ $name }”: { $error }
workspace-recent-missing = Couldn’t find “{ $path }”; removed it from the recent list
workspace-reloaded = Reloaded { $names }
workspace-disk-conflicts = { $names } changed on disk and conflict with unsaved changes; switch to each tab to choose which version to keep

## Closing

workspace-close-save-one = Do you want to save the changes you made to “{ $name }”?
workspace-close-save-detail = Your changes will be lost if you don’t save them.
workspace-close-save = Save
workspace-close-more-files = { $count ->
        [one] …and { $count } more file
       *[other] …and { $count } more files
    }
workspace-close-save-many = { $count ->
        [one] Do you want to save the changes to the following { $count } file?
       *[other] Do you want to save the changes to the following { $count } files?
    }
workspace-close-save-all = Save All
workspace-close-dont-save = Don’t Save
workspace-reopen-tab-none = No closed tabs to reopen

## Notices and window

workspace-notice-show-output = Show Output
workspace-notice-install = How to Install
workspace-notice-open-settings = Open Settings
workspace-format-output-title = Formatter Output
workspace-font-missing = Couldn’t use the configured fonts { $fonts }; using the default font instead (press ⌘, to open Settings)
workspace-font-missing-item = { $key } “{ $name }”
workspace-font-unusable = This font can’t be used; choose another font
workspace-stdin-title = Standard Input
workspace-stdin-read-failed = Error reading standard input: { $error }
workspace-stdin-truncated = Only the first { $size } MB of standard input was read
workspace-backup-locked = This folder is open in another winc window; unsaved changes in this window won’t be backed up automatically
workspace-backup-failed = Couldn’t back up unsaved changes: { $error }
