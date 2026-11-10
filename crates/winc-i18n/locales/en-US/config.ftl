## Problems found while reading the configuration file.

config-read-failed = Couldn’t read the configuration file ({ $error }); using default settings
config-invalid-json = The configuration file isn’t valid JSON; using default settings
config-root-not-object = The top level of the configuration file should be an object; using default settings
config-unknown-key = Unknown setting { $key }; ignored
config-project-read-failed = Couldn’t read ({ $error }); ignored
config-project-invalid-json = Not a valid JSON object; ignored
config-project-unsupported-key = Project settings support only format; ignored { $key }

## 运行配置（.winc/run.json）。

config-run-read-failed = Couldn’t read ({ $error }); ignored
config-run-invalid-json = Invalid file format: expected a JSON object with a configurations array; ignored
config-run-unsupported-key = Unsupported field { $key }; ignored
config-run-invalid-entry = Entry { $index } is invalid and was ignored: name and command must be non-empty strings, args an array of strings, and env an object of strings
config-run-duplicate-name = Duplicate configuration name “{ $name }”; later entries were ignored
config-run-template-note = Each entry requires name and command; args, cwd, and env are optional.

## Problems with setting values.

config-theme-invalid = { $name } must be auto, dark, or light; ignored
config-render-whitespace-invalid = { $name } must be none, selection, trailing, or all; ignored
config-background-fit-invalid = { $name } must be fill, fit, stretch, or tile; ignored
config-language-invalid = { $name } must be auto, zh-CN, or en-US; ignored
config-not-integer = { $name } should be an integer (currently { $value }); ignored
config-not-string = { $name } should be a string; ignored
config-terminal-position-invalid = { $name } should be "bottom" or "right"; ignored
config-title-bar-style-invalid = { $name } should be "custom" or "native"; ignored
config-option-as-meta-invalid = { $name } should be "left", "right", "both", or "none"; ignored
config-not-array = { $name } should be an array; ignored
config-item-not-non-empty-string = Items in { $name } should be non-empty strings; ignored
config-item-not-string = Items in { $name } should be strings; ignored
config-not-object = { $name } should be an object; ignored
config-duplicate-language = “{ $language }” in { $name } refers to the same language as another entry; using “{ $language }”
config-unknown-language = Unknown language “{ $language }” in { $name }; ignored
config-empty = { $name } is empty; ignored
config-not-bool = { $name } should be true or false; ignored
config-not-number = { $name } should be a number; ignored
config-out-of-range = { $setting } is outside the { $label } range ({ $range }); ignored
config-label-font-size = font size
config-label-font-weight = font weight
config-label-background-opacity = background opacity
config-label-zoom = zoom
config-label-tab-width = tab width
config-label-repository-scan-depth = repository scan depth
config-label-lines = line count
config-label-contrast = contrast
