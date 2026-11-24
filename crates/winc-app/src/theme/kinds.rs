//! 按文件名给出文件图标的记号和颜色。
//!
//! [`file_kind`] 返回记号和一对颜色，由 [`Palette::file_icon`] 按当前配色深浅选其中一个；
//! 记号的形状见 `crate::icons::FileGlyph`。

use super::*;
use FileGlyph::*;

// 图标配色，每种一对：深色配色用的亮色、浅色配色用的暗色，均为 `0xRRGGBB`。
const GREY: [u32; 2] = [0x9AA0AA, 0x5F6570];
const DIM: [u32; 2] = [0x80858F, 0x8A8F99];
const SHELL: [u32; 2] = [0x8CCB84, 0x3D7F35];
const YELLOW: [u32; 2] = [0xE0BD6A, 0x93700F];
const ORANGE: [u32; 2] = [0xE68C66, 0xB04F24];
const BLUE: [u32; 2] = [0x72A8E0, 0x2F68A8];
const PURPLE: [u32; 2] = [0xB38BE6, 0x7246B5];
const RED: [u32; 2] = [0xE57373, 0xB23A3A];
const GREEN: [u32; 2] = [0x7CC38A, 0x2E8540];
const TEAL: [u32; 2] = [0x5CC6B8, 0x16806F];
const PINK: [u32; 2] = [0xE38AC0, 0xA83E7F];
const DOC: [u32; 2] = [0x8FB3D9, 0x44709E];
const DOCKER: [u32; 2] = [0x5AA9E6, 0x1D63B8];
const GIT: [u32; 2] = [0xF0806A, 0xC4442E];
const TOML: [u32; 2] = [0xD89A6A, 0x9A5A25];
const NPM: [u32; 2] = [0xE57373, 0xC03A3A];
const RUST: [u32; 2] = [0xE79A6C, 0xB4551E];
const GO: [u32; 2] = [0x6CC4DC, 0x1C7F99];
const REACT: [u32; 2] = [0x61D0EC, 0x0E8AA8];
const PYTHON: [u32; 2] = [0x6CA6E0, 0x2F6FB0];
const SWIFT: [u32; 2] = [0xF0845E, 0xC4441E];
const DART: [u32; 2] = [0x5CB8E6, 0x1A78A8];
const JULIA: [u32; 2] = [0x7FA8D8, 0x3B6AA0];
const C: [u32; 2] = [0x8FA6D6, 0x4A65A0];
const DOTNET: [u32; 2] = [0x9B8BE0, 0x5A47B0];
/// 项目文件的颜色：Winc 的蓝色。
const PROJECT: [u32; 2] = [0x6FA3FF, 0x1769FC];
/// 未识别的类型的颜色。
const PLAIN: [u32; 2] = [0x9AA0AA, 0x6B707A];

