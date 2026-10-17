use std::path::{Path, PathBuf};

#[path = "package_name.rs"]
mod package_name;

pub(crate) fn primary_package_name(url: &str, extension: &str) -> Result<String, String> {
    if !matches!(extension, "msi" | "exe") {
        return Err("package cache type invalid".into());
    }
    let path = url
        .strip_prefix("https://download.yan.life/rustdesk/stable/")
        .ok_or("package cache source invalid")?;
    if path.contains(['%', '\\', '?', '#', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("package cache source invalid".into());
    }
    let name = path.rsplit('/').next().ok_or("package cache name absent")?;
    if name.len() > 180
        || !name.ends_with(&format!(".{extension}"))
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
    {
        return Err("package cache name invalid".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("package cache reserved name".into());
    }
    Ok(name.to_owned())
}

pub(crate) struct BindingFields<'a> {
    pub transaction_id: String,
    pub client_id: String,
    pub client_uuid: String,
    pub installation_id: String,
    pub target_key: &'a str,
    pub package_sha256: String,
    pub from_version: &'a str,
    pub from_build_seq: u64,
    pub to_version: &'a str,
    pub to_build_seq: u64,
    pub source: &'a str,
    pub command_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BindingRecord {
    pub transaction_id: String,
    pub client_id: String,
    pub client_uuid: String,
    pub installation_id: String,
    pub target_key: String,
    pub package_sha256: String,
    pub from_version: String,
    pub from_build_seq: u64,
    pub to_version: String,
    pub to_build_seq: u64,
    pub source: String,
    pub command_id: Option<String>,
}

pub(crate) fn binding(fields: BindingFields<'_>) -> BindingRecord {
    BindingRecord {
        transaction_id: fields.transaction_id,
        client_id: fields.client_id,
        client_uuid: fields.client_uuid,
        installation_id: fields.installation_id,
        target_key: fields.target_key.into(),
        package_sha256: fields.package_sha256,
        from_version: fields.from_version.into(),
        from_build_seq: fields.from_build_seq,
        to_version: fields.to_version.into(),
        to_build_seq: fields.to_build_seq,
        source: fields.source.into(),
        command_id: fields.command_id,
    }
}

pub(crate) fn receipt_package_name(name: Option<&str>) -> Result<String, String> {
    match name {
        Some(name) => package_name::validate(name),
        None => Ok("package.msi".into()),
    }
}

pub(crate) fn runtime_package_destination(
    directory: &Path,
    name: Option<&str>,
) -> Result<PathBuf, String> {
    Ok(directory.join(receipt_package_name(name)?))
}

pub(crate) fn runner_arguments<'a>(
    transaction_id: &'a str,
    digest: &'a str,
    exit_code: &'a str,
) -> [&'a str; 4] {
    ["--ordinary-msi-outcome", transaction_id, digest, exit_code]
}
