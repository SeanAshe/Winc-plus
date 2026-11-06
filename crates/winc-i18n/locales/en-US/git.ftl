## Git command line: locating, running and reading output

git-unavailable-not-installed = Git not found
git-unavailable-no-developer-tools = Command Line Developer Tools aren't installed
git-status-unreadable = Couldn't read the output of git status
git-status-truncated = Too many changed files in the repository; only the first { $count } are shown
git-push-no-remote = This repository has no remote configured
git-file-outside-repository = File isn't in a Git repository
git-run-failed = Git failed
git-run-spawn-failed = Couldn't run Git ({ $error })
git-commit-partial-during-operation = { $operation } in progress; can't commit only some changes
git-trust-config-unreadable = Couldn't read the repository's configuration
git-trust-required = This repository is set up to run programs automatically during Git operations ({ $commands }). Trust the repository to perform this action
git-list-separator = { ", " }
git-worktree-copy-invalid-glob = Invalid pattern “{ $glob }” in git.worktreeCopy: { $error }
git-worktree-copy-failed = Couldn't copy “{ $path }”: { $error }

## Operations in progress

git-operation-merge = Merge
git-operation-rebase = Rebase
git-operation-cherry-pick = Cherry-Pick
git-operation-revert = Revert

## Resolving merge conflicts

git-conflict-accept-current = Accept Current
git-conflict-accept-incoming = Accept Incoming
git-conflict-accept-both = Accept Both

## Branch menu

git-branch-checkout = Check Out
git-branch-checkout-new-worktree = Check Out in New Worktree
git-branch-new-from = New Branch from This Branch…
git-branch-merge-into-current = Merge into Current Branch
git-branch-rebase-current-onto = Rebase Current Branch onto This Branch
git-branch-delete = Delete Branch…
git-branch-delete-confirm = Delete branch “{ $name }”?
git-branch-delete-confirm-detail = Only the local branch is deleted. Remote branches aren't affected.
git-branch-delete-confirm-button = Delete
git-branch-force-delete-confirm = Branch “{ $name }” isn't fully merged. Delete it anyway?
git-branch-force-delete-confirm-detail = Commits that exist only on this branch may be lost.
git-branch-force-delete-confirm-button = Delete Anyway
git-branch-compare-with-current = Compare with Current Branch
git-stash-menu = Stashes ({ $count })
git-stash-apply = Apply
git-stash-pop = Pop
git-stash-drop = Drop…
git-stash-drop-confirm = Drop stash “{ $name }”?
git-stash-gone = This stash no longer exists
git-stash-drop-confirm-detail = The changes in this stash can't be recovered after it's dropped.
git-stash-drop-confirm-button = Drop

## Background Git operations: what is running, and messages when it finishes

git-op-fetch = fetching
git-op-pull = pulling
git-op-push = pushing
git-op-sync = syncing
git-op-stash = stashing changes
git-op-stash-apply = applying stash “{ $name }”
git-op-stash-pop = popping stash “{ $name }”
git-op-continue-merge = continuing the merge
git-op-continue-rebase = continuing the rebase
git-op-continue-cherry-pick = continuing the cherry-pick
git-op-continue-revert = continuing the revert
git-op-abort-merge = aborting the merge
git-op-abort-rebase = aborting the rebase
git-op-abort-cherry-pick = aborting the cherry-pick
git-op-abort-revert = aborting the revert
git-op-checkout = checking out “{ $name }”
git-op-merge = merging “{ $name }”
git-op-rebase-onto = rebasing onto “{ $name }”
git-op-delete-branch = deleting branch “{ $name }”
git-op-new-branch = creating branch “{ $name }”
git-op-new-tag = creating tag “{ $name }”
git-op-busy = The previous Git operation hasn't finished yet
git-op-done = Finished { $what }
git-op-done-summary = Finished { $what }: { $summary }
git-op-done-in = Finished { $what } in “{ $name }”
git-op-failed = Error { $what }: { $error }
git-op-failed-in = Error { $what } in “{ $name }”: { $error }