/// 返回文件名对应的类型记号和颜色（深色、浅色配色各一个，均为 `0xRRGGBB`）。
///
/// 先按整个文件名识别（如 `Dockerfile`、`go.mod`、`.gitignore`），再按扩展名，再按编辑器的
/// 语言表；许可证、README 这类按名字认的文档只在扩展名说明不了是什么时才认。都不匹配时返回
/// `Plain`。
pub fn file_kind(name: &str) -> (FileGlyph, [u32; 2]) {
    // 每帧会对每个可见的文件名调用一次；名字已是小写时借用原字符串，不分配内存。
    let lowered: Cow<'_, str> = if name.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    };
    let lower: &str = &lowered;
    // 名字里的前半截（去掉扩展名）：`README.md`、`LICENSE.txt`、`docker-compose.yml` 按它认。
    let stem = lower.split('.').next().unwrap_or(lower);
    let whole = match lower {
        // 容器：Dockerfile、compose 文件和 .dockerignore 都用容器图标。
        "dockerfile" | "containerfile" | ".dockerignore" => Some((Docker, DOCKER)),
        _ if lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile") => {
            Some((Docker, DOCKER))
        }
        _ if (stem == "docker-compose" || stem == "compose")
            && (lower.ends_with(".yml") || lower.ends_with(".yaml")) =>
        {
            Some((Docker, DOCKER))
        }
        // Git 自己的文件。
        ".gitignore"
        | ".gitattributes"
        | ".gitmodules"
        | ".gitkeep"
        | ".gitconfig"
        | ".mailmap"
        | ".git-blame-ignore-revs" => Some((Git, GIT)),
        // 锁文件按整个名字先认，不当成 JSON、YAML。
        "package-lock.json"
        | "npm-shrinkwrap.json"
        | "pnpm-lock.yaml"
        | "yarn.lock"
        | "bun.lockb"
        | "bun.lock"
        | "deno.lock"
        | "cargo.lock"
        | "poetry.lock"
        | "pdm.lock"
        | "pipfile.lock"
        | "pixi.lock"
        | "composer.lock"
        | "gemfile.lock"
        | "podfile.lock"
        | "package.resolved"
        | "cartfile.resolved"
        | "pubspec.lock"
        | "mix.lock"
        | "rebar.lock"
        | "shard.lock"
        | "go.sum"
        | "go.work.sum"
        | "flake.lock"
        | "uv.lock"
        | "gradle.lockfile"
        | "packages.lock.json"
        | "conan.lock"
        | "manifest.toml"
        | ".terraform.lock.hcl" => Some((Lock, DIM)),
        // 包的清单：按生态配色。
        "package.json" => Some((Package, NPM)),
        "deno.json" | "deno.jsonc" => Some((Package, GREY)),
        "cargo.toml" => Some((Package, RUST)),
        "go.mod" | "go.work" => Some((Package, GO)),
        "pyproject.toml" | "setup.py" | "setup.cfg" | "pipfile" | "constraints.txt" => {
            Some((Package, PYTHON))
        }
        // `requirements.txt`、`requirements-dev.txt`、`requirements.in`。
        _ if stem.starts_with("requirements")
            && (lower.ends_with(".txt") || lower.ends_with(".in")) =>
        {
            Some((Package, PYTHON))
        }
        "composer.json" => Some((Package, PURPLE)),
        "gemfile" | "podfile" => Some((Package, RED)),
        "package.swift" | "cartfile" => Some((Package, SWIFT)),
        "pubspec.yaml" => Some((Package, DART)),
        "mix.exs" | "rebar.config" | "dune-project" => Some((Package, PURPLE)),
        "build.sbt" => Some((Package, PURPLE)),
        "project.toml" => Some((Package, JULIA)),
        "build.zig.zon" | "shard.yml" => Some((Package, YELLOW)),
        "conanfile.txt" | "conanfile.py" | "vcpkg.json" => Some((Package, C)),
        "build.gradle"
        | "build.gradle.kts"
        | "settings.gradle"
        | "settings.gradle.kts"
        | "pom.xml" => Some((Package, PURPLE)),
        // 构建脚本：与 Makefile 同一个提示符。
        "makefile" | "gnumakefile" | "justfile" | "rakefile" | "vagrantfile" | "brewfile"
        | "procfile" | "jenkinsfile" | "cmakelists.txt" | "meson.build" | "meson_options.txt"
        | "build" | "build.bazel" | "workspace" | "workspace.bazel" | "module.bazel" | "buck"
        | "sconstruct" | "sconscript" | "dune" | "configure" | "gradlew" | "mvnw" | "tiltfile"
        | "earthfile" => Some((Shell, SHELL)),
        // 指定工具版本的文件。
        ".python-version" | ".node-version" | ".ruby-version" | ".java-version" | ".go-version"
        | ".terraform-version" | ".tool-versions" | "rust-toolchain" => Some((Config, GREY)),
        "authors" | "changelog" | "contributing" | "changes" | "history" => Some((Text, DOC)),
        "codeowners" | "owners" => Some((Data, GREY)),
        _ if lower.starts_with(".env.") => Some((Key, YELLOW)),
        _ => None,
    };
    if let Some(kind) = whole {
        return kind;
    }

    // 以点开头且之后没有点的名字（`.gitignore`、`.bashrc`），把点之后的整个名字当作扩展名。
    let ext = match lower.strip_prefix('.') {
        Some(rest) if !rest.contains('.') => rest,
        _ => lower.rsplit_once('.').map_or(lower, |(_, e)| e),
    };
    let known = by_extension(ext).or_else(|| by_language(name));
    // 许可证、README 这类按名字认的文档：扩展名说明不了是什么时才按名字认，`copying.rs` 仍是代码。
    let documentish = known.is_none_or(|(glyph, _)| matches!(glyph, Text | Markdown));
    if documentish && let Some(kind) = document_kind(stem) {
        return kind;
    }
    known.unwrap_or((Plain, PLAIN))
}

