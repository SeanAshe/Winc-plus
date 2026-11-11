## Settings card.

settings-language = Language
settings-language-auto = System Default
settings-title = Settings
settings-search-placeholder = Search Settings
settings-search-results = Search Results
settings-search-empty = No matching settings

## Pages and groups.

settings-page-appearance = Appearance
settings-page-editor = Editor
settings-page-format = Formatting
settings-page-git = Git
settings-page-terminal = Terminal
settings-page-modules = Modules
settings-page-keys = Shortcuts
settings-page-about = About
settings-section-format-tools = Formatters
settings-section-fix-tools = Fixers

## Setting names.

settings-theme = Mode
settings-light-theme = Light Theme
settings-dark-theme = Dark Theme
settings-ui-font = Interface Font
settings-ui-size = Interface Font Size
settings-font-weight = Global Font Weight
settings-ui-zoom = Zoom
settings-editor-font = Editor Font
settings-editor-size = Editor Font Size
settings-tab-width = Tab Width
settings-auto-save = Auto Save
settings-hot-exit = Hot Exit
settings-restore-folder = Restore Last Folder on Launch
settings-sticky-scroll = Sticky Scroll
settings-render-whitespace = Render Whitespace
settings-render-whitespace-none = None
settings-render-whitespace-selection = Selection
settings-render-whitespace-trailing = Trailing
settings-render-whitespace-all = All
settings-whole-lines = Show Long Lines in Full
settings-word-wrap = Word Wrap
settings-outline = Outline
settings-compact-folders = Compact Folders
settings-preview-on-click = Preview Tabs
settings-minimap = Minimap
settings-line-numbers = Line Numbers
settings-diff-inline = Inline Diff View
settings-status-bar = Status Bar
settings-background-image = Custom Background
settings-background-opacity = Background Opacity
settings-background-fit = Background Fit
settings-background-fit-fill = Fill
settings-background-fit-fit = Fit
settings-background-fit-stretch = Stretch
settings-background-fit-tile = Tile
settings-background-none = None
settings-background-choose = Choose Image…
settings-background-failed = Unable to load the background image: { $reason }
settings-restore-window = Remember Window Size and Position
settings-animations = Animations
settings-remote-images = Load Remote Images in Markdown Preview
settings-check-updates = Check for Updates Automatically
settings-system-title-bar = Use System Title Bar
settings-format-on-save = Format on Save
settings-fix-on-save = Fix Problems on Save
settings-skip-on-save = Excluded Languages
settings-add-language = Add Language…
settings-terminal-module = Terminal
settings-terminal-shell = Default Shell
settings-terminal-position = Position
settings-terminal-font = Font
settings-terminal-size = Font Size
settings-terminal-copy-on-select = Copy on Select
settings-terminal-confirm-close = Confirm Before Closing
settings-git-confirm-push = Confirm Before Pushing
settings-terminal-prompt = Winc Prompt
settings-terminal-shell-integration = Shell Integration
settings-terminal-option-as-meta = Meta Key
settings-key-search = Search Shortcuts

## Setting descriptions.