## Worktrees
git-worktree-detached = { $oid } (detached HEAD)
git-worktree-missing = Folder no longer exists
git-worktree-copied-ignored = { $count ->
    [one] Copied { $count } ignored file
   *[other] Copied { $count } ignored files
}
git-worktree-open-elsewhere = Worktree “{ $name }” is open in another window
git-worktree-open-elsewhere-detail = Close that window first, then remove this worktree.
git-worktree-switch-to-window = Switch to Window
git-worktree-close-window-first = Close the window showing worktree “{ $name }” first, then remove this worktree
git-worktree-remove-confirm = Remove worktree “{ $name }”?
git-worktree-remove-confirm-detail = The worktree folder will be moved to the { common-trash }, where you can restore it.
git-worktree-remove-unreachable = { $count ->
    [one] { $count } commit on the detached HEAD doesn't belong to any branch and can't be recovered after removal. To keep it, create a branch for it in this worktree first.
   *[other] { $count } commits on the detached HEAD don't belong to any branch and can't be recovered after removal. To keep them, create a branch for them in this worktree first.
}
git-worktree-modified-files = { $count ->
    [one] { $count } modified file
   *[other] { $count } modified files
}
git-worktree-untracked-files = { $count ->
    [one] { $count } untracked file
   *[other] { $count } untracked files
}
git-worktree-files-separator = { " and " }
git-worktree-remove-uncommitted = This worktree has uncommitted changes: { $files }.
git-worktree-remove-button = Remove
git-worktree-remove-delete-branch = Also delete branch “{ $branch }”
git-worktree-close-dialog-first = Close the current dialog first, then remove the worktree
git-worktree-trash-failed = Couldn't remove worktree “{ $name }”: { $error }
git-worktree-remove-record-failed = The folder was moved to the { common-trash }, but the worktree record couldn't be removed from Git ({ $error }). Use “Prune Stale Worktrees” later
git-worktree-branch-deleted = Deleted branch “{ $branch }”
git-worktree-branch-delete-failed = The folder was moved to the { common-trash }, but branch “{ $branch }” couldn't be deleted: { $error }
git-op-checkout-new-worktree = checking out “{ $name }” in a new worktree
git-op-checkout-commit-new-worktree = checking out { $commit } in a new worktree
git-op-prune-worktrees = pruning stale worktrees
git-op-check-worktree = checking worktree “{ $name }”
git-op-remove-worktree = removing worktree “{ $name }”

## Log details pane

git-log-no-selection = Select a Commit
git-log-no-selection-detail = Select a commit on the left to see its message and changed files
git-log-details-commit = Commit
git-log-details-parents = { $count ->
    [one] Parent
   *[other] Parents
}
git-log-details-loading-files = Loading changed files…
git-compare-title = Compare Two Versions
git-compare-old = Old
git-compare-new = New
git-compare-no-differences = No differences between the two versions
git-compare-comparing = Comparing…
git-log-hint-select-file = Select File
git-log-hint-open-diff = Open Diff
git-log-hint-return = Back
git-log-hint-file-list = File List
git-log-hint-exit-compare = Exit Compare
git-log-hint-click = Click
git-log-hint-view-commit = View Single Commit
git-log-hint-double-click = Double-Click
git-log-hint-cmd-click = ⌘-Click
git-log-hint-compare = Compare
git-log-hint-go-back = Back

## Commit times

git-time-just-now = Just now
git-time-minutes-ago = { $count ->
    [one] { $count } minute ago
   *[other] { $count } minutes ago
}
git-time-today = Today { $time }
git-time-yesterday = Yesterday { $time }
git-time-date = { $month ->
    [1] Jan
    [2] Feb
    [3] Mar
    [4] Apr
    [5] May
    [6] Jun
    [7] Jul
    [8] Aug
    [9] Sep
    [10] Oct
    [11] Nov
   *[12] Dec
} { $day }
git-time-date-year = { $month ->
    [1] Jan
    [2] Feb
    [3] Mar
    [4] Apr
    [5] May
    [6] Jun
    [7] Jul
    [8] Aug
    [9] Sep
    [10] Oct
    [11] Nov
   *[12] Dec
} { $day }, { $year }

## Committing and rolling back

