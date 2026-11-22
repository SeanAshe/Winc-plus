//! 跳转与定位：书签、转到定义、文件内的符号、配置文件的键路径。
//!
//! - `bookmarks`：行书签的增删，以及书签随文本编辑移动。
//! - `definition`：按常见的定义写法，在当前文件或整个工作区里查找定义。
//! - `usages`：从全字匹配的搜索结果中剔除注释、无关语言和其他同名定义，留下名字的用法。
//! - `symbols`：提取文件内的函数、类型、标题等符号，供转到符号使用。
//! - `keypath`：计算光标在 JSON、YAML、TOML 文件中的键路径。
//! - `links`：找出一行中的网址、Markdown 链接和维基链接。
//!
//! 各模块只处理调用方传入的文本，不访问磁盘，也不依赖界面。

pub mod bookmarks;
pub mod definition;
pub mod keypath;
pub mod links;
pub mod symbols;
pub mod usages;
