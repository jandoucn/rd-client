//! 更新后端不可达时的 OSS 后备 manifest。
//!
//! 后端 `/check` 按设备下发策略，OSS 上的静态文件不带策略，所以只在后端连不上时使用，
//! 并沿用上一次从后端拿到的策略：从没连上过后端、上一次是 `disabled`、缓存超过 7 天，
//! 都不使用后备。后端会按设备固定版本、只给仍启用的版本，所以后备也不会越过上一次
//! 后端给这台设备的目标构建号。包的真伪仍由每个目标的 SHA-256 与签名保证。

use base::{config::keys, update::update_target_release};
use hbb_common::{
    bail, config, is_newer_version, log, ResultType, UpdateManifest, VersionCheckResponse,
};

pub(crate) const FALLBACK_MANIFEST_URLS: [&str; 2] = [
    "https://download.yan.life/rustdesk/stable/manifest/stable.json",
    "https://rustdesk-release.oss-cn-shanghai.aliyuncs.com/rustdesk/stable/manifest/stable.json",
];
const MAX_FALLBACK_MANIFEST_BYTES: usize = 512 * 1024;
const POLICY_CACHE_TTL_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize,
)]
pub(crate) struct CachedPolicy {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub auto_install: bool,
    #[serde(default)]
    pub max_build_seq: u64,
    #[serde(default)]
    pub cached_at: i64,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn policy_of(response: &VersionCheckResponse, now: i64) -> CachedPolicy {
    CachedPolicy {
        mode: response.mode.clone(),
        auto_install: response.auto_install,
        max_build_seq: response.target_build_seq,
        cached_at: now,
    }
}

fn parse_cached_policy(value: &str, now: i64) -> Option<CachedPolicy> {
    serde_json::from_str::<CachedPolicy>(value)
        .ok()
        .filter(|policy| {
            !policy.mode.is_empty()
                && policy.max_build_seq > 0
                && policy.cached_at > 0
                && policy.cached_at <= now
                && now - policy.cached_at <= POLICY_CACHE_TTL_SECS
        })
}

fn remember_backend_policy(response: &VersionCheckResponse) {
    let Ok(value) = serde_json::to_string(&policy_of(response, now_unix())) else {
        return;
    };
    config::Config::set_option(keys::OPTION_UPDATE_LAST_POLICY.to_owned(), value);
}

fn cached_policy() -> Option<CachedPolicy> {
    parse_cached_policy(
        &config::Config::get_option(keys::OPTION_UPDATE_LAST_POLICY),
        now_unix(),
    )
}

// 远程命令由后端下发，能收到命令就说明后端可达，不应拿静态文件替代它。
fn origin_allows_fallback(request_origin: &str) -> bool {
    !request_origin.starts_with("command")
}

// 4xx 是后端明确拒绝（鉴权失败、设备被吊销），不能借后备文件绕过。
fn status_means_unreachable(status: Option<u16>) -> bool {
    status.map_or(true, |status| (500..600).contains(&status))
}

fn backend_unreachable(err: &hbb_common::anyhow::Error) -> bool {
    err.downcast_ref::<reqwest::Error>().is_some_and(|err| {
        !err.is_decode()
            && !err.is_builder()
            && !err.is_redirect()
            && status_means_unreachable(err.status().map(|status| status.as_u16()))
    })
}

fn parse_fallback_manifest(bytes: &[u8]) -> ResultType<UpdateManifest> {
    if bytes.len() > MAX_FALLBACK_MANIFEST_BYTES {
        bail!("fallback manifest exceeds size limit");
    }
    let manifest: UpdateManifest = serde_json::from_slice(bytes)?;
    if manifest.schema < 2 || manifest.targets.is_empty() {
        bail!("fallback manifest is not a complete schema 2 manifest");
    }
    Ok(manifest)
}

fn fallback_response(
    policy: &CachedPolicy,
    manifest: UpdateManifest,
    manifest_url: &str,
    target_key: &str,
    current_version: &str,
    current_build_seq: u64,
) -> Option<VersionCheckResponse> {
    if policy.mode.is_empty() || policy.mode == "disabled" {
        return None;
    }
    let target = manifest.targets.get(target_key)?;
    let (target_version, target_build_seq, _) = update_target_release(&manifest, target).ok()?;
    if target_build_seq > policy.max_build_seq {
        return None;
    }
    let target_version = target_version.to_owned();
    let url = target.primary.clone();
    let update_available = is_newer_version(
        &target_version,
        target_build_seq,
        current_version,
        current_build_seq,
    );
    Some(VersionCheckResponse {
        url,
        update_available,
        mode: policy.mode.clone(),
        auto_install: policy.auto_install,
        channel: manifest.channel.clone(),
        current_version: current_version.to_owned(),
        current_build_seq,
        target_version,
        target_build_seq,
        manifest_url: manifest_url.to_owned(),
        manifest: Some(manifest),
    })
}

async fn fetch_one(url: &str) -> ResultType<UpdateManifest> {
    let client = crate::hbbs_http::create_http_client_async_with_url_strict(url).await?;
    let response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FALLBACK_MANIFEST_BYTES as u64)
    {
        bail!("fallback manifest exceeds size limit");
    }
    parse_fallback_manifest(&response.bytes().await?)
}

