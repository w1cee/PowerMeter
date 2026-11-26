pub fn installed_version() -> Option<(u32, u32)> {
    Some((2, 2))
}

pub fn install(_progress: impl Fn(&str)) -> Result<(), String> {
    Ok(())
}
