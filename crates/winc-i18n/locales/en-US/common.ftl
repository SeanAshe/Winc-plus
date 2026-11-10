## Text shared across areas: common buttons and options.

common-cancel = Cancel
common-dont-ask = Don’t ask again
common-open = Open
common-save = Save
common-save-as-label = Save As:
common-list-separator = {", "}
common-reveal-in-finder = Reveal in Finder
common-reveal-in-folder = { $platform ->
    [windows] Reveal in File Explorer
   *[other] Show in Folder
}
common-key-enter = { $platform ->
    [macos] ↩
   *[other] Enter
}
common-close = Close
common-close-dialog-first = A dialog is already open in this window; close it first

## Move to Trash

common-trash = { $platform ->
    [windows] Recycle Bin
   *[other] Trash
}
common-trash-confirm-folder = Move the folder “{ $name }” and its contents to the { common-trash }?
common-trash-confirm-item = Move “{ $name }” to the { common-trash }?
common-trash-confirm-items = { $count ->
        [one] Move this item to the { common-trash }?
       *[other] Move these { $count } items to the { common-trash }?
    }
common-trash-confirm-detail = You can restore items from the { common-trash }.
common-trash-move = Move to { common-trash }
common-trash-moved-item = Moved to { common-trash }: { $name }
common-trash-moved-items = { $count ->
        [one] Moved { $count } item to the { common-trash }
       *[other] Moved { $count } items to the { common-trash }
    }
common-trash-delete-failed = Couldn’t delete “{ $name }”: { $error }
common-trash-invalid-path = Invalid path
common-trash-no-file-manager = NSFileManager is unavailable
common-trash-failed = Couldn’t move to the { common-trash }
common-trash-failed-code = Couldn’t move to the { common-trash } (error { $code })
common-trash-cancelled = Cancelled; nothing was moved to the { common-trash }
common-trash-unsupported = Moving to the Recycle Bin isn’t supported on this platform

## Opening links (terminal, Markdown preview)
common-link-scheme-not-allowed = Only http, https, and mailto links are supported

## New Worktree and New Branch dialogs

common-dialog-new-worktree = New Worktree
common-dialog-new-branch = New Branch
common-dialog-branch = Branch
common-dialog-name = Name
common-dialog-base = Based On
common-dialog-location = Location
common-dialog-worktree-placeholder = New or existing branch name
common-dialog-branch-placeholder = New branch name
common-dialog-branch-name = branch-name
common-dialog-copy-ignored = Copy Ignored Files
common-dialog-open-in-new-window = Open in New Window
common-dialog-checkout-after-create = Check Out After Creating
common-dialog-creating = Creating…

## Move to the Applications folder

app-move-title = Move Winc to the Applications folder?
app-move-detail = Winc is running directly from “{ $folder }” and can’t update itself. After it moves to the Applications folder, the original is moved to the Trash and only this copy of Winc stays on your Mac.
app-move-detail-image = Winc is running from a disk image and can’t update itself. After it moves to the Applications folder, the disk image is ejected and only this copy of Winc stays on your Mac.
app-move-button = Move to Applications Folder
app-move-stay = Don’t Move
app-move-failed = Couldn’t move to the Applications folder: { $error }
app-move-target-running = The copy of Winc in the Applications folder is running. Quit it first
app-move-trash-failed = Couldn’t move “{ $path }” to the Trash
app-move-copy-failed = Copying failed ({ $status })
app-move-relaunch-failed = Couldn’t relaunch Winc: { $error }
app-move-unknown-location = Couldn’t determine where Winc is
