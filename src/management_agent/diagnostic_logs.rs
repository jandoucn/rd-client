//! Bounded, redacted retrieval of fixed-location logs for `diagnostics.logs`.
//!
//! The agent runs as SYSTEM, so a remote caller must never be able to name a
//! path. `source` selects one fixed location, `transaction_id` selects one
//! journal transaction directory, and everything else is refused. Parsing,
//! path mapping, tail extraction and redaction are pure so they can be tested
//! without a live agent.

use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const MAX_BYTES: u64 = 32 * 1024;
pub const MAX_LINES: usize = 400;
pub const REDACTED: &str = "[redacted]";
const TOKEN_RUN: usize = 32;

const SENSITIVE: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "private",
    "key_pair",
    "signature",
    "enc_id",
    "authorization",
    "cookie",
    "bearer",
    "api_key",
    "access_key",
    "otp",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Agent,
    Server,
    Service,
    Update,
    SilentInstall,
    Install,
    MsiUpdate,
    RecoveryMsi,
    Transaction,
}

impl Source {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "agent" => Self::Agent,
            "server" => Self::Server,
            "service" => Self::Service,
            "update" => Self::Update,
            "silent-install" => Self::SilentInstall,
            "install" => Self::Install,
            "msi-update" => Self::MsiUpdate,
            "recovery-msi" => Self::RecoveryMsi,
            "transaction" => Self::Transaction,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Server => "server",
            Self::Service => "service",
            Self::Update => "update",
            Self::SilentInstall => "silent-install",
            Self::Install => "install",
            Self::MsiUpdate => "msi-update",
            Self::RecoveryMsi => "recovery-msi",
            Self::Transaction => "transaction",
        }
    }

    fn log_subdirectory(self) -> Option<&'static str> {
        match self {
            Self::Agent => Some("management-agent"),
            Self::Server => Some("server"),
            Self::Service => Some("service"),
            Self::Update => Some("update"),
            Self::SilentInstall => Some("silent-install"),
            Self::Install => Some("install"),
            Self::MsiUpdate | Self::RecoveryMsi | Self::Transaction => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub source: Source,
    pub transaction_id: Option<String>,
}

pub fn parse_args(args: &serde_json::Value) -> Result<Request, &'static str> {
    let object = args.as_object().ok_or("log arguments must be an object")?;
    for key in object.keys() {
        if !matches!(key.as_str(), "source" | "transaction_id") {
            return Err("unknown log argument");
        }
    }
    let source = object
        .get("source")
        .ok_or("log source absent")?
        .as_str()
        .ok_or("log source must be a string")?;
    let source = Source::parse(source).ok_or("log source unsupported")?;
    let transaction_id = object.get("transaction_id");
    if source == Source::Transaction {
        let id = transaction_id
            .ok_or("transaction id absent")?
            .as_str()
            .ok_or("transaction id must be a string")?;
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("transaction id invalid");
        }
        return Ok(Request {
            source,
            transaction_id: Some(id.to_owned()),
        });
    }
    if transaction_id.is_some() {
        return Err("transaction id not allowed for this source");
    }
    Ok(Request {
        source,
        transaction_id: None,
    })
}

/// Fixed agent-side locations: the agent's own log directory and the
/// transaction workspace root.
pub struct AgentLocations<'a> {
    pub logs: &'a Path,
    pub transactions: &'a Path,
}

/// Fixed path for a request. Pure: no filesystem access, no caller-controlled
/// path component beyond a validated 32-character transaction id.
pub fn resolve(log_root: &Path, agent: &AgentLocations, request: &Request) -> PathBuf {
    let transactions = agent.transactions;
    match request.source {
        Source::MsiUpdate => log_root.join("msi-update").join("msi-update.log"),
        Source::RecoveryMsi => agent.logs.join("recovery-msi.log"),
        Source::Transaction => transactions
            .join(request.transaction_id.as_deref().unwrap_or_default())
            .join("installer.log"),
        other => log_root.join(other.log_subdirectory().unwrap_or_default()),
    }
}

pub struct Tail {
    pub size: u64,
    pub modified_at: u64,
    pub truncated: bool,
    pub lines: Vec<String>,
}

