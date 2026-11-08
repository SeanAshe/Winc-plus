//! 命令面板的测试：模糊匹配的打分、匹配字符的标注，以及中英文命令名都能命中。

use super::*;

fn titles(query: &str) -> Vec<&'static str> {
    search_palette(query)
        .into_iter()
        .map(|i| palette()[i].title())
        .collect()
}

/// 标出来的字和打分一样：连续出现时是第一处那一段（空白跳过，下标按原文算），分散出现时是按
/// 顺序最先找到的几处，不命中时为空。
#[test]
fn matched_characters_are_the_ones_that_were_rated() {
    assert_eq!(matched(&needle("格式"), "格式化文档"), vec![0, 1]);
    assert_eq!(
        matched(&needle("tdoc"), "Format Document"),
        vec![5, 7, 8, 9]
    );
    assert_eq!(
        matched(&needle("fmtsel"), "Format Selection"),
        vec![0, 3, 5, 7, 8, 9]
    );
    assert!(matched(&needle("zzz"), "格式化文档").is_empty());
    assert!(matched(&needle(""), "格式化文档").is_empty());
}

#[test]
fn commands_match_chinese_and_english_names() {
    assert_eq!(titles("格式化")[0], "格式化文档");
    assert_eq!(titles("format doc")[0], "格式化文档");
    assert_eq!(titles("FORMAT")[0], "格式化文档");
    // 查询中的空白被忽略；分散出现的字符序列同样匹配（`zoomin`、`fmtsel`）。
    assert_eq!(titles("zoomin")[0], "放大界面");
    assert_eq!(titles("increase font")[0], "放大字号");
    assert_eq!(titles("fmtsel")[0], "格式化所选内容");
    assert!(titles("修复").contains(&"修复问题"));
    assert!(titles("没有这条命令").is_empty());
    assert_eq!(titles("").len(), palette().len());
    // 仅在特定上下文有意义的动作（如查找条的开关）不在面板中列出。
    assert!(titles("区分大小写").is_empty());
    assert!(titles("快捷键").contains(&"快捷键"));
}