git-commit-message-required = Enter a commit message first
git-commit-unresolved-conflicts = { $count ->
    [one] { $count } file still has unresolved conflicts. Resolve them before committing
   *[other] { $count } files still have unresolved conflicts. Resolve them before committing
}
git-commit-nothing-checked = No files selected to commit
git-commit-done = Committed { $oid }: { $subject }
git-commit-failed = Couldn't commit: { $error }
git-commit-done-in = Committed { $oid } in “{ $name }”
git-commit-failed-in = Couldn't commit in “{ $name }”: { $error }
git-undo-commit-confirm = Undo the last commit?
git-undo-commit-confirm-detail = The changes in “{ $subject }” return to the staging area, and its message returns to the input box.
git-undo-commit-confirm-button = Undo Commit
git-force-push-confirm = Force push “{ $branch }”?
git-force-push-confirm-detail = { $count ->
    [one] Pushed commits were rewritten locally. 1 upstream commit will be replaced by the local version. The push is rejected if upstream has received new commits in the meantime.
   *[other] Pushed commits were rewritten locally. { $count } upstream commits will be replaced by the local version. The push is rejected if upstream has received new commits in the meantime.
}
git-force-push-confirm-button = Force Push
git-commit-push-confirm = Commit and push?
git-commit-push-confirm-scope = { $count ->
    [one] 1 file will be committed to { $branch } and pushed to upstream.
   *[other] { $count } files will be committed to { $branch } and pushed to upstream.
}
git-commit-push-confirm-detail = The commit will be pushed to upstream.
git-commit-push-confirm-button = Commit and Push
git-undo-commit-done = Undid commit { $oid }; its changes are staged
git-undo-commit-done-in = Undid commit { $oid } in “{ $name }”
git-undo-commit-failed = Couldn't undo the commit: { $error }
git-commit-partial-not-utf8 = “{ $name }” isn't UTF-8 text, so its changes can't be partially committed
git-abort-confirm = Abort { $operation }?
git-abort-confirm-detail = The working tree will be restored to its state before the operation began, and resolved conflicts will be discarded.
git-abort-confirm-button = Abort { $operation }
git-rollback-nothing-checked = No files selected to roll back
git-rollback-confirm-one = Roll back changes to “{ $name }”?
git-rollback-confirm-many = Roll back changes to { $count } files?
git-rollback-confirm-detail = Tracked files will be restored to their contents in HEAD; untracked files will be moved to the { common-trash }, where you can restore them.
git-rollback-confirm-button = Roll Back
git-rollback-file-failed = Couldn't roll back “{ $name }”: { $error }
git-rollback-done = { $count ->
    [one] Rolled back { $count } file
   *[other] Rolled back { $count } files
}
git-rollback-failed = Couldn't roll back: { $error }
git-rollback-done-in = { $count ->
    [one] Rolled back { $count } file in “{ $name }”
   *[other] Rolled back { $count } files in “{ $name }”
}
git-rollback-failed-in = Couldn't roll back in “{ $name }”: { $error }
git-read-file-failed = Couldn't read “{ $name }”: { $error }

## Repository status and file diffs

git-unusable = Can't use Git: { $error }
git-not-a-repository = The current folder isn't a Git repository
git-no-file-in-tab = The current tab has no file
git-diff-title = { $name } · Diff
git-diff-submodule = “{ $name }” is a submodule; no diff is shown
git-diff-head-unreadable = Couldn't read the HEAD version of “{ $name }”: { $error }
git-diff-too-large = “{ $name }” is too large; no diff is shown
git-diff-binary = “{ $name }” is a binary file; no diff is shown
git-diff-identical = “{ $name }” is the same as HEAD; no changes
git-diff-side-worktree-deleted = Working Tree · Deleted
git-diff-side-head-missing = HEAD · File Doesn't Exist

## Trusting repository configuration

git-trust-confirm = Do you trust the repository “{ $name }”?
git-trust-confirm-detail = This repository is set up to run the programs below automatically during Git operations such as committing and pushing. Choose “Trust” only if you know the repository comes from a reliable source. If you don't trust it, winc still shows the status of your changes, but actions that would run these programs, such as committing and switching branches, are unavailable.
git-trust-more = { $count ->
    [one] …{ $count } item in total
   *[other] …{ $count } items in total
}
git-trust-deny = Don't Trust
git-trust-allow = Trust

