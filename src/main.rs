#[cfg(not(feature = "android"))]
fn main() -> anyhow::Result<()> {
    normal_startup()
}

#[cfg(feature = "android")]
fn main() -> anyhow::Result<()> {
    android_startup()
}

#[allow(dead_code)]
fn normal_startup() -> anyhow::Result<()> {
    Ok(())
}

#[allow(dead_code)]
fn android_startup() -> anyhow::Result<()> {
    Ok(())
}
