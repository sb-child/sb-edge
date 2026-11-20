use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "sb-edge",
    author,
    version,
    about = "sbchild edge network server",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Start(StartArgs),
}

/// 启动服务器
#[derive(Args, Debug)]
pub struct StartArgs {
    /// 配置目录路径
    #[arg(long, required = true)]
    pub conf_path: PathBuf,

    /// 临时目录路径
    #[arg(long, required = true)]
    pub temp_path: PathBuf,
}