## Commit view

git-commit-view-problem = { $problem }. Refresh from “⋯” in the top-right corner to try again
git-commit-view-loading = Reading repository status…
git-commit-view-empty = No Changes
git-commit-view-empty-detail = The working tree matches HEAD
git-operation-banner = { $operation } in progress
git-operation-banner-resolved = { $operation } in progress · Conflicts resolved
git-operation-banner-conflicts = { $count ->
    [one] { $operation } in progress · { $count } file has conflicts
   *[other] { $operation } in progress · { $count } files have conflicts
}
git-operation-abort = Abort
git-commit-message-placeholder = Commit message ({ $key } to commit)
git-commit-message-amend-placeholder = Amended commit message
git-branch-picker-placeholder = Search or create a branch
git-branch-picker-name = New branch name
git-branch-picker-from = Create from { $branch }
git-branch-picker-from-carry = { $count ->
    [one] Create from { $branch }. Uncommitted changes in 1 file stay in the working tree.
   *[other] Create from { $branch }. Uncommitted changes in { $count } files stay in the working tree.
}
git-branch-picker-new = New Branch
git-branch-picker-new-worktree = New Worktree
git-branch-picker-create-row = Create branch “{ $name }”
git-branch-picker-current = Current
git-picker-back = Back
git-location-picker-placeholder = Search repositories and worktrees
git-location-picker-empty = No matching repositories or worktrees
git-location-main-worktree = Main worktree
git-location-only-one = This folder has only one repository and worktree
git-location-new-window = Open in New Window
git-location-remove = Remove Worktree…
git-location-prune = Clean Up Stale Worktrees
git-location-new-worktree-in = New Worktree in “{ $repository }”
git-location-count = { $count ->
    [one] 1 worktree
   *[other] { $count } worktrees
}
git-tip-other-worktrees = { $count ->
    [one] 1 other worktree has uncommitted changes
   *[other] { $count } other worktrees have uncommitted changes
}
git-tip-other-locations = { $count ->
    [one] 1 other repository or worktree has uncommitted changes
   *[other] { $count } other repositories or worktrees have uncommitted changes
}
git-tip-fetch = Fetch from remotes
git-tip-in-sync = Fetch from remotes (in sync with { $upstream })
git-tip-push = { $count ->
    [one] Push 1 commit
   *[other] Push { $count } commits
} to { $upstream }
git-tip-pull = { $count ->
    [one] Pull 1 commit
   *[other] Pull { $count } commits
} from { $upstream }
git-tip-both = Sync with { $upstream }: pull { $behind }, push { $ahead }
git-tip-publish = Publish Branch: push to a remote and set the upstream
git-switch-dirty = Check out “{ $branch }” with uncommitted changes?
git-switch-dirty-detail = { $count ->
    [one] 1 file in “{ $current }” has uncommitted changes. Migrated changes stay in the working tree; stashed changes can be restored at any time.
   *[other] { $count } files in “{ $current }” have uncommitted changes. Migrated changes stay in the working tree; stashed changes can be restored at any time.
}
git-switch-carry = Migrate Changes
git-switch-stash = Stash and Check Out
git-switch-failed-restored = { $error } (your uncommitted changes were restored)
git-switch-failed-stashed = { $error } (your uncommitted changes are still in the stash and can be restored from the stash list)
git-switch-stash-message = Changes on { $branch } before switching branches
git-commit-button-nothing = No Changes to Commit
git-branch-picker-empty = No matching branches
git-branch-picker-recent = Recent
git-branch-picker-other = Other Local Branches
git-commit-button = Commit
git-commit-button-files = { $count ->
    [one] Commit 1 File
   *[other] Commit { $count } Files
}
git-commit-button-amend = Amend Commit
git-commit-button-committing = Committing…
git-commit-button-committed = Committed { $oid }
git-commit-button-continue-rebase = Continue Rebase
git-commit-button-finish-merge = Finish Merge
git-commit-button-finish-cherry-pick = Finish Cherry-Pick
git-commit-button-finish-revert = Finish Revert

## Log

