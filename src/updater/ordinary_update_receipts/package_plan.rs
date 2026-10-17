use std::path::{Path, PathBuf};

use super::package_contract;

/// 调用方须先验证 manifest 签名及下载内容；传输文件名不决定批准名称。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PackagePlan {
    pub source: PathBuf,
    pub approved_package_name: String,
    pub destination: PathBuf,
}

pub(crate) fn plan(
    signed_primary: &str,
    downloaded_source: &Path,
    directory: &Path,
) -> Result<PackagePlan, String> {
    let approved_package_name = package_contract::primary_package_name(signed_primary, "msi")?;
    let destination =
        package_contract::runtime_package_destination(directory, Some(&approved_package_name))?;
    Ok(PackagePlan {
        source: downloaded_source.to_owned(),
        approved_package_name,
        destination,
    })
}
