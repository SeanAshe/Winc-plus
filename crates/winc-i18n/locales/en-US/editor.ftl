## Editor placeholders and welcome page.

editor-preview-failed = Couldn’t show the preview: { $reason }
editor-show-source = Show Source
editor-reading = Reading “{ $name }”…
editor-open-anyway = Open Anyway
editor-open-with-default-app = Open with Default App
editor-retry = Retry
editor-welcome-open = Open File or Folder
editor-welcome-open-file = Open File
editor-welcome-open-folder = Open Folder
editor-welcome-new-file = New File
editor-welcome-go-to-file = Go to File
editor-welcome-find-in-files = Find in Files
editor-welcome-toggle-sidebar = Show or Hide Sidebar
editor-welcome-settings = Settings
editor-line-truncated = … Line too long; showing the first { $count } characters
editor-line-show-whole = Show Whole Line
editor-blame-uncommitted = Uncommitted

## Find, replace, and multiple cursors.

editor-no-other-match = No other matches
editor-no-match = No matches
editor-too-many-matches = Too many matches; selected only the first { $count }
editor-too-many-cursors = Too many cursors; kept only { $count }
editor-find-large-filter = The file is too large to show only matching lines
editor-replace-none = No matches to replace
editor-replaced = Replaced { $count ->
        [one] { $count } occurrence
       *[other] { $count } occurrences
    }
editor-find-large-multiline = Multiline search isn’t supported in large files
editor-replace-all-pending = Search isn’t finished yet; try Replace All again shortly
editor-replace-all-too-many = More than 10,000 matches; can’t replace all
editor-replace-all-gave-up = The regular expression backtracked too much on some lines, so the results are incomplete; can’t replace all
editor-replace-too-long = The matched text is too long to replace
editor-large-no-multi-cursor = The file is too large for multi-cursor search

## Word wrap and folding.

editor-word-wrap-on-large = File too large to wrap
editor-word-wrap-on-pending = Word wrap on (large file mode)
editor-word-wrap-on-table = Column-aligned tables can’t be wrapped
editor-word-wrap-on = Word wrap on
editor-word-wrap-off = Word wrap off
editor-fold-large = The file is too large to fold
editor-large-whole-file = The file is too large for this operation on the whole file

## Bookmarks, symbols, definitions, and usages.

editor-bookmark-added = Bookmark added
editor-bookmark-removed = Bookmark removed
editor-bookmark-none = No bookmarks in this file
editor-bookmark-cleared = Cleared { $count ->
        [one] { $count } bookmark
       *[other] { $count } bookmarks
    }
editor-no-open-file = No open file
editor-symbol-function = Function
editor-symbol-method = Method
editor-symbol-type = Type
editor-symbol-module = Module
editor-symbol-variable = Variable
editor-symbol-heading = Heading
editor-symbol-section = Section
editor-definition-no-name = No name to look up at the cursor
editor-usages-not-found = No other usages of “{ $name }” found
editor-definition-at-caret = The cursor is already at the definition of “{ $name }”
editor-definition-not-found = No definition of “{ $name }” found
editor-open-folder-first = Open a folder first
editor-places-definitions-count = { $count ->
        [one] { $count } definition
       *[other] { $count } definitions
    }
editor-places-usages-count = { $count ->
        [one] { $count } usage
       *[other] { $count } usages
    }
editor-places-bookmarks-count = { $count ->
        [one] { $count } bookmark
       *[other] { $count } bookmarks
    }
editor-places-no-match = No matching locations
editor-places-usages-suffix = { " " }usages
editor-places-definitions-suffix = { " " }definitions
editor-places-bookmarks = Bookmarks

## Diff view.

editor-diff-identical = No Differences
editor-diff-unchanged-lines = { $count ->
        [one] { $count } unchanged line
       *[other] { $count } unchanged lines
    }
editor-diff-empty-file = Empty File

## Encodings.

editor-encoding-gb18030 = Simplified Chinese (GB18030)
editor-encoding-gbk = Simplified Chinese (GBK)
editor-encoding-big5 = Traditional Chinese (Big5)
editor-encoding-shift-jis = Japanese (Shift JIS)
editor-encoding-euc-jp = Japanese (EUC-JP)
editor-encoding-euc-kr = Korean (EUC-KR)
editor-encoding-windows-1252 = Western (Windows 1252)
editor-encoding-mac-roman = Western (Mac Roman)
editor-encoding-windows-1250 = Central European (Windows 1250)
editor-encoding-windows-1251 = Cyrillic (Windows 1251)
editor-encoding-koi8-r = Cyrillic (KOI8-R)
editor-encoding-windows-1253 = Greek (Windows 1253)
editor-encoding-windows-1254 = Turkish (Windows 1254)
editor-encoding-windows-1255 = Hebrew (Windows 1255)
editor-encoding-windows-1256 = Arabic (Windows 1256)
editor-encoding-windows-1257 = Baltic (Windows 1257)
editor-encoding-windows-1258 = Vietnamese (Windows 1258)
editor-encoding-windows-874 = Thai (Windows 874)
editor-encoding-lossy = Non-UTF-8
editor-encoding-binary = Binary
editor-encoding-unencodable = “{ $char }” can’t be saved in { $encoding }; use “Save with Encoding” to save as UTF-8

## Reading and writing files.