git-log-filter-placeholder = Filter by message or hash
git-log-filter-placeholder-key = Filter by message or hash ({ $key })
git-log-filter-all = All
git-log-filter-branch = Branch
git-log-filter-author = Author
git-log-filter-file = File { $path }
git-log-clear-filter = Clear Filter
git-log-refreshing = Refreshing…
git-log-loading = Loading…
git-log-commit-count = { $count ->
    [one] { $count }{ $more } commit
   *[other] { $count }{ $more } commits
}
git-log-uncommitted = Uncommitted Changes
git-log-uncommitted-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
}
git-log-error = Couldn't Read Commits
git-log-loading-commits = Loading commits…
git-log-no-matches = No Matching Commits
git-log-no-matches-detail = Try a different branch, author, or keyword
git-log-empty = No Commits Yet
git-log-empty-detail = Commits will appear here

## Conflicts when another app modifies a file

git-disk-conflict-compare = Compare
git-disk-conflict-take-disk = Use Version on Disk
git-disk-conflict-keep-mine = Keep My Changes
git-disk-conflict-kept-mine = Kept your changes; the next save will overwrite the version on disk
git-disk-conflict-side-disk = Version on Disk
git-disk-conflict-side-unsaved = Unsaved Changes
git-disk-conflict-identical = “{ $name }” matches the version on disk; the conflict is resolved
git-disk-conflict-compare-title = Compare: { $name }
git-disk-conflict-took-disk = “{ $name }” now uses the version on disk. Press ⌘Z to restore your unsaved changes
git-disk-conflict-reloaded = Reloaded “{ $name }”; unsaved changes were discarded
git-disk-conflict-overwrite = Overwrite
git-disk-conflict-modified = “{ $name }” was modified by another app
git-disk-conflict-modified-detail = Saving will overwrite the version on disk with the contents of this window. Consider comparing the two first.
git-disk-conflict-choose-first = “{ $name }” was modified by another app. First choose which version to keep in the conflict notice at the top right

## New Branch and New Worktree dialogs

git-form-base-current-branch = { $name } (current branch)
git-form-base-current-commit = Current commit
git-form-base-commit = Commit { $commit }
git-form-base-existing-branch = Existing branch
git-form-base-current = Current branch
git-form-name-leading-dash = Branch names can't start with “-”
git-form-name-exists = A branch with this name already exists
git-form-current-branch-held = The current branch can't be checked out in another worktree
git-form-stale-worktree = Checked out by a stale worktree; prune it first
git-form-create = Create
git-form-open = Open
git-op-new-worktree = creating worktree “{ $name }”

## Commit actions in the log

git-op-checkout-commit = checking out { $commit }
git-op-cherry-pick = cherry-picking { $commit }
git-op-revert = reverting { $commit }
git-op-reset = resetting “{ $branch }” to { $commit }
git-op-reset-hard = hard resetting “{ $branch }” to { $commit }
git-op-stash-drop = dropping stash “{ $name }”
git-log-copied-oid = Copied commit hash { $commit }
git-log-copied-short-oid = Copied short commit hash { $commit }
git-log-copied-message = Copied commit message
git-log-detach-confirm = Check out commit { $commit }?
git-log-detach-confirm-detail = You'll be in a detached HEAD state, not on any branch. To keep commits made in this state, create a branch for them.
git-log-detach-confirm-button = Check Out
git-log-reset-hard-confirm = Hard reset “{ $branch }” to { $commit }?
git-log-reset-hard-confirm-detail = All uncommitted changes in the working tree and staging area will be discarded, and commits on “{ $branch }” after this commit will no longer belong to the branch. This can't be undone.
git-log-reset-hard-confirm-button = Hard Reset

## Log tab and file history

git-log-title = Git Log
git-history-title = { $name } · History
git-log-target-not-loaded = The target commit hasn't loaded yet. Scroll down the list and try again
git-log-parent-not-loaded = The parent commit hasn't loaded yet. Scroll down the list and try again
git-next-file = Switched to the next file: { $name } ({ $index }/{ $total })
git-previous-file = Switched to the previous file: { $name } ({ $index }/{ $total })
git-press-again-next-file = This is the last change. Repeat to switch to the next file: { $name }
git-press-again-previous-file = This is the first change. Repeat to switch to the previous file: { $name }
git-diff-side-commit-missing = { $commit } · File Doesn't Exist
git-diff-side-before-root = Empty · Before Root Commit

