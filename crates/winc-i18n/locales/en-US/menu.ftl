## Menu bar and Dock menu

menu-settings = Settings…
menu-install-command = Install winc Command…
menu-release-notes = Release Notes
menu-check-updates = Check for Updates
menu-window = Window
menu-window-minimize = Minimize
menu-window-zoom = Zoom
menu-window-bring-all-to-front = Bring All to Front

## Status bar menus

menu-reopen-with-encoding = Reopen with Encoding
menu-save-with-encoding = Save with Encoding
menu-indent-size = Indent Size
menu-tab-size = Tab Size

## Editor and file menus

menu-table = Table
menu-open-recent-in-new-window = Open “{ $name }” in New Window
menu-copied-path = Copied path
menu-copied-relative-path = Copied relative path
menu-copied-reference = Copied { $reference }
menu-copy-reference-unsaved = The file hasn’t been saved and has no path

## Formatter menu

menu-tool-auto-format-desc = Uses the project’s configured tool first, then an installed one
menu-tool-auto-fix-desc = Uses only the project’s configured fix tool
menu-tool-not-installed = { $name } (Not Installed)

## Commit menu in the log

menu-log-checkout-branch = Check Out “{ $branch }”
menu-log-new-worktree-branch = Check Out “{ $branch }” in New Worktree
menu-log-cherry-pick-onto = Cherry-Pick onto “{ $branch }”
menu-log-reset-branch = Reset “{ $branch }” to This Commit
menu-log-reset-soft-desc = Keep all changes staged
menu-log-reset-mixed-desc = Keep changes in the working tree, unstaged
menu-log-reset-hard-desc = Discard all uncommitted changes
menu-form-base-current-branch = { $branch } (Current Branch)
menu-form-base-current-commit = Current Commit

## Branch menu

menu-git-current-branch = Current Branch
menu-git-switch-to-worktree = Switch to Worktree “{ $name }”
menu-git-checkout-stale-worktree = The worktree’s folder no longer exists. Prune stale worktrees first
menu-git-finish-operation = Finish { $operation }…
menu-git-continue-operation = Continue { $operation }
menu-git-abort-operation = Abort { $operation }
menu-git-continue-desc = Resolve the conflicts in the files, then mark them as resolved
menu-git-abort-desc = Restore the state from before the operation started
menu-git-skip = Skip This Commit
menu-git-skip-desc = Discard this commit’s changes and continue with the rest
menu-git-push-ahead = { $count ->
        [one] { $count } commit → { $upstream }
       *[other] { $count } commits → { $upstream }
    }
menu-git-push-up-to-date = To { $upstream }, nothing to push
menu-git-push-set-upstream = Push “{ $branch }” and set it as upstream
menu-git-pull-behind = { $count ->
        [one] { $count } commit ← { $upstream }
       *[other] { $count } commits ← { $upstream }
    }
menu-git-pull-from = From { $upstream }
menu-git-pull-no-upstream = The current branch has no upstream
menu-git-fetch-desc = Download remote updates without changing local branches

## Context menu items

menu-new-file = New File…
menu-new-folder = New Folder…
menu-open-in-terminal = Open in Terminal
menu-open-with-winc = Open with Winc
menu-close-tabs-right = Close Tabs to the Right
menu-close-tabs-left = Close Tabs to the Left
menu-close-saved-tabs = Close Saved Tabs
menu-compare-selected = Compare Selected Files
menu-reveal-in-tree = Reveal in File Tree
menu-tree-collapse-all = Collapse All Folders
menu-tree-locate = Reveal Active File
menu-reveal-source = Show in Source
menu-add-bookmark = Add Bookmark
menu-remove-bookmark = Remove Bookmark
menu-detect-language = Detect from File Name
menu-search-dismiss = Remove from Results
menu-terminal-clear = Clear Terminal
menu-terminal-maximize = Maximize Panel
menu-terminal-close = Close Terminal
menu-terminal-copy-command = Copy Command
menu-terminal-copy-output = Copy Output
menu-terminal-rerun = Run Again

