use std::path::Path;

pub(crate) fn validate(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 180
        || !name.ends_with(".msi")
        || name.contains(['/', '\\', ':', '%', '?', '#'])
        || name.chars().any(char::is_control)
        || name
            .split('.')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
    {
        return Err("ordinary package basename invalid".into());
    }
    Ok(name.to_owned())
}

pub(crate) fn from_path(package: &Path) -> Result<String, String> {
    validate(
        package
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("ordinary package basename unavailable")?,
    )
}