## Resolving merge conflicts

git-merge-side-current = Current
git-merge-side-incoming = Incoming
git-merge-side-base = Common Ancestor
git-merge-compare = Compare
git-merge-previous = Previous
git-merge-next = Next
git-merge-mark-resolved = Mark as Resolved
git-merge-accept-all-current = Accept All Current
git-merge-accept-all-incoming = Accept All Incoming
git-merge-card-conflicts = { $count ->
    [one] { $count } conflict
   *[other] { $count } conflicts
}
git-merge-card-resolved = All conflicts resolved
git-merge-compare-title = { $name } · Conflict { $index }
git-merge-all-resolved = All conflicts resolved; the file will be marked as resolved when saved
git-merge-picked-current = { $count ->
    [one] Accepted current; { $count } conflict left
   *[other] Accepted current; { $count } conflicts left
}
git-merge-picked-incoming = { $count ->
    [one] Accepted incoming; { $count } conflict left
   *[other] Accepted incoming; { $count } conflicts left
}
git-merge-picked-both = { $count ->
    [one] Kept both; { $count } conflict left
   *[other] Kept both; { $count } conflicts left
}
git-merge-picked-all-current = { $count ->
    [one] Accepted current in { $count } conflict
   *[other] Accepted current in all { $count } conflicts
}
git-merge-picked-all-incoming = { $count ->
    [one] Accepted incoming in { $count } conflict
   *[other] Accepted incoming in all { $count } conflicts
}
git-merge-picked-all-both = { $count ->
    [one] Kept both in { $count } conflict
   *[other] Kept both in all { $count } conflicts
}
git-merge-no-next = No next conflict
git-merge-no-previous = No previous conflict
git-merge-unresolved-left = There are still unresolved conflicts
git-op-mark-resolved = marking “{ $name }” as resolved

## Line changes in the editor

git-change-line = Line { $line }
git-change-lines = Lines { $start }–{ $end }
git-change-added = Added · { $lines }
git-change-modified = Modified · { $lines }
git-change-deleted-at-start = { $count ->
    [one] Deleted { $count } line · Start of file
   *[other] Deleted { $count } lines · Start of file
}
git-change-deleted-after = { $count ->
    [one] Deleted { $count } line · After line { $line }
   *[other] Deleted { $count } lines · After line { $line }
}
git-change-reverted = Reverted change
git-change-no-next = No next change
git-no-next-file = No next file
git-no-previous-file = No previous file
git-change-no-previous = No previous change
git-change-none-at-caret = No change on the current line

## Diff view

git-diff-side-worktree = Working Tree
git-diff-side-worktree-unsaved = Working Tree · Unsaved
git-diff-identical-sides = Both sides are identical
git-diff-none-at-caret = No change at the cursor
git-diff-revert-failed = Couldn't revert the change: { $error }

## Change popup

git-change-popup-added = { $count ->
    [one] Added { $count } line
   *[other] Added { $count } lines
}
git-change-popup-deleted = { $count ->
    [one] Deleted { $count } line
   *[other] Deleted { $count } lines
}
git-change-popup-modified = { $count ->
    [one] Modified { $count } line
   *[other] Modified { $count } lines
}
git-change-popup-not-in-head = Not in HEAD
git-change-popup-more-lines = { $count ->
    [one] { $count } more line not shown
   *[other] { $count } more lines not shown
}

## Commit view file list

git-commit-group-changes = Changes
git-commit-group-conflicts = Conflicts
git-commit-group-untracked = Untracked Files
git-commit-open-deleted = The file was deleted and can't be opened
git-commit-blocker-conflicts = { $count ->
    [one] { $count } file still has unresolved conflicts
   *[other] { $count } files still have unresolved conflicts
}
git-log-files-changed = { $count ->
    [one] { $count } file changed
   *[other] { $count } files changed
}
git-log-file-binary = Binary

## Blame

git-blame-not-in-repository = The current file isn't in a Git repository, so it can't be blamed
git-blame-failed = Couldn't blame: { $error }
git-blame-uncommitted = Uncommitted changes
git-blame-line-uncommitted = This line hasn't been committed