editor-file-no-path = The file has no path yet
editor-original-changed = The file on disk was changed by another program after it was opened; can’t save
editor-large-identity = The file has multiple hard links or belongs to another user, and saving a large file replaces it, which would break the links or change the owner; can’t save
editor-moved-while-saving = The file was moved while it was being saved
editor-large-cannot-compare = The file is too large to compare
editor-large-utf8-only = The file is too large; it can only be opened as UTF-8
editor-file-is-binary = This is a binary file
editor-file-read-only = The file is read-only
editor-temp-file-failed = Couldn’t create a temporary file
editor-large-original-truncated = Reading the original file failed or it was truncated; the content is incomplete
editor-paste-cannot-copy = Can’t copy this item
editor-paste-into-itself = Can’t copy an item into itself
editor-open-binary = This file is binary and can’t be opened
editor-open-too-large = The file is too large ({ $size } MB); the limit is { $limit } GB
editor-unreadable-not-found = The file doesn’t exist
editor-unreadable-permission = No permission to read the file
editor-unreadable-failed = Reading failed ({ $error })
editor-unsupported-image = Image
editor-unsupported-workbook = Workbook
editor-unsupported-docx = Word document
editor-unsupported-pdf = PDF document
editor-unsupported-binary = Binary file or unsupported text encoding ({ $size }); can’t show a preview
editor-unsupported-too-large = The file is too large ({ $size } GB, limit { $limit } GB) to show a preview
editor-unsupported-unreadable = { $reason }; can’t show a preview
editor-size-bytes = { $count ->
        [one] { $count } byte
       *[other] { $count } bytes
    }

## External programs.

editor-process-timeout = Timed out
editor-process-too-large = Too much output
editor-process-cancelled = Cancelled
editor-process-no-stdout = Couldn’t get standard output

## Why a document is read-only or can’t be edited.

editor-readonly-pending = The file hasn’t been read yet
editor-readonly-unsupported = The file can’t be previewed or edited
editor-readonly-previewing = Previewing; switch to the source to edit
editor-readonly-tab = This tab is read-only
editor-readonly-binary = Binary file; opened as read-only
editor-readonly-long-lines = The file has lines longer than 16 MB; opened as read-only
editor-readonly-not-utf8 = The file isn’t UTF-8 encoded; opened as read-only
editor-selection-too-large-to-edit = The selection is larger than 16 MB and can’t be edited
editor-vim-unsupported = This command isn’t supported in Vim mode yet
editor-vim-too-large = Too much text involved; nothing was done
editor-vim-register-empty = Register { $name } is empty
editor-vim-register-read-only = Register { $name } can’t be written
editor-vim-not-found = Pattern not found
editor-vim-bad-pattern = Unsupported item at character { $column } of the pattern
editor-vim-no-search = No previous search
editor-vim-no-mark = Mark { $name } is not set
editor-vim-bad-command = Not an editor command
editor-vim-bad-argument = Invalid argument
editor-vim-bad-range = Invalid range
editor-vim-no-substitute = No previous substitute
editor-vim-no-command = No previous command line
editor-vim-interrupted = Stopped: too many steps or too much time
editor-vim-unknown-option = Unknown option { $name }
editor-vim-substituted = { $count } substitutions
editor-vim-substituted-lines = { $count } substitutions on { $lines } lines
editor-vim-matched = { $count } matches on { $lines } lines
editor-vim-more-lines = { $count } more lines
editor-vim-fewer-lines = { $count } fewer lines
editor-vim-no-buffer = No such document
editor-vim-no-file = No file at that path under the cursor
editor-vim-file-exists = File exists; add ! to overwrite
editor-vim-written = Wrote { $name }
editor-vim-diff-read-only = This side of the diff is read-only
editor-vim-help-title = Vim Mode: Commands
editor-vim-help-modes = Modes: N normal, V visual, O operator-pending, I insert
editor-vim-help-keys = Keys
editor-vim-help-mode = Modes
editor-vim-help-kind = Kind
editor-vim-help-status = Status
editor-vim-help-motion = motion
editor-vim-help-operator = operator
editor-vim-help-object = text object
editor-vim-help-command = command
editor-vim-help-prefix = prefix
editor-vim-help-supported = supported
editor-vim-help-unsupported = unsupported
editor-vim-help-ex = Command-line commands (the bracketed part is optional)
editor-vim-help-options = Options for :set (abbreviation in parentheses)

## Copy and paste.

editor-selection-copied = Copied selection
editor-selection-too-large-to-copy = The selection is larger than { $size } MB and can’t be copied
editor-paste-copied-item = Copied “{ $name }”
editor-paste-copied-items = { $count ->
        [one] Copied { $count } item
       *[other] Copied { $count } items
    }
editor-paste-cut-item = Cut “{ $name }”
editor-paste-cut-items = { $count ->
        [one] Cut { $count } item
       *[other] Cut { $count } items
    }
editor-paste-copy-failed = Couldn’t copy “{ $name }”: { $error }
editor-paste-failed = Couldn’t paste “{ $name }”: { $error }
editor-paste-image-save-first = Save this file first; the image will be saved in its folder
editor-paste-image-markdown-only = Images can only be pasted into Markdown files or the file tree
editor-paste-image-save-failed = Couldn’t save the image: { $error }
editor-paste-clipboard-empty = The clipboard is empty
editor-paste-copy-name = { $stem } copy{ $ext }
editor-paste-copy-name-numbered = { $stem } copy { $number }{ $ext }
