//! 配置文件：各平台的路径、JSON 解析、范围校验和写回。
//!
//! [`Config`] 是用户编写的用户级配置（各平台的路径见 [`config_path`]），项目级配置见
//! [`Config::load_project`]；[`State`] 是程序维护的运行状态，存在 [`state_path`] 指向的
//! `state.json` 中。写入都先写临时文件并落盘，再改名覆盖。

mod config;
pub mod files;
mod font;
mod json;
mod run;
mod state;
mod version;

pub use config::{
    BackgroundFit, Backup, Config, OptionAsMeta, RenderWhitespace, Saved, ThemeChoice,
    VimClipboard, VimMapMode, VimMapping, config_path,
};
pub use font::{FontRequest, resolve_font};
pub use json::Json;
pub use run::{
    RunConfig, affects_detected_configs, create_run_configs, detected_run_configs,
    load_run_configs, run_configs_path,
};
pub use state::{
    BADGE_COLORS, MAX_RECENT, Session, State, Tab, TerminalSession, WindowFrame, state_path,
};
pub use version::compare_versions;
