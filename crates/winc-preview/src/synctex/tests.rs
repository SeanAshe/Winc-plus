//! `synctex` 的测试：解析、单位换算、源码到 PDF 和 PDF 到源码的对照，以及 gzip。

use super::*;

/// 两页、两个输入文件的样本；坐标按 `Unit:1` 写成 sp，65781.76 sp 是 1 bp。
const SAMPLE: &str = "SyncTeX Version:1
Input:1:./main.tex
Input:3:
Output:pdf
Magnification:1000
Unit:1
X Offset:0
Y Offset:0
Content:
{1
[1,10:0,0:100,100,0
h1,10:657818,1315635:10,10,0
x1,12:657818,2631270
Input:2:./chapters/intro.tex
k2,3:1315635,3946906
$2,3:1973453,3946906
]
}1
{2
[1,12:0,0:100,100,0
x2,8:657818,657818
}2
Postamble:
";

fn sample() -> Synctex {
    Synctex::parse(SAMPLE, Path::new("/p"))
}

#[test]
fn source_lines_find_their_place_on_the_pages() {
    let s = sample();
    let main = Path::new("/p/main.tex");
    // 第 12 行：第 1 页，横 10 bp、纵 40 bp（从页面左上角算起）。
    let (page, h, v) = s.forward(main, 12).unwrap();
    assert_eq!(page, 0);
    assert!(
        (h - 10.0).abs() < 0.01 && (v - 40.0).abs() < 0.01,
        "{h} {v}"
    );
    // 第 2 页的盒子排完时正读到第 12 行，但第 12 行的内容在第 1 页。
    assert_eq!(s.forward(main, 12).map(|f| f.0), Some(0));
    // 没有记录的行取前面最近的一行；比所有记录都靠前时取后面最近的。
    assert_eq!(
        s.forward(main, 13).map(|f| f.2),
        s.forward(main, 12).map(|f| f.2)
    );
    assert_eq!(s.forward(main, 1).map(|f| f.0), Some(0));
    // 引入的文件按自己的行找，`./` 和 `..` 不影响。
    let intro = Path::new("/p/chapters/../chapters/intro.tex");
    assert_eq!(s.forward(intro, 8).map(|f| f.0), Some(1));
    assert_eq!(s.forward(Path::new("/p/other.tex"), 1), None);
}

#[test]
fn places_on_a_page_lead_back_to_the_source() {
    let s = sample();
    // 第 1 页纵 40 bp 附近是 main.tex 的第 12 行；更低处是 intro.tex 的第 3 行。
    assert_eq!(s.inverse(0, 10.0, 41.0), Some(("/p/main.tex".into(), 12)));
    assert_eq!(
        s.inverse(0, 28.0, 60.0),
        Some(("/p/chapters/intro.tex".into(), 3))
    );
    assert_eq!(
        s.inverse(1, 0.0, 0.0),
        Some(("/p/chapters/intro.tex".into(), 8))
    );
    assert_eq!(s.inverse(5, 0.0, 0.0), None);
}

#[test]
fn magnification_and_unit_scale_the_coordinates() {
    let text = SAMPLE
        .replace("Unit:1", "Unit:2")
        .replace("Magnification:1000", "Magnification:500");
    let s = Synctex::parse(&text, Path::new("/p"));
    let (_, h, _) = s.forward(Path::new("/p/main.tex"), 12).unwrap();
    assert!((h - 10.0).abs() < 0.01, "{h}");
}

#[test]
fn gzip_files_are_read() {
    let deflated = miniz_oxide::deflate::compress_to_vec(SAMPLE.as_bytes(), 6);
    // gzip 头：带文件名，后面是压缩数据和 8 字节的校验与长度（读的时候不用）。
    let mut gz = vec![0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 0, 3];
    gz.extend_from_slice(b"main.synctex\0");
    gz.extend_from_slice(&deflated);
    gz.extend_from_slice(&[0; 8]);
    assert_eq!(Synctex::read(&gz, Path::new("/p")), Some(sample()));
    assert_eq!(
        Synctex::read(SAMPLE.as_bytes(), Path::new("/p")),
        Some(sample())
    );
    assert_eq!(Synctex::read(&[0x1f, 0x8b, 9], Path::new("/p")), None);
}