## Table menu

menu-table-insert-row-above = Insert Row Above
menu-table-insert-row-below = Insert Row Below
menu-table-insert-column-left = Insert Column Left
menu-table-insert-column-right = Insert Column Right
menu-table-align-left = Align Left
menu-table-align-center = Align Center
menu-table-align-right = Align Right

## Git menu items

menu-new-worktree = New Worktree…
menu-git-fetch = Fetch
menu-git-pull = Pull
menu-git-push = Push
menu-git-continue = Continue
menu-git-show-diff = Show Diff
menu-git-open-file = Open File
menu-git-pull-rebase = Pull with Rebase
menu-log-tag = Tag “{ $name }”
menu-git-roll-back = Roll Back
menu-git-stash-files = Stash…
menu-git-ignore = Add to .gitignore
menu-git-ignore-file = This File
menu-git-ignore-extension = All Files with This Extension
menu-git-ignore-folder = Containing Folder
menu-git-roll-back-change = Roll Back This Change
menu-git-file-history = Show File History
menu-git-blame = Blame
menu-git-stash = Stash Changes…
menu-git-local-branches = Local Branches
menu-git-remote-branches = Remote Branches
menu-log-checkout = Check Out Commit
menu-log-new-worktree = Check Out This Commit in New Worktree
menu-log-cherry-pick = Cherry-Pick onto Current Branch
menu-log-revert = Revert Commit
menu-log-reset-soft = Soft Reset
menu-log-reset-mixed = Mixed Reset
menu-log-reset-hard = Hard Reset…
menu-log-new-tag = New Tag…
menu-log-interactive-rebase = Interactively Rebase from Here…
menu-log-copy-oid = Copy Commit Hash
menu-log-copy-short-oid = Copy Short Commit Hash
menu-log-copy-patch = Copy as Patch
menu-log-compare-with-local = Compare with Local
menu-log-copy-message = Copy Commit Message
menu-log-go-to = Go to Hash, Branch or Tag…
menu-log-all-branches = All Branches
menu-log-all-dates = All Dates
menu-log-all-paths = All Paths
menu-log-other-path = Other Path…
menu-log-last-day = Last 24 Hours
menu-log-last-days = Last { $count } Days
menu-log-last-year = Last Year
menu-log-author-me = Me
menu-log-column-author = Author
menu-log-column-date = Date
menu-log-column-hash = Hash
menu-log-all-authors = All Authors
menu-commit-check = Check
menu-commit-uncheck = Uncheck
menu-git-stage = Stage
menu-git-unstage = Unstage
menu-commit-list = Show as List
menu-commit-change-list = Changelist
menu-commit-sort-path = Sort by Path
menu-commit-sort-name = Sort by Name
menu-commit-sort-status = Sort by Status
menu-commit-view = View and Sort
menu-commit-discard-all = Discard All Changes…
menu-commit-check-all = Check All
menu-commit-uncheck-all = Uncheck All
menu-commit-commits = Commit
menu-commit-changes = Changes
menu-git-tags = Tags
menu-commit-tree = Show as Tree
menu-commit-staging-area = Staging View
menu-git-branches = Branches
menu-git-worktrees = Worktrees
menu-git-repositories = Repositories
menu-commit-all-repositories = Show All Repositories
menu-git-show-output = Show Git Output
menu-commit-expand-all = Expand All
menu-commit-collapse-all = Collapse All
menu-commit-roll-back = Roll Back Checked Files
menu-commit-undo = Undo Commit
menu-commit-and-push = Commit and Push
menu-commit-and-sync = Commit and Sync
menu-commit-amend = Amend Last Commit
menu-commit-sign-off = Sign Off (Signed-off-by)
menu-commit-checks = Commit Checks
menu-commit-reformat = Reformat Code
menu-commit-check-todo = Check TODOs
menu-commit-recent-messages = Recent Commit Messages
menu-commit-co-author = Add Co-Author
menu-commit-no-verify = Skip Hooks for This Commit