/// 按名字认的文档：许可证（`LICENSE`、`LICENSE-MIT`、`COPYING`）是徽章，NOTICE 是普通的文字
/// 文档，README（`README-zh` 等）是说明。`stem` 是小写的、去掉扩展名的名字。
fn document_kind(stem: &str) -> Option<(FileGlyph, [u32; 2])> {
    if stem.starts_with("license") || stem.starts_with("licence") || stem == "copying" {
        Some((License, YELLOW))
    } else if stem == "notice" {
        Some((Text, DOC))
    } else if stem.starts_with("readme") {
        Some((Info, DOC))
    } else {
        None
    }
}

/// 返回语言 `language` 的类型记号和颜色：这种语言的文件用的那一个，纯文本用文本文件的。
pub fn language_kind(language: &winc_syntax::Language) -> (FileGlyph, [u32; 2]) {
    (language.file_name("a")).map_or_else(|| file_kind("a.txt"), |name| file_kind(&name))
}

/// 扩展名表认不出、编辑器能高亮的文件：按该语言的主扩展名配图标，没有时画 `Code`。编辑器新加
/// 的语言不必在扩展名表中再登记一次。
fn by_language(name: &str) -> Option<(FileGlyph, [u32; 2])> {
    let language = winc_syntax::detect(name);
    let main = language
        .extensions
        .first()
        .and_then(|ext| by_extension(ext));
    (language.id != winc_syntax::detect("").id).then(|| main.unwrap_or((Code, GREY)))
}