async fn fetch_fallback_manifest() -> ResultType<(&'static str, UpdateManifest)> {
    for url in FALLBACK_MANIFEST_URLS {
        match fetch_one(url).await {
            Ok(manifest) => return Ok((url, manifest)),
            Err(err) => log::warn!("Fallback update manifest {url} failed: {err}"),
        }
    }
    bail!("all fallback update manifest sources failed")
}

/// 后端 `/check` 成功时记下它的策略；后端不可达时按上次策略改用 OSS 后备 manifest。
/// 用不上后备时原样返回后端的错误。
pub(crate) async fn resolve_check_response(
    backend: ResultType<VersionCheckResponse>,
    request_origin: &str,
    target_key: &str,
) -> ResultType<VersionCheckResponse> {
    let err = match backend {
        Ok(response) => {
            remember_backend_policy(&response);
            return Ok(response);
        }
        Err(err) => err,
    };
    if !origin_allows_fallback(request_origin) || !backend_unreachable(&err) {
        return Err(err);
    }
    let Some(policy) = cached_policy() else {
        return Err(err);
    };
    if policy.mode == "disabled" {
        return Err(err);
    }
    match fetch_fallback_manifest().await {
        Ok((url, manifest)) => match fallback_response(
            &policy,
            manifest,
            url,
            target_key,
            crate::VERSION,
            crate::BUILD_SEQ,
        ) {
            Some(response) => {
                log::warn!("Update backend unreachable ({err}); using fallback manifest {url}");
                *crate::common::LAST_UPDATE_ERROR.lock().unwrap() =
                    format!("update backend unreachable, used fallback manifest {url}: {err}");
                Ok(response)
            }
            None => Err(err),
        },
        Err(fallback_err) => {
            log::warn!("Fallback update manifest unavailable: {fallback_err}");
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::UpdateTarget;
    use std::collections::HashMap;

    const KEY: &str = "windows-x86_64-msi-sos";

    fn manifest(build_seq: u64) -> UpdateManifest {
        UpdateManifest {
            schema: 2,
            catalog_revision: build_seq,
            version: "1.5.4".to_owned(),
            build_number: "20261029.3".to_owned(),
            build_seq,
            product: "rustdesk-yan".to_owned(),
            edition: "multi".to_owned(),
            channel: "stable".to_owned(),
            source_commit: "a".repeat(40),
            targets: HashMap::from([(
                KEY.to_owned(),
                UpdateTarget {
                    version: "1.5.4".to_owned(),
                    build_number: "20261029.3".to_owned(),
                    build_seq,
                    source_commit: "a".repeat(40),
                    source_tag: "v1.5.4-build-2026.10.29-03".to_owned(),
                    primary: "https://download.yan.life/rustdesk/stable/t/a.msi".to_owned(),
                    mirrors: vec![],
                    size: 1,
                    sha256: "b".repeat(64),
                    signature: "sig".to_owned(),
                    signature_key_id: "yan-release-2026".to_owned(),
                },
            )]),
        }
    }

    const NOW: i64 = 1_793_273_210;

    fn policy(mode: &str, auto_install: bool) -> CachedPolicy {
        CachedPolicy {
            mode: mode.to_owned(),
            auto_install,
            max_build_seq: 2026102903,
            cached_at: NOW,
        }
    }

    #[test]
    fn fallback_urls_share_one_path_on_two_https_hosts() {
        let paths: Vec<_> = FALLBACK_MANIFEST_URLS
            .iter()
            .map(|url| {
                let url = url::Url::parse(url).unwrap();
                assert_eq!(url.scheme(), "https");
                url.path().to_owned()
            })
            .collect();
        assert_eq!(paths[0], "/rustdesk/stable/manifest/stable.json");
        assert_eq!(paths[0], paths[1]);
        assert!(FALLBACK_MANIFEST_URLS[0].starts_with("https://download.yan.life/"));
    }

    #[test]
    fn backend_policy_is_cached_with_its_target_ceiling() {
        let response = VersionCheckResponse {
            mode: "auto_install".to_owned(),
            auto_install: true,
            target_build_seq: 2026102903,
            ..Default::default()
        };
        let cached = policy_of(&response, NOW);
        assert_eq!(cached, policy("auto_install", true));
        let stored = serde_json::to_string(&cached).unwrap();
        assert_eq!(parse_cached_policy(&stored, NOW), Some(cached));
    }

    #[test]
    fn missing_broken_or_ceilingless_cached_policy_means_no_fallback() {
        assert_eq!(parse_cached_policy("", NOW), None);
        assert_eq!(parse_cached_policy("not json", NOW), None);
        assert_eq!(parse_cached_policy(r#"{"mode":""}"#, NOW), None);
        assert_eq!(
            parse_cached_policy(r#"{"mode":"download","cached_at":1793273210}"#, NOW),
            None
        );
        assert_eq!(
            parse_cached_policy(r#"{"mode":"download","max_build_seq":2026102903}"#, NOW),
            None
        );
    }

    #[test]
    fn cached_policy_expires_after_seven_days() {
        let stored = serde_json::to_string(&policy("download", false)).unwrap();
        assert!(parse_cached_policy(&stored, NOW + POLICY_CACHE_TTL_SECS).is_some());
        assert_eq!(
            parse_cached_policy(&stored, NOW + POLICY_CACHE_TTL_SECS + 1),
            None
        );
        assert_eq!(parse_cached_policy(&stored, NOW - 3600), None);
    }

    #[test]
    fn fallback_never_offers_a_build_above_the_backend_ceiling() {
        let mut pinned = policy("auto_install", true);
        pinned.max_build_seq = 2026102501;
        assert!(fallback_response(
            &pinned,
            manifest(2026102903),
            FALLBACK_MANIFEST_URLS[0],
            KEY,
            "1.5.4",
            2026102401,
        )
        .is_none());
    }

    #[test]
    fn remote_commands_never_use_fallback() {
        assert!(!origin_allows_fallback("command"));
        assert!(!origin_allows_fallback("command_install"));
        assert!(origin_allows_fallback("startup"));
        assert!(origin_allows_fallback("scheduled"));
        assert!(origin_allows_fallback("manual"));
    }

    #[test]
    fn only_transport_failures_and_server_errors_count_as_unreachable() {
        assert!(status_means_unreachable(None));
        assert!(status_means_unreachable(Some(500)));
        assert!(status_means_unreachable(Some(503)));
        assert!(!status_means_unreachable(Some(200)));
        assert!(!status_means_unreachable(Some(401)));
        assert!(!status_means_unreachable(Some(403)));
        assert!(!status_means_unreachable(Some(404)));
        assert!(!status_means_unreachable(Some(429)));
    }

    #[test]
    fn fallback_manifest_parsing_rejects_oversized_empty_and_legacy_files() {
        let good = serde_json::to_vec(&manifest(2026102903)).unwrap();
        assert_eq!(parse_fallback_manifest(&good).unwrap().build_seq, 2026102903);

        let oversized = vec![b' '; MAX_FALLBACK_MANIFEST_BYTES + 1];
        assert!(parse_fallback_manifest(&oversized).is_err());
        assert!(parse_fallback_manifest(b"not json").is_err());

        let mut empty = manifest(2026102903);
        empty.targets.clear();
        assert!(parse_fallback_manifest(&serde_json::to_vec(&empty).unwrap()).is_err());

        let mut legacy = manifest(2026102903);
        legacy.schema = 1;
        assert!(parse_fallback_manifest(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }

    #[test]
    fn disabled_or_unknown_policy_yields_no_fallback_response() {
        for cached in [policy("disabled", false), policy("", false)] {
            assert!(fallback_response(
                &cached,
                manifest(2026102903),
                FALLBACK_MANIFEST_URLS[0],
                KEY,
                "1.5.4",
                2026102901,
            )
            .is_none());
        }
    }

    #[test]
    fn newer_target_yields_response_that_passes_manifest_contract() {
        let response = fallback_response(
            &policy("auto_install", true),
            manifest(2026102903),
            FALLBACK_MANIFEST_URLS[1],
            KEY,
            "1.5.4",
            2026102901,
        )
        .unwrap();

        assert!(response.update_available);
        assert_eq!(response.mode, "auto_install");
        assert!(response.auto_install);
        assert_eq!(response.target_version, "1.5.4");
        assert_eq!(response.target_build_seq, 2026102903);
        assert_eq!(response.current_build_seq, 2026102901);
        assert_eq!(response.manifest_url, FALLBACK_MANIFEST_URLS[1]);
        assert_eq!(
            response.url,
            "https://download.yan.life/rustdesk/stable/t/a.msi"
        );
        let manifest = response.manifest.clone().unwrap();
        base::update::validate_manifest_contract(
            &response,
            &manifest,
            "1.5.4",
            2026102901,
            "rustdesk-yan",
            "sos",
            "stable",
            KEY,
        )
        .unwrap();
    }

    #[test]
    fn same_or_older_target_reports_no_update() {
        for current in [2026102903, 2026103001] {
            let response = fallback_response(
                &policy("download", false),
                manifest(2026102903),
                FALLBACK_MANIFEST_URLS[0],
                KEY,
                "1.5.4",
                current,
            )
            .unwrap();
            assert!(!response.update_available);
        }
    }

    async fn connect_error() -> hbb_common::anyhow::Error {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        reqwest::Client::new()
            .get(format!("http://{address}/rd/update/v1/check"))
            .send()
            .await
            .unwrap_err()
            .into()
    }

    #[tokio::test]
    async fn real_transport_error_is_unreachable_but_parse_error_is_not() {
        assert!(backend_unreachable(&connect_error().await));
        let parse_error: hbb_common::anyhow::Error =
            serde_json::from_slice::<VersionCheckResponse>(b"<html>")
                .unwrap_err()
                .into();
        assert!(!backend_unreachable(&parse_error));
    }

    async fn status_error(status: u16) -> hbb_common::anyhow::Error {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        let err = reqwest::Client::new()
            .get(format!("http://{address}/rd/update/v1/check"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap_err()
            .into();
        server.join().unwrap();
        err
    }

    #[tokio::test]
    async fn real_server_errors_are_unreachable_but_rejections_are_not() {
        assert!(backend_unreachable(&status_error(503).await));
        assert!(backend_unreachable(&status_error(502).await));
        assert!(!backend_unreachable(&status_error(403).await));
        assert!(!backend_unreachable(&status_error(401).await));
    }

    #[tokio::test]
    async fn local_request_build_error_is_not_unreachable() {
        let err: hbb_common::anyhow::Error = reqwest::Client::new()
            .get("not a url")
            .send()
            .await
            .unwrap_err()
            .into();
        assert!(!backend_unreachable(&err));
    }

    #[tokio::test]
    async fn command_check_keeps_backend_error_without_fallback() {
        let err = connect_error().await;
        let message = err.to_string();
        let result = resolve_check_response(Err(err), "command_install", KEY).await;
        assert_eq!(result.unwrap_err().to_string(), message);
    }

    #[tokio::test]
    async fn malformed_backend_reply_keeps_backend_error_without_fallback() {
        let err: hbb_common::anyhow::Error =
            serde_json::from_slice::<VersionCheckResponse>(b"<html>")
                .unwrap_err()
                .into();
        let message = err.to_string();
        let result = resolve_check_response(Err(err), "scheduled", KEY).await;
        assert_eq!(result.unwrap_err().to_string(), message);
    }

    #[test]
    fn missing_target_yields_no_fallback_response() {
        assert!(fallback_response(
            &policy("download", false),
            manifest(2026102903),
            FALLBACK_MANIFEST_URLS[0],
            "macos-aarch64-dmg-sos",
            "1.5.4",
            2026102901,
        )
        .is_none());
    }
}