/// Tail of a regular file, bounded by `MAX_BYTES` and `MAX_LINES`. Symlinks and
/// reparse points are refused so a fixed location cannot be redirected.
pub fn read_tail(path: &Path) -> Result<Tail, &'static str> {
    refuse_redirects(path)?;
    let mut file = open_shared(path)?;
    let metadata = file.metadata().map_err(|_| "log_unreadable")?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err("log_path_refused");
    }
    let size = metadata.len();
    let mut head = [0u8; 2];
    let utf16 = size >= 2 && file.read_exact(&mut head).is_ok() && head == [0xff, 0xfe];
    let mut start = size.saturating_sub(MAX_BYTES);
    if utf16 && start % 2 == 1 {
        start += 1;
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|_| "log_unreadable")?;
    let mut buffer = Vec::new();
    file.take(MAX_BYTES)
        .read_to_end(&mut buffer)
        .map_err(|_| "log_unreadable")?;
    let text = decode(&buffer, utf16, start == 0);
    let (lines, line_truncated) = tail_lines(&text, start > 0);
    Ok(Tail {
        size,
        modified_at: modified_at(&metadata),
        truncated: start > 0 || line_truncated,
        lines,
    })
}

#[cfg(windows)]
fn open_shared(path: &Path) -> Result<std::fs::File, &'static str> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x7;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "log_unreadable")
}

#[cfg(not(windows))]
fn open_shared(path: &Path) -> Result<std::fs::File, &'static str> {
    std::fs::File::open(path).map_err(|_| "log_unreadable")
}

/// Windows Installer writes verbose logs as UTF-16LE with a BOM; agent and
/// client logs are UTF-8. Both decode lossily.
pub fn decode(bytes: &[u8], utf16: bool, at_start: bool) -> String {
    let text = if utf16 {
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    if at_start {
        if let Some(stripped) = text.strip_prefix('\u{feff}') {
            return stripped.to_owned();
        }
    }
    text
}

fn refuse_redirects(path: &Path) -> Result<(), &'static str> {
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "log_unreadable")?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err("log_path_refused");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn modified_at(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Splits the tail window into lines. `partial` drops the first line when the
/// window started in the middle of a line.
pub fn tail_lines(text: &str, partial: bool) -> (Vec<String>, bool) {
    let text = if partial {
        match text.find('\n') {
            Some(index) => &text[index + 1..],
            None => text,
        }
    } else {
        text
    };
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect::<Vec<_>>();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let truncated = lines.len() > MAX_LINES;
    if truncated {
        lines.drain(..lines.len() - MAX_LINES);
    }
    (lines, truncated)
}

/// Keeps the newest lines whose JSON encoding fits `MAX_BYTES`, so escaping
/// can never push the journaled event past its size limit.
pub fn fit_budget(lines: Vec<String>) -> (Vec<String>, bool) {
    let budget = MAX_BYTES as usize;
    let mut total = 2usize;
    let mut keep = 0usize;
    for line in lines.iter().rev() {
        let size = encoded_len(line) + 1;
        if total.saturating_add(size) > budget {
            break;
        }
        total += size;
        keep += 1;
    }
    if keep == lines.len() {
        return (lines, false);
    }
    let mut kept = lines[lines.len() - keep..].to_vec();
    if kept.is_empty() {
        if let Some(last) = lines.last() {
            kept.push(tail_within(last, budget - 2));
        }
    }
    (kept, true)
}

fn encoded_len(line: &str) -> usize {
    2 + line.chars().map(escaped_len).sum::<usize>()
}

fn escaped_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        character if (character as u32) < 0x20 => 6,
        character => character.len_utf8(),
    }
}

fn tail_within(line: &str, budget: usize) -> String {
    let mut used = 2usize;
    let mut begin = line.len();
    for (index, character) in line.char_indices().rev() {
        let cost = escaped_len(character);
        if used + cost > budget {
            break;
        }
        used += cost;
        begin = index;
    }
    line[begin..].to_owned()
}

pub fn redact_line(line: &str) -> Option<String> {
    let lowered = line.to_ascii_lowercase();
    if SENSITIVE.iter().any(|pattern| lowered.contains(pattern)) {
        return None;
    }
    Some(mask_tokens(line))
}