/// 按小写的扩展名 `ext` 给出类型记号和颜色；认不出时返回 `None`。
fn by_extension(ext: &str) -> Option<(FileGlyph, [u32; 2])> {
    Some(match ext {
        // 代码，按语言配色。
        // 常见语言画缩写；JSX、TSX 与 JS、TS 用同一个缩写，改用青绿色与纯 JS、TS 区分。
        "py" | "pyi" | "pyw" | "pyx" | "ipynb" => (Letters("PY"), PYTHON),
        "rs" => (Letters("RS"), RUST),
        "js" | "mjs" | "cjs" => (Letters("JS"), [0xE6C35C, 0x9A7A0E]),
        "jsx" => (Letters("JS"), REACT),
        "ts" | "mts" | "cts" => (Letters("TS"), [0x6C9FE0, 0x2D62B0]),
        "tsx" => (Letters("TS"), REACT),
        "ets" => (Letters("ETS"), [0x6C9FE0, 0x2D62B0]),
        "wxs" => (Code, [0xE6C35C, 0x9A7A0E]),
        "go" => (Letters("GO"), GO),
        "java" => (Java, [0xE08C5A, 0xA9521D]),
        "kt" | "kts" => (Letters("KT"), PURPLE),
        "scala" | "sc" | "groovy" | "gradle" => (Code, PURPLE),
        "swift" => (Letters("SW"), SWIFT),
        "c" | "h" => (Letters("C"), C),
        "cpp" | "cc" | "hpp" | "cxx" | "hh" | "hxx" | "ino" | "cu" => (Letters("C++"), C),
        "m" | "mm" => (Code, C),
        "cs" | "csx" => (Letters("C#"), DOTNET),
        "fs" | "fsx" | "vb" => (Code, DOTNET),
        "html" | "htm" | "xhtml" | "vue" | "svelte" | "astro" | "wxml" => (Code, ORANGE),
        // 生成 HTML 的模板。
        "hbs" | "handlebars" | "mustache" | "ejs" | "pug" | "jade" | "njk" | "nunjucks"
        | "liquid" | "twig" | "j2" | "jinja" | "jinja2" | "haml" | "slim" | "tmpl" | "tpl"
        | "gohtml" | "cshtml" | "razor" => (Code, ORANGE),
        "xml" | "xsd" | "xsl" | "xslt" | "plist" | "props" | "targets" | "xib" | "storyboard"
        | "wsdl" | "xaml" | "rss" | "atom" | "kml" | "gpx" | "manifest" | "xcscheme"
        | "xcworkspacedata" => (Code, ORANGE),
        // 各生态中写成文件扩展名的项目与包的清单。
        "sln" | "csproj" | "fsproj" | "vbproj" | "nuspec" => (Package, DOTNET),
        "pbxproj" => (Package, SWIFT),
        "podspec" => (Package, RED),
        "cabal" | "opam" | "nimble" => (Package, PURPLE),
        "css" | "scss" | "less" | "sass" | "styl" | "pcss" | "wxss" => (Code, BLUE),
        "sql" | "psql" | "mysql" | "pgsql" => (Database, YELLOW),
        "prisma" | "graphql" | "gql" => (Code, PINK),
        "rb" | "erb" | "rake" | "gemspec" => (Letters("RB"), RED),
        "php" | "phtml" => (Code, [0x8F9BD8, 0x4D5AA0]),
        "lua" | "luau" => (Code, [0x6C8FE0, 0x2A4FB0]),
        "dart" => (Code, DART),
        "r" | "rmd" | "jl" => (Code, JULIA),
        "ex" | "exs" | "heex" | "erl" | "hrl" | "gleam" => (Code, PURPLE),
        "hs" | "lhs" | "elm" | "purs" | "ml" | "mli" | "re" | "res" => (Code, PURPLE),
        "clj" | "cljs" | "cljc" | "edn" | "lisp" | "el" | "scm" | "rkt" => (Code, GREEN),
        "zig" | "nim" | "v" | "odin" | "d" => (Code, YELLOW),
        "pl" | "pm" | "tcl" | "awk" | "sed" => (Code, BLUE),
        "proto" | "thrift" | "capnp" | "fbs" => (Code, TEAL),
        "tf" | "tfvars" | "hcl" | "nix" | "dhall" | "bicep" => (Code, PURPLE),
        "vim" | "vimrc" => (Code, GREEN),
        "wgsl" | "glsl" | "hlsl" | "metal" | "frag" | "vert" | "shader" => (Code, TEAL),
        "asm" | "s" => (Code, GREY),
        "sol" | "move" | "cairo" => (Code, GREY),
        "f" | "for" | "f77" | "f90" | "f95" | "f03" | "f08" | "cob" | "cbl" | "cpy" => (Code, BLUE),
        "pas" | "pp" | "dpr" | "lpr" | "cr" | "coffee" | "litcoffee" => (Code, YELLOW),
        "ada" | "adb" | "ads" => (Code, GREEN),
        "hx" => (Code, ORANGE),
        "gd" | "gdshader" | "sv" | "svh" | "vhd" | "vhdl" => (Code, TEAL),
        // 用文字写的图：Mermaid、PlantUML、Graphviz、D2。
        "mmd" | "mermaid" | "puml" | "plantuml" | "dot" | "gv" | "d2" => (Code, TEAL),

        // 脚本。
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "bashrc" | "zshrc" | "profile"
        | "bash_profile" | "zprofile" | "envrc" | "mk" | "cmake" | "just" | "nu" => (Shell, SHELL),
        "ps1" | "psm1" | "psd1" | "bat" | "cmd" | "vbs" | "vbe" | "wsf" => (Shell, BLUE),
        "applescript" | "scpt" | "ahk" => (Shell, GREY),

        // 项目文件：Winc 及其他编辑器。
        "winc-project" | "code-workspace" | "sublime-project" | "ipr" => (Project, PROJECT),
        // 配置和数据。
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" | "topojson" | "jsonld"
        | "har" | "webmanifest" => (Json, YELLOW),
        // 配置文件都是齿轮，按格式配色：先凭形状认出是配置文件，再凭颜色分辨具体格式。
        "yaml" | "yml" => (Config, RED),
        "toml" => (Config, TOML),
        "ini" | "cfg" | "conf" | "config" | "properties" | "editorconfig" | "npmrc" | "yarnrc"
        | "nvmrc" | "prettierrc" | "eslintrc" | "babelrc" | "browserslistrc" | "stylelintrc"
        | "reg" | "desktop" | "service" | "xcconfig" | "cnf" | "htaccess" | "entitlements" => {
            (Config, GREY)
        }
        "env" => (Key, YELLOW),
        // 各种 `.xxxignore`：一行一个要忽略的路径。
        "keep" => (Data, DIM),
        _ if ext.ends_with("ignore") => (Data, DIM),
        "csv" | "tsv" | "psv" => (Table, GREEN),
        "xls" | "xlsx" | "numbers" | "ods" => (Table, GREEN),
        "sqlite" | "sqlite3" | "db" | "mdb" | "duckdb" => (Database, YELLOW),
        "parquet" | "arrow" | "feather" | "avro" | "orc" | "h5" | "hdf5" | "npy" | "npz"
        | "pkl" | "pickle" => (Binary, YELLOW),

        // 文档。
        "md" | "markdown" | "mdx" | "qmd" => (Markdown, DOC),
        "tex" | "ltx" | "sty" | "cls" | "dtx" => (Letters("TEX"), GREEN),
        "rst" | "adoc" | "asciidoc" | "org" | "bib" | "typ" => (Text, DOC),
        "txt" | "text" | "log" | "out" | "rtf" | "nfo" => (Text, GREY),
        "pdf" => (Text, RED),
        "doc" | "docx" | "pages" | "odt" | "epub" => (Text, BLUE),
        "ppt" | "pptx" | "odp" => (Text, ORANGE),
        "diff" | "patch" => (Text, GREEN),

        // 图片。
        "png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp" | "bmp" | "dib"
        | "ico" | "cur" | "icns" | "heic" | "heif" | "avif" | "tif" | "tiff" | "psd" | "raw"
        | "cr2" | "nef" | "dng" | "tga" | "exr" | "hdr" | "jxl" | "qoi" => (Image, PURPLE),
        "svg" | "svgz" | "ai" | "eps" | "sketch" | "fig" | "xcf" | "drawio" | "excalidraw" => {
            (Image, YELLOW)
        }

        // 音视频。
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "oga" | "opus" | "aiff" | "aif"
        | "mid" | "midi" | "wma" | "caf" => (Audio, PINK),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp"
        | "ogv" => (Video, RED),

        // 字体。
        "ttf" | "otf" | "woff" | "woff2" | "ttc" | "eot" | "pfb" => (Font, RED),

        // 压缩包、安装包。
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "lz4" | "7z"
        | "rar" | "br" | "lz" | "lzma" | "zstd" | "tzst" | "tbz" | "z" | "lzh" | "lha" | "cab"
        | "jar" | "war" | "ear" | "whl" | "egg" | "crate" | "gem" | "nupkg" | "vsix" | "apk"
        | "aab" | "ipa" | "xpi" | "deb" | "rpm" | "pkg" | "dmg" | "iso" | "img" | "msi"
        | "snap" | "appimage" => (Archive, [0xC9A06A, 0x8A6420]),

        // 程序、库、目标文件。
        "exe" | "dll" | "so" | "dylib" | "a" | "lib" | "o" | "obj" | "bin" | "class" | "pyc"
        | "pyo" | "pyd" | "wasm" | "elf" | "pdb" | "node" | "rlib" | "rmeta" | "dat" | "beam"
        | "elc" | "ko" | "jmod" => (Binary, GREY),

        // 锁文件、证书、密钥。
        "lock" | "lockb" => (Lock, DIM),
        // `.key` 在代码仓库里几乎都是私钥，不当作演示文稿。
        "pem" | "crt" | "cer" | "der" | "p12" | "pfx" | "pub" | "asc" | "gpg" | "sig"
        | "keystore" | "jks" | "csr" | "key" | "p7b" | "p7c" | "crl" | "kdbx" => (Lock, YELLOW),

        // 界面文字的翻译：一条条「键 值」。
        "ftl" | "po" | "pot" | "mo" | "strings" | "stringsdict" | "xcstrings" | "arb" | "resx"
        | "xlf" | "xliff" => (Data, TEAL),

        _ => return None,
    })
}