settings-auto-save-desc = Save changes automatically when switching tabs or apps
settings-hot-exit-desc = Quit without confirming each file, and restore unsaved changes on next launch
settings-restore-folder-desc = Remember the open folder and tabs on quit, and restore them on next launch
settings-sticky-scroll-desc = Pin the current function, class and heading to the top of the editor while scrolling
settings-render-whitespace-desc = Mark spaces and tabs with dots and arrows without changing the file
settings-whole-lines-desc = Lines over 10,000 characters are no longer cut off; the rest is not highlighted
settings-word-wrap-desc = Wrap long lines to the editor width without changing the file
settings-outline-desc = List the current file’s functions, types and headings at the bottom of the Files view
settings-compact-folders-desc = Show chains of single child folders as one row in the file tree and Git change lists
settings-preview-on-click-desc = Single-clicking a file opens it in a preview tab; double-click or edit it to keep it open
settings-minimap-desc = Show a code overview to the right of the editor; click or drag to scroll
settings-line-numbers-desc = Show line numbers to the left of the editor; diff views always show them
settings-diff-inline-desc = Show old and new content stacked in one column; when off, show them side by side
settings-status-bar-desc = Show the cursor position, encoding, Git branch, and other details at the bottom of the window
settings-background-image-desc = Setting a background increases memory use and drawing time
settings-background-opacity-desc = Higher values make the background more visible
settings-restore-window-desc = Open windows at the size and position they had when last closed
settings-animations-desc = Animate panels, popups, and list changes
settings-remote-images-desc = Load remote images such as badges and screenshots; when off, show alt text
settings-check-updates-desc = Check on launch and once a day, and show new versions in the status bar
settings-system-title-bar-desc = Use the title bar and window frame drawn by the desktop
settings-version = Version
settings-version-line = Version { $version }
settings-license = License
settings-link-open-failed = Couldn’t open link: { $error }
settings-version-copied = Copied version { $version }
settings-updates = Software Update
settings-check-now = Check Now
settings-update-download = Download
settings-update-restart = Restart
settings-update-retry = Retry
settings-updates-off = In-app updates aren’t available in this build
settings-updates-idle = Not checked yet
settings-updates-checking = Checking for updates…
settings-updates-latest = Winc is up to date
settings-updates-found = A new version is available: { $version }
settings-updates-failed = Couldn’t check for updates. Try again later.
settings-updates-downloading = Downloading { $version }: { $percent }%
settings-updates-ready = { $platform ->
    [windows] { $version } is downloaded. It installs when you restart or quit Winc.
   *[other] { $version } is installed. Restart Winc to use it.
}
settings-updates-install-failed = Couldn’t install the update: { $error }
settings-format-on-save-desc = Format files with the tools below before saving
settings-fix-on-save-desc = Fix problems that lint tools can fix automatically before saving
settings-skip-on-save-desc = Don’t fix or format files in these languages on save; running manually still works
settings-git-module-desc = Show file status and the current branch in Git repositories
settings-git-confirm-push-desc = Show the target remote before publishing, pushing, or syncing
settings-terminal-module-desc-key = Terminal panel below the editor ({ $key }); starts a helper process when opened
settings-terminal-module-desc = Terminal panel below the editor; starts a helper process when opened
settings-terminal-copy-on-select-desc = Copy selected text to the clipboard automatically
settings-terminal-confirm-close-desc = Ask before closing a tab that’s still running a program
settings-terminal-prompt-desc = Replace the default prompt with the folder, Git branch and exit status
settings-terminal-shell-integration-desc = { $platform ->
    [windows] Split WSL zsh, bash and fish output into command blocks to fold and jump between
   *[other] Split zsh, bash and fish output into command blocks to fold and jump between
}
settings-terminal-option-as-meta-desc = Send an Esc prefix instead of typing special characters

## Setting values and hints.

settings-theme-auto = Auto
settings-theme-light = Light
settings-theme-dark = Dark
settings-skip-on-save-none = None
settings-list-separator = { ", " }
settings-font-default = System Default
settings-terminal-shell-default = System Default
settings-terminal-shell-custom = Custom Command…
settings-terminal-shell-placeholder = Enter a command; quote paths that contain spaces
settings-terminal-position-bottom = Bottom
settings-terminal-position-right = Right
settings-terminal-font-default = Match Editor
settings-terminal-option-as-meta-left = Left ⌥
settings-terminal-option-as-meta-right = Right ⌥
settings-terminal-option-as-meta-both = Both ⌥
settings-terminal-option-as-meta-none = None
settings-command-placeholder = Enter a command; { "{file}" } stands for the file path
settings-command-edit-hint = Press { common-key-enter } to save, Esc to cancel
settings-font-not-installed = Font “{ $name }” is not installed
settings-font-file-not-found = Font file “{ $name }” not found
settings-font-unsupported = The glyph format of font “{ $name }” is not supported
settings-theme-used-when-dark = This theme is used when the system appearance is dark
settings-theme-used-when-light = This theme is used when the system appearance is light

## Theme names.

settings-theme-name-winc-day = Winc Daylight
settings-theme-name-winc-paper = Winc Rice Paper
settings-theme-name-sky = Clear Sky
settings-theme-name-paper = Plain Paper
settings-theme-name-vermilion = Vermilion
settings-theme-name-mist = Morning Mist
settings-theme-name-veil-snow = Snow Veil
settings-theme-name-veil-linen = Linen Veil
settings-theme-name-winc-night = Winc Night Flight
settings-theme-name-winc-slate = Winc Distant Hills
settings-theme-name-ink = Ink Blue
settings-theme-name-ember = Ember
settings-theme-name-midnight = Midnight
settings-theme-name-dusk = Dusk
settings-theme-name-veil-night = Night Veil
settings-theme-name-veil-umber = Umber Veil

## Shortcuts page.

settings-keys-recording = Press the shortcut to look up; press Esc to stop
settings-keys-search-record = Type a command name to search, or press { $key } to record keys
settings-keys-search = Type a command name to search
settings-keys-no-command = No command uses this shortcut
settings-keys-no-match = No matching commands

## Font size and the configuration file.

settings-font-size-limit = Editor font size is already { $size } and can’t be adjusted further
settings-font-size = Editor font size { $size }
settings-terminal-font-size-limit = Terminal font size is already { $size } and can’t be adjusted further
settings-terminal-font-size = Terminal font size { $size }
settings-zoom-limit = Zoom is already { $zoom }% and can’t be adjusted further
settings-zoom = Zoom { $zoom }%
settings-config-backed-up = The configuration file couldn’t be parsed; it was saved as { $path } and the new settings were written
settings-config-comments-backed-up = Comments in the configuration file can’t be kept when writing settings; the original was saved as { $path }
settings-config-write-failed = Couldn’t write the configuration file: { $error }
