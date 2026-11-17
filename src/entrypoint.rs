use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchArgs {}

pub fn entrypoint(_args: LaunchArgs) -> anyhow::Result<()> {
    Ok(())
}
