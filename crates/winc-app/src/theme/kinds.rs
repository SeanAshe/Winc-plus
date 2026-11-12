//! 按文件名给出文件图标的记号和颜色。
//!
//! [`file_kind`] 返回记号和一对颜色，由 [`Palette::file_icon`] 按当前配色深浅选其中一个；
//! 记号的形状见 `crate::icons::FileGlyph`。

use super::*;

/// 返回文件名对应的类型记号和颜色（深色、浅色配色各一个，均为 `0xRRGGBB`）。
///
/// 先按整个文件名识别（如 `Dockerfile`、`go.mod`、`.gitignore`），再按扩展名识别；两者都不
/// 匹配时返回 `Plain`。
pub fn file_kind(name: &str) -> (FileGlyph, [u32; 2]) {
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
        "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "bun.lockb" | "bun.lock"
        | "cargo.lock" | "poetry.lock" | "pipfile.lock" | "composer.lock" | "gemfile.lock"
        | "go.sum" | "flake.lock" | "uv.lock" | "podfile.lock" | "mix.lock" => Some((Lock, DIM)),
        // 包的清单：按生态配色。
        "package.json" => Some((Package, NPM)),
        "cargo.toml" => Some((Package, RUST)),
        "go.mod" | "go.work" => Some((Package, GO)),
        "pyproject.toml" | "setup.py" | "setup.cfg" | "pipfile" | "requirements.txt" => {
            Some((Package, [0x6CA6E0, 0x2F6FB0]))
        }
        "composer.json" => Some((Package, PURPLE)),
        "gemfile" | "podfile" => Some((Package, RED)),
        "build.gradle" | "build.gradle.kts" | "pom.xml" => Some((Package, PURPLE)),
        "makefile" | "gnumakefile" | "justfile" | "rakefile" | "vagrantfile" | "brewfile"
        | "procfile" | "jenkinsfile" | "cmakelists.txt" => Some((Shell, SHELL)),
        // 许可证才是徽章；NOTICE、CHANGELOG 这些是普通的文字文档。
        _ if stem == "license" || stem == "licence" || stem == "copying" => Some((License, YELLOW)),
        _ if stem == "notice" => Some((Text, DOC)),
        _ if stem == "readme" => Some((Info, DOC)),
        "authors" | "changelog" | "contributing" | "changes" | "history" => Some((Text, DOC)),
        "codeowners" | "owners" => Some((Data, GREY)),
        _ if lower.starts_with(".env") => Some((Key, YELLOW)),
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

    match ext {
        // 代码，按语言配色。
        // 常见语言画缩写；JSX、TSX 与 JS、TS 用同一个缩写，改用青绿色与纯 JS、TS 区分。
        "py" | "pyi" | "pyw" | "pyx" | "ipynb" => (Letters("PY"), [0x6CA6E0, 0x2F6FB0]),
        "rs" => (Letters("RS"), RUST),
        "js" | "mjs" | "cjs" => (Letters("JS"), [0xE6C35C, 0x9A7A0E]),
        "jsx" => (Letters("JS"), REACT),
        "ts" | "mts" | "cts" => (Letters("TS"), [0x6C9FE0, 0x2D62B0]),
        "tsx" => (Letters("TS"), REACT),
        "ets" => (Letters("ETS"), [0x6C9FE0, 0x2D62B0]),
        "go" => (Letters("GO"), GO),
        "java" => (Java, [0xE08C5A, 0xA9521D]),
        "kt" | "kts" => (Letters("KT"), PURPLE),
        "scala" | "sc" | "groovy" | "gradle" => (Code, PURPLE),
        "swift" => (Letters("SW"), [0xF0845E, 0xC4441E]),
        "c" | "h" => (Letters("C"), [0x8FA6D6, 0x4A65A0]),
        "cpp" | "cc" | "hpp" | "cxx" | "hh" | "hxx" | "ino" | "cu" => {
            (Letters("C++"), [0x8FA6D6, 0x4A65A0])
        }
        "m" | "mm" => (Code, [0x8FA6D6, 0x4A65A0]),
        "cs" | "csx" => (Letters("C#"), [0x9B8BE0, 0x5A47B0]),
        "fs" | "fsx" | "vb" => (Code, [0x9B8BE0, 0x5A47B0]),
        "html" | "htm" | "xhtml" | "vue" | "svelte" | "astro" => (Code, ORANGE),
        "xml" | "xsd" | "xsl" | "xslt" | "plist" | "csproj" | "fsproj" | "vbproj" | "props"
        | "targets" | "xib" | "storyboard" | "wsdl" => (Code, ORANGE),
        "css" | "scss" | "less" | "sass" | "styl" | "pcss" => (Code, BLUE),
        "sql" | "psql" | "mysql" | "pgsql" => (Database, YELLOW),
        "prisma" | "graphql" | "gql" => (Code, PINK),
        "rb" | "erb" | "rake" | "gemspec" => (Letters("RB"), RED),
        "php" | "phtml" => (Code, [0x8F9BD8, 0x4D5AA0]),
        "lua" | "luau" => (Code, [0x6C8FE0, 0x2A4FB0]),
        "dart" => (Code, [0x5CB8E6, 0x1A78A8]),
        "r" | "rmd" | "jl" => (Code, [0x7FA8D8, 0x3B6AA0]),
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

        // 脚本。
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "bashrc" | "zshrc" | "profile"
        | "bash_profile" | "zprofile" | "mk" | "cmake" | "just" | "nu" => (Shell, SHELL),
        "ps1" | "psm1" | "psd1" | "bat" | "cmd" => (Shell, BLUE),

        // 配置和数据。
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" | "webmanifest" => {
            (Json, YELLOW)
        }
        // 配置文件都是齿轮，按格式配色：先凭形状认出是配置文件，再凭颜色分辨具体格式。
        "yaml" | "yml" => (Config, RED),
        "toml" => (Config, TOML),
        "ini" | "cfg" | "conf" | "config" | "properties" | "editorconfig" | "npmrc" | "yarnrc"
        | "nvmrc" | "prettierrc" | "eslintrc" | "babelrc" | "browserslistrc" | "stylelintrc"
        | "reg" | "desktop" | "service" => (Config, GREY),
        "env" => (Key, YELLOW),
        "ignore" | "npmignore" | "eslintignore" | "prettierignore" | "keep" => (Data, DIM),
        "csv" | "tsv" | "psv" => (Table, GREEN),
        "xls" | "xlsx" | "numbers" | "ods" => (Table, GREEN),
        "sqlite" | "sqlite3" | "db" | "mdb" | "duckdb" => (Database, YELLOW),
        "parquet" | "arrow" | "feather" | "avro" | "orc" | "h5" | "hdf5" | "npy" | "npz"
        | "pkl" | "pickle" => (Binary, YELLOW),

        // 文档。
        "md" | "markdown" | "mdx" => (Markdown, DOC),
        "rst" | "adoc" | "asciidoc" | "org" | "tex" | "bib" | "typ" => (Text, DOC),
        "txt" | "text" | "log" | "out" | "rtf" | "nfo" => (Text, GREY),
        "pdf" => (Text, RED),
        "doc" | "docx" | "pages" | "odt" | "epub" => (Text, BLUE),
        "ppt" | "pptx" | "key" | "odp" => (Text, ORANGE),
        "diff" | "patch" => (Text, GREEN),

        // 图片。
        "png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp" | "bmp" | "dib"
        | "ico" | "cur" | "icns" | "heic" | "heif" | "avif" | "tif" | "tiff" | "psd" | "raw"
        | "cr2" | "nef" | "dng" | "tga" | "exr" | "hdr" | "jxl" | "qoi" => (Image, PURPLE),
        "svg" | "svgz" | "ai" | "eps" | "sketch" | "fig" | "xcf" => (Image, YELLOW),

        // 音视频。
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "oga" | "opus" | "aiff" | "aif"
        | "mid" | "midi" | "wma" | "caf" => (Audio, PINK),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp"
        | "ogv" => (Video, RED),

        // 字体。
        "ttf" | "otf" | "woff" | "woff2" | "ttc" | "eot" | "pfb" => (Font, RED),

        // 压缩包、安装包。
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "lz4" | "7z"
        | "rar" | "br" | "lz" | "lzma" | "cab" | "jar" | "war" | "ear" | "whl" | "egg"
        | "crate" | "gem" | "nupkg" | "vsix" | "apk" | "aab" | "ipa" | "xpi" | "deb" | "rpm"
        | "pkg" | "dmg" | "iso" | "img" | "msi" | "snap" | "appimage" => {
            (Archive, [0xC9A06A, 0x8A6420])
        }

        // 程序、库、目标文件。
        "exe" | "dll" | "so" | "dylib" | "a" | "lib" | "o" | "obj" | "bin" | "class" | "pyc"
        | "pyo" | "wasm" | "elf" | "pdb" | "node" | "rlib" | "rmeta" | "dat" => (Binary, GREY),

        // 锁文件、证书、密钥。
        "lock" | "lockb" => (Lock, DIM),
        "pem" | "crt" | "cer" | "der" | "p12" | "pfx" | "pub" | "asc" | "gpg" | "sig"
        | "keystore" | "jks" | "csr" => (Lock, YELLOW),

        _ => (Plain, [0x9AA0AA, 0x6B707A]),
    }
}