fn mask_tokens(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut output = String::with_capacity(line.len());
    let mut index = 0;
    while index < bytes.len() {
        if token_byte(bytes[index]) {
            let start = index;
            while index < bytes.len() && token_byte(bytes[index]) {
                index += 1;
            }
            if looks_like_secret(&bytes[start..index]) {
                while index < bytes.len() && bytes[index] == b'=' {
                    index += 1;
                }
                output.push_str(REDACTED);
            } else {
                output.push_str(&line[start..index]);
            }
        } else {
            let character = line[index..].chars().next().unwrap_or('\u{fffd}');
            output.push(character);
            index += character.len_utf8();
        }
    }
    output
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/')
}

fn looks_like_secret(run: &[u8]) -> bool {
    run.len() >= TOKEN_RUN
}

pub fn present(
    source: Source,
    file: &str,
    modified_at: u64,
    size: u64,
    truncated: bool,
    lines: Vec<String>,
) -> serde_json::Value {
    let mut redacted_lines = 0usize;
    let lines = lines
        .into_iter()
        .map(|line| match redact_line(&line) {
            Some(masked) => {
                if masked != line {
                    redacted_lines += 1;
                }
                masked
            }
            None => {
                redacted_lines += 1;
                REDACTED.to_owned()
            }
        })
        .collect::<Vec<_>>();
    let (lines, trimmed) = fit_budget(lines);
    serde_json::json!({
        "schema": 1,
        "source": source.name(),
        "file": file,
        "modified_at": modified_at,
        "size": size,
        "truncated": truncated || trimmed,
        "lines": lines,
        "redacted_lines": redacted_lines,
    })
}

pub fn missing(source: Source) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "source": source.name(),
        "file": null,
        "lines": [],
        "missing": true,
    })
}

fn exists(path: &Path) -> Result<bool, &'static str> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("log_unreadable"),
    }
}

fn latest_log(directory: &Path) -> Result<Option<PathBuf>, &'static str> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("log_unreadable"),
    };
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|_| "log_unreadable")?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("log") {
            continue;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("log_unreadable"),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
            continue;
        }
        let modified = metadata.modified().map_err(|_| "log_unreadable")?;
        if latest
            .as_ref()
            .map(|(current, current_path)| {
                modified > *current || (modified == *current && path > *current_path)
            })
            .unwrap_or(true)
        {
            latest = Some((modified, path));
        }
    }
    Ok(latest.map(|(_, path)| path))
}

fn locate(
    log_root: &Path,
    agent: &AgentLocations,
    request: &Request,
) -> Result<Option<PathBuf>, &'static str> {
    let path = resolve(log_root, agent, request);
    if request.source == Source::Transaction {
        let directory = path.parent().ok_or("log_unreadable")?;
        if !exists(directory)? {
            return Ok(None);
        }
        super::workspace::verify(directory).map_err(|_| "log_path_refused")?;
        return Ok(if exists(&path)? { Some(path) } else { None });
    }
    if matches!(request.source, Source::MsiUpdate | Source::RecoveryMsi) {
        return Ok(if exists(&path)? { Some(path) } else { None });
    }
    latest_log(&path)
}

pub fn execute_in(
    args: &serde_json::Value,
    log_root: &Path,
    agent: &AgentLocations,
) -> Result<serde_json::Value, &'static str> {
    let request = parse_args(args).map_err(|_| "log_parameters_invalid")?;
    if !log_root.is_absolute() || !agent.logs.is_absolute() || !agent.transactions.is_absolute() {
        return Err("log_unreadable");
    }
    let Some(path) = locate(log_root, agent, &request)? else {
        return Ok(missing(request.source));
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("log_unreadable")?;
    let tail = read_tail(&path)?;
    Ok(present(
        request.source,
        &name,
        tail.modified_at,
        tail.size,
        tail.truncated,
        tail.lines,
    ))
}

pub fn execute(
    args: &serde_json::Value,
    paths: &super::AgentPaths,
) -> Result<serde_json::Value, &'static str> {
    let log_root = hbb_common::config::Config::log_path();
    let transactions = paths.agent.join("transactions");
    let agent = AgentLocations {
        logs: &paths.logs,
        transactions: &transactions,
    };
    execute_in(args, &log_root, &agent)
}

#[cfg(test)]
#[path = "diagnostic_logs_tests.rs"]
mod tests;
