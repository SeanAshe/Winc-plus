//! 粘贴文件与图片的测试：重名规则、复制目录与符号链接、Markdown 链接、扩展名猜测与 PNG 编码。

use std::fs;

use super::*;
use crate::test_util::scratch;

#[test]
fn names_follow_the_numbered_and_copy_schemes() {
    assert_eq!(split_name("a.tar.gz"), ("a.tar", ".gz"));
    assert_eq!(split_name(".env"), (".env", ""));
    assert_eq!(split_name("Makefile"), ("Makefile", ""));
    assert_eq!(candidate("image", ".png", 0, Naming::Numbered), "image.png");
    assert_eq!(
        candidate("image", ".png", 2, Naming::Numbered),
        "image-2.png"
    );
    assert_eq!(candidate("a", ".txt", 1, Naming::Copy), "a 副本.txt");
    assert_eq!(candidate("a", ".txt", 2, Naming::Copy), "a 副本 2.txt");
}

#[test]
fn saved_images_never_overwrite() {
    let dir = scratch("image");
    let first = save_image(&dir, b"one").unwrap();
    let second = save_image(&dir, b"two").unwrap();
    assert_eq!(first, dir.join("image.png"));
    assert_eq!(second, dir.join("image-1.png"));
    assert_eq!(fs::read(&first).unwrap(), b"one");
    assert_eq!(fs::read(&second).unwrap(), b"two");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn copies_files_and_folders_without_overwriting() {
    let dir = scratch("copy");
    let src = dir.join("src");
    fs::create_dir_all(src.join("folder/inner")).unwrap();
    fs::write(src.join("a.txt"), "a").unwrap();
    fs::write(src.join("folder/inner/b.txt"), "b").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.txt", src.join("link")).unwrap();
    let dest = dir.join("dest");
    fs::create_dir(&dest).unwrap();

    let sources = [src.join("a.txt"), src.join("folder")];
    let copied = copy_into(&sources, &dest, &Copying::default()).unwrap();
    assert_eq!(copied, [dest.join("a.txt"), dest.join("folder")]);
    assert_eq!(
        fs::read_to_string(dest.join("folder/inner/b.txt")).unwrap(),
        "b"
    );

    // 再粘贴一次：同名项已存在，按重名规则取名，原有内容不变。
    fs::write(dest.join("a.txt"), "changed").unwrap();
    let again = copy_into(&sources, &dest, &Copying::default()).unwrap();
    assert_eq!(again, [dest.join("a 副本.txt"), dest.join("folder 副本")]);
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "changed");
    let third = copy_into(&sources[..1], &dest, &Copying::default()).unwrap();
    assert_eq!(third, [dest.join("a 副本 2.txt")]);

    // 粘贴到源文件所在的目录：得到一个副本。
    let beside = copy_into(&sources[..1], &src, &Copying::default()).unwrap();
    assert_eq!(beside, [src.join("a 副本.txt")]);

    // 符号链接复制为链接本身。
    #[cfg(unix)]
    {
        let link = copy_into(&[src.join("link")], &dest, &Copying::default()).unwrap();
        assert_eq!(fs::read_link(&link[0]).unwrap(), Path::new("a.txt"));
    }

    // 目录粘贴到自身：在旁边得到副本；粘贴到自身之内更深的位置则被拒绝。
    let folder = src.join("folder");
    let duplicate = copy_into(std::slice::from_ref(&folder), &folder, &Copying::default()).unwrap();
    assert_eq!(duplicate, [src.join("folder 副本")]);
    assert!(src.join("folder 副本/inner/b.txt").is_file());
    let err = copy_into(
        std::slice::from_ref(&src),
        &src.join("folder"),
        &Copying::default(),
    )
    .unwrap_err();
    assert_eq!(err.error.kind(), io::ErrorKind::InvalidInput);
    // 不存在的源报告是哪一项。
    let missing = src.join("missing.txt");
    let err = copy_into(std::slice::from_ref(&missing), &dest, &Copying::default()).unwrap_err();
    assert_eq!(err.path, missing);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn markdown_links_are_relative_and_escaped() {
    let dir = scratch("links");
    let docs = dir.join("docs");
    fs::create_dir_all(docs.join("img")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    for file in [
        "docs/img/a.png",
        "assets/my logo.svg",
        "assets/x(1.pdf",
        "notes.txt",
    ] {
        fs::write(dir.join(file), "").unwrap();
    }

    let one = |file: &str| markdown_links(Some(&docs), Some(&dir), &[dir.join(file)]);
    assert_eq!(
        one("docs/img/a.png"),
        Snippet {
            text: "![a](img/a.png)".into(),
            placeholder: Some(2..3),
        }
    );
    // 空白和不配对的括号用尖括号括起来；文件写成链接，链接文字是完整的文件名。
    assert_eq!(
        one("assets/my logo.svg").text,
        "![my logo](<../assets/my logo.svg>)"
    );
    assert_eq!(one("assets/x(1.pdf").text, "[x(1.pdf](<../assets/x(1.pdf>)");
    assert_eq!(one("notes.txt").placeholder, Some(1..10));

    // 多项之间用空格隔开，不选中占位文字。
    let both = markdown_links(
        Some(&docs),
        Some(&dir),
        &[dir.join("notes.txt"), dir.join("docs/img/a.png")],
    );
    assert_eq!(both.text, "[notes.txt](../notes.txt) ![a](img/a.png)");
    assert_eq!(both.placeholder, None);

    // 项目之外的文件，以及文档尚未保存时，写成绝对路径。
    let outside = std::env::temp_dir().join(format!("winc-outside-{}.pdf", std::process::id()));
    fs::write(&outside, "").unwrap();
    let text = markdown_links(Some(&docs), Some(&dir), std::slice::from_ref(&outside)).text;
    // 绝对路径：Unix 上以 `/` 开头，Windows 上以盘符开头（`C:/…`）。
    let absolute = if cfg!(windows) { ":/" } else { "](/" };
    assert!(
        text.starts_with("[winc-outside-") && text.contains(absolute),
        "{text}"
    );
    let unsaved = markdown_links(None, None, &[dir.join("notes.txt")]).text;
    assert!(unsaved.contains(absolute), "{unsaved}");

    let _ = fs::remove_file(&outside);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn images_from_outside_the_project_are_copied_next_to_the_document() {
    let dir = scratch("bring");
    let project = dir.join("project");
    fs::create_dir_all(project.join("docs")).unwrap();
    fs::write(project.join("inside.png"), "in").unwrap();
    fs::write(dir.join("photo.JPG"), "out").unwrap();
    fs::write(dir.join("report.pdf"), "pdf").unwrap();
    fs::write(project.join("docs/photo.JPG"), "taken").unwrap();

    let docs = project.join("docs");
    let files = [
        project.join("inside.png"),
        dir.join("photo.JPG"),
        dir.join("report.pdf"),
    ];
    let targets = bring_images(&docs, Some(&project), &files).unwrap();
    // 项目里的图片和项目外的其他文件原样引用；项目外的图片复制到文档旁边，重名时加序号。
    assert_eq!(targets[0], project.join("inside.png"));
    assert_eq!(targets[1], docs.join("photo-1.JPG"));
    assert_eq!(fs::read_to_string(&targets[1]).unwrap(), "out");
    assert_eq!(targets[2], dir.join("report.pdf"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pasted_text_and_images_get_free_names_with_a_guessed_extension() {
    let dir = scratch("names");
    assert_eq!(text_file_name(&dir, "随便写点什么"), "untitled.txt");
    fs::write(dir.join("untitled.txt"), "").unwrap();
    assert_eq!(text_file_name(&dir, "还是文本"), "untitled-1.txt");
    assert_eq!(image_file_name(&dir), "image.png");
    fs::write(dir.join("image.png"), "").unwrap();
    assert_eq!(image_file_name(&dir), "image-1.png");

    for (text, ext) in [
        ("  {\"a\": 1}\n", "json"),
        ("[1, 2]", "json"),
        ("<svg xmlns=\"http://www.w3.org/2000/svg\"/>", "svg"),
        ("<?xml version=\"1.0\"?>\n<svg/>", "svg"),
        ("<?xml version=\"1.0\"?>\n<plist/>", "xml"),
        ("<!DOCTYPE html>\n<p>hi</p>", "html"),
        ("#!/usr/bin/env python3\nprint(1)", "py"),
        ("#!/bin/bash\necho hi", "sh"),
        ("#!/usr/bin/env node\n", "js"),
        ("# 标题\n正文", "txt"),
        ("{ 不配对", "txt"),
    ] {
        assert_eq!(guess_extension(text), ext, "{text:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rgba_pixels_round_trip_through_png() {
    let pixels: Vec<u8> = (0..2 * 3 * 4).map(|i| (i * 10) as u8).collect();
    let png = encode_png(2, 3, &pixels).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut decoded).unwrap();
    assert_eq!((info.width, info.height), (2, 3));
    assert_eq!(&decoded[..info.buffer_size()], pixels.as_slice());

    // 尺寸与数据长度不符（2 × 2 只需 16 字节）、尺寸为 0：都不编码。
    assert_eq!(encode_png(2, 2, &pixels), None);
    assert_eq!(encode_png(0, 0, &[]), None);
}

#[test]
fn link_targets_need_brackets_only_when_ambiguous() {
    assert!(!needs_brackets("a/b(c).png"));
    assert!(!needs_brackets(r"a\(b.png"));
    assert!(!needs_brackets("中文/截图.png"));
    assert!(needs_brackets("a b.png"));
    assert!(needs_brackets("a)b.png"));
    assert!(needs_brackets("(a.png"));
    assert!(needs_brackets("<a.png"));
    assert_eq!(escape_link_path("a <b>.png"), r"<a \<b\>.png>");
    assert_eq!(escape_brackets("a[1]"), r"a\[1\]");
}
