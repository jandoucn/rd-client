use super::*;

fn fixture(name: &str) -> PathBuf {
    let base = if cfg!(windows) {
        std::env::temp_dir()
    } else {
        std::env::temp_dir().canonicalize().unwrap()
    };
    let root = base.join(format!("agent-logs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn run(
    args: &serde_json::Value,
    logs: &Path,
    transactions: &Path,
) -> Result<serde_json::Value, &'static str> {
    let agent_logs = transactions.with_file_name("agent-logs");
    let agent = AgentLocations {
        logs: &agent_logs,
        transactions,
    };
    execute_in(args, logs, &agent)
}

#[test]
fn arguments_admit_each_fixed_source_and_refuse_everything_else() {
    for source in [
        "agent",
        "server",
        "service",
        "update",
        "silent-install",
        "install",
        "msi-update",
        "recovery-msi",
    ] {
        let args = serde_json::json!({"source": source});
        assert_eq!(parse_args(&args).unwrap().source.name(), source);
    }
    let id = "0123456789abcdef0123456789abcdef";
    let request =
        parse_args(&serde_json::json!({"source":"transaction","transaction_id":id})).unwrap();
    assert_eq!(request.source, Source::Transaction);
    assert_eq!(request.transaction_id.as_deref(), Some(id));
}

#[test]
fn argument_failures_are_refused_before_any_path_is_built() {
    for (args, reason) in [
        (serde_json::json!({}), "log source absent"),
        (serde_json::json!(null), "log arguments must be an object"),
        (serde_json::json!([]), "log arguments must be an object"),
        (
            serde_json::json!({"source":"agent","extra":1}),
            "unknown log argument",
        ),
        (
            serde_json::json!({"source":1}),
            "log source must be a string",
        ),
        (
            serde_json::json!({"source":"System"}),
            "log source unsupported",
        ),
        (
            serde_json::json!({"source":"agent","transaction_id":"0123456789abcdef0123456789abcdef"}),
            "transaction id not allowed for this source",
        ),
        (
            serde_json::json!({"source":"transaction"}),
            "transaction id absent",
        ),
        (
            serde_json::json!({"source":"transaction","transaction_id":7}),
            "transaction id must be a string",
        ),
        (
            serde_json::json!({"source":"transaction","transaction_id":"0123456789ABCDEF0123456789ABCDEF"}),
            "transaction id invalid",
        ),
        (
            serde_json::json!({"source":"transaction","transaction_id":"0123456789abcdef0123456789abcde"}),
            "transaction id invalid",
        ),
        (
            serde_json::json!({"source":"transaction","transaction_id":"../../../windows/system32"}),
            "transaction id invalid",
        ),
        (
            serde_json::json!({"source":"transaction","transaction_id":"0123456789abcdef0123456789abcdeg"}),
            "transaction id invalid",
        ),
    ] {
        assert_eq!(parse_args(&args), Err(reason), "{args}");
    }
}

#[test]
fn sources_map_to_fixed_locations_only() {
    let root = Path::new(r"C:\logs");
    let transactions = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions");
    let agent_logs = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\logs");
    let agent = AgentLocations {
        logs: agent_logs,
        transactions,
    };
    let cases = [
        ("agent", "management-agent"),
        ("server", "server"),
        ("service", "service"),
        ("update", "update"),
        ("silent-install", "silent-install"),
        ("install", "install"),
    ];
    for (source, expected) in cases {
        let request = Request {
            source: Source::parse(source).unwrap(),
            transaction_id: None,
        };
        assert_eq!(
            resolve(root, &agent, &request),
            root.join(expected),
            "{source}"
        );
    }
    let msi = Request {
        source: Source::MsiUpdate,
        transaction_id: None,
    };
    assert_eq!(
        resolve(root, &agent, &msi),
        root.join("msi-update").join("msi-update.log")
    );
    let recovery = Request {
        source: Source::RecoveryMsi,
        transaction_id: None,
    };
    assert_eq!(
        resolve(root, &agent, &recovery),
        agent_logs.join("recovery-msi.log")
    );
    let transaction = Request {
        source: Source::Transaction,
        transaction_id: Some("0123456789abcdef0123456789abcdef".into()),
    };
    assert_eq!(
        resolve(root, &agent, &transaction),
        transactions
            .join("0123456789abcdef0123456789abcdef")
            .join("installer.log")
    );
}

#[test]
fn tail_keeps_the_last_lines_within_both_bounds() {
    assert_eq!(
        tail_lines("one\ntwo\r\nthree\n", false).0,
        vec!["one", "two", "three"]
    );
    let many = (0..MAX_LINES + 25)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (lines, truncated) = tail_lines(&many, false);
    assert!(truncated);
    assert_eq!(lines.len(), MAX_LINES);
    assert_eq!(lines.first().unwrap(), "line-25");
    assert_eq!(lines.last().unwrap(), &format!("line-{}", MAX_LINES + 24));
    let exact = (25..MAX_LINES + 25)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (exact, truncated) = tail_lines(&exact, false);
    assert_eq!(exact.len(), MAX_LINES);
    assert!(!truncated);
    assert_eq!(
        tail_lines("garbage-without-newline\nkept\n", true).0,
        vec!["kept"]
    );
    assert_eq!(tail_lines("one long tail", true).0, vec!["one long tail"]);
    assert!(tail_lines("", false).0.is_empty());
}

#[test]
fn decoding_handles_utf8_lossy_and_msi_utf16() {
    assert_eq!(decode(b"bad \xff utf8", false, true), "bad \u{fffd} utf8");
    assert_eq!(decode("\u{feff}bom".as_bytes(), false, true), "bom");
    let utf16 = [0xff, 0xfe, b'M', 0, b'S', 0, b'I', 0, b'\n', 0, 0x2d, 0x4e];
    assert_eq!(decode(&utf16, true, true), "MSI\n中");
    assert_eq!(decode(&utf16[2..], true, false), "MSI\n中");
    assert_eq!(decode(&[0x00, 0xd8, b'a', 0], true, false), "\u{fffd}a");
}

#[test]
fn budget_keeps_newest_lines_and_bounds_escaped_size() {
    let lines = vec!["a".repeat(20_000), "b".repeat(20_000)];
    let (kept, trimmed) = fit_budget(lines);
    assert!(trimmed);
    assert_eq!(kept, vec!["b".repeat(20_000)]);
    let control = "\u{1}".repeat(MAX_BYTES as usize);
    let (kept, trimmed) = fit_budget(vec![control]);
    assert!(trimmed);
    assert!(serde_json::to_string(&kept).unwrap().len() <= MAX_BYTES as usize);
    assert!(!kept[0].is_empty());
    let small = vec!["x".to_owned(), "y".to_owned()];
    assert_eq!(fit_budget(small.clone()), (small, false));
    let quotes = (0..MAX_LINES).map(|_| "\"".repeat(80)).collect::<Vec<_>>();
    let (kept, trimmed) = fit_budget(quotes);
    assert!(trimmed);
    assert!(serde_json::to_string(&kept).unwrap().len() <= MAX_BYTES as usize);
}

#[test]
fn read_tail_bounds_the_window_and_reports_metadata() {
    let root = fixture("read");
    let path = root.join("agent.log");
    let mut contents = String::new();
    for index in 0..500u32 {
        contents.push_str(&format!("entry-{index}-{}-end\n", "x".repeat(80)));
    }
    contents.push_str("needle at the end\n");
    std::fs::write(&path, &contents).unwrap();
    let tail = read_tail(&path).unwrap();
    assert_eq!(tail.size, contents.len() as u64);
    assert!(tail.truncated);
    assert!(tail.lines.len() <= MAX_LINES);
    assert_eq!(tail.lines.last().unwrap(), "needle at the end");
    assert!(
        tail.lines.iter().all(|line| !line.starts_with("entry-0-")),
        "window kept the head"
    );
    assert!(tail.modified_at > 0);

    let msi = root.join("installer.log");
    let mut wide = vec![0xff, 0xfe];
    for index in 0..30_000u32 {
        for unit in format!("msi-{index}\r\n").encode_utf16() {
            wide.extend_from_slice(&unit.to_le_bytes());
        }
    }
    std::fs::write(&msi, &wide).unwrap();
    let tail = read_tail(&msi).unwrap();
    assert!(tail.truncated);
    assert_eq!(tail.lines.last().unwrap(), "msi-29999");
    assert!(
        tail.lines.iter().all(|line| line.starts_with("msi-")),
        "{:?}",
        tail.lines.first()
    );

    let exact = root.join("exact.log");
    let body = (0..MAX_BYTES as usize)
        .map(|index| b'a' + (index % 26) as u8)
        .collect::<Vec<_>>();
    std::fs::write(&exact, &body).unwrap();
    let tail = read_tail(&exact).unwrap();
    assert!(!tail.truncated);
    assert_eq!(tail.lines.len(), 1);
    assert_eq!(tail.lines[0].len(), MAX_BYTES as usize);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn redaction_replaces_keyword_lines_and_long_tokens() {
    for line in [
        "connect password=hunter2",
        "PASSWD file",
        "pwd C:\\x",
        "secret value",
        "token abc",
        "private key",
        "key_pair present",
        "signature sent",
        "enc_id 42",
        "Authorization: Bearer",
        "Cookie: a=b",
        "Bearer abc",
        "API_KEY=1",
        "access_key: x",
        "OTP 123456",
    ] {
        assert_eq!(redact_line(line), None, "{line}");
    }
    let key = "WAVL+YUYZ6EXOqLCGZEq56VD2LalZc121gtVeXxpp78=";
    assert_eq!(
        redact_line(&format!("pre {key} post")).unwrap(),
        "pre [redacted] post"
    );
    let words = "a".repeat(40);
    assert_eq!(redact_line(&words).unwrap(), "[redacted]");
    let path = "life/rd/update/v1/agent/commands/claim/receipt";
    assert_eq!(redact_line(path).unwrap(), "[redacted]");
    assert_eq!(
        redact_line("dGhpcyBpcyBhIHRlc3QgdmFsdWUgZm9yIGI2NA== end").unwrap(),
        "[redacted] end"
    );
    let hex = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        redact_line(&format!("sha256={hex}")).unwrap(),
        "sha256=[redacted]"
    );
    assert_eq!(
        redact_line(&hex.to_ascii_uppercase()).unwrap(),
        "[redacted]"
    );
    let short = "Ab1".repeat(10);
    assert_eq!(
        redact_line(&format!("pre {short} post")).unwrap(),
        format!("pre {short} post")
    );
    assert_eq!(redact_line("plain status ok").unwrap(), "plain status ok");
    assert_eq!(
        redact_line("路径 logs\\管理 ünïcode").unwrap(),
        "路径 logs\\管理 ünïcode"
    );
}

#[test]
fn present_counts_every_redacted_line_and_keeps_the_contract_shape() {
    let value = present(
        Source::Agent,
        "agent_rCURRENT.log",
        1700000000,
        4096,
        true,
        vec![
            "plain".into(),
            "token abc".into(),
            format!("hash {}", "0123456789abcdef".repeat(4)),
            "password".into(),
        ],
    );
    assert_eq!(value["schema"], 1);
    assert_eq!(value["source"], "agent");
    assert_eq!(value["file"], "agent_rCURRENT.log");
    assert_eq!(value["modified_at"], 1700000000);
    assert_eq!(value["size"], 4096);
    assert_eq!(value["truncated"], true);
    assert_eq!(
        value["lines"],
        serde_json::json!(["plain", "[redacted]", "hash [redacted]", "[redacted]"])
    );
    assert_eq!(value["redacted_lines"], 3);
    assert!(value.get("missing").is_none());
}

#[test]
fn missing_sources_succeed_with_an_empty_shape() {
    let value = missing(Source::Install);
    assert_eq!(
        value,
        serde_json::json!({
            "schema": 1, "source": "install", "file": null, "lines": [], "missing": true,
        })
    );
}

#[test]
fn execute_reads_the_latest_file_and_reports_absent_sources() {
    let root = fixture("execute");
    let logs = root.join("logs");
    let agent = logs.join("management-agent");
    std::fs::create_dir_all(&agent).unwrap();
    let older = agent.join("agent.rlog.2026-01-01.log");
    let newer = agent.join("agent.rlog.2026-01-02.log");
    std::fs::write(&older, b"old\n").unwrap();
    std::fs::write(&newer, b"new line\npassword=hidden\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&newer, b"new line\npassword=hidden\n").unwrap();
    let transactions = root.join("transactions");
    let value = run(&serde_json::json!({"source":"agent"}), &logs, &transactions).unwrap();
    assert_eq!(value["source"], "agent");
    assert_eq!(value["file"], "agent.rlog.2026-01-02.log");
    assert_eq!(
        value["lines"],
        serde_json::json!(["new line", "[redacted]"])
    );
    assert_eq!(value["redacted_lines"], 1);
    assert!(value["truncated"].is_boolean());

    let value = run(
        &serde_json::json!({"source":"service"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(value["missing"], true);
    let value = run(
        &serde_json::json!({"source":"msi-update"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(value["missing"], true);
    let value = run(
        &serde_json::json!({"source":"transaction","transaction_id":"0123456789abcdef0123456789abcdef"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(value["missing"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transaction_source_reads_only_inside_a_verified_directory() {
    let root = fixture("transaction");
    let logs = root.join("logs");
    let transactions = root.join("transactions");
    super::super::workspace::create(&transactions).unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let directory = transactions.join(id);
    super::super::workspace::create(&directory).unwrap();
    std::fs::write(
        directory.join("installer.log"),
        b"MSI started\nLogging stopped\n",
    )
    .unwrap();
    let value = run(
        &serde_json::json!({"source":"transaction","transaction_id":id}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(value["source"], "transaction");
    assert_eq!(value["file"], "installer.log");
    assert_eq!(
        value["lines"],
        serde_json::json!(["MSI started", "Logging stopped"])
    );

    let other = "ffffffffffffffffffffffffffffffff";
    std::fs::create_dir_all(transactions.join(other)).unwrap();
    std::fs::write(
        transactions.join(other).join("installer.log"),
        b"unofficial\n",
    )
    .unwrap();
    assert_eq!(
        run(
            &serde_json::json!({"source":"transaction","transaction_id":other}),
            &logs,
            &transactions,
        ),
        Err("log_path_refused")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn msi_update_source_reads_the_fixed_file() {
    let root = fixture("msi");
    let logs = root.join("logs");
    std::fs::create_dir_all(logs.join("update")).unwrap();
    std::fs::create_dir_all(logs.join("msi-update")).unwrap();
    std::fs::write(
        logs.join("msi-update").join("msi-update.log"),
        b"=== Verbose logging started ===\n",
    )
    .unwrap();
    std::fs::write(logs.join("update").join("newer.log"), b"other\n").unwrap();
    let value = run(
        &serde_json::json!({"source":"msi-update"}),
        &logs,
        &root.join("transactions"),
    )
    .unwrap();
    assert_eq!(value["file"], "msi-update.log");
    assert_eq!(
        value["lines"],
        serde_json::json!(["=== Verbose logging started ==="])
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_file(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).unwrap();
}

#[cfg(unix)]
fn link_directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_directory(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).unwrap();
}

#[test]
fn redirected_locations_are_refused_and_never_followed() {
    let root = fixture("redirect");
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"system secret\n").unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(logs.join("update")).unwrap();
    std::fs::create_dir_all(logs.join("msi-update")).unwrap();
    link_file(
        &outside.join("secret.txt"),
        &logs.join("msi-update").join("msi-update.log"),
    );
    let transactions = root.join("transactions");
    assert_eq!(
        run(
            &serde_json::json!({"source":"msi-update"}),
            &logs,
            &transactions
        ),
        Err("log_path_refused")
    );
    assert_eq!(
        read_tail(&logs.join("msi-update").join("msi-update.log")).err(),
        Some("log_path_refused")
    );

    link_directory(&outside, &logs.join("server"));
    std::fs::write(outside.join("redirected.log"), b"system secret\n").unwrap();
    assert_eq!(
        run(
            &serde_json::json!({"source":"server"}),
            &logs,
            &transactions
        ),
        Err("log_path_refused")
    );

    let agent = logs.join("management-agent");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(agent.join("real.log"), b"real\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    link_file(&outside.join("secret.txt"), &agent.join("zz-newer.log"));
    let value = run(&serde_json::json!({"source":"agent"}), &logs, &transactions).unwrap();
    assert_eq!(value["file"], "real.log");
    assert_eq!(value["lines"], serde_json::json!(["real"]));
    assert_eq!(
        std::fs::read(outside.join("secret.txt")).unwrap(),
        b"system secret\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn relative_log_roots_are_refused() {
    let root = fixture("relative");
    let transactions = root.join("transactions");
    let absolute = AgentLocations {
        logs: &root,
        transactions: &transactions,
    };
    assert_eq!(
        execute_in(
            &serde_json::json!({"source":"agent"}),
            Path::new("logs"),
            &absolute
        ),
        Err("log_unreadable")
    );
    let relative = AgentLocations {
        logs: Path::new("logs"),
        transactions: &transactions,
    };
    assert_eq!(
        execute_in(
            &serde_json::json!({"source":"recovery-msi"}),
            &root,
            &relative
        ),
        Err("log_unreadable")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn admission_and_execution_parse_the_same_argument_contract() {
    let id = "0123456789abcdef0123456789abcdef";
    for args in [
        serde_json::json!({"source":"agent"}),
        serde_json::json!({"source":"server"}),
        serde_json::json!({"source":"service"}),
        serde_json::json!({"source":"update"}),
        serde_json::json!({"source":"silent-install"}),
        serde_json::json!({"source":"install"}),
        serde_json::json!({"source":"msi-update"}),
        serde_json::json!({"source":"recovery-msi"}),
        serde_json::json!({"source":"recovery-msi","transaction_id":id}),
        serde_json::json!({"source":"transaction","transaction_id":id}),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!(null),
        serde_json::json!({"source":"agent","extra":true}),
        serde_json::json!({"source":"agent","transaction_id":id}),
        serde_json::json!({"source":"transaction"}),
        serde_json::json!({"source":"transaction","transaction_id":"ABCDEF0123456789abcdef0123456789"}),
        serde_json::json!({"source":"transaction","transaction_id":7}),
        serde_json::json!({"source":"Agent"}),
        serde_json::json!({"source":null}),
    ] {
        assert_eq!(
            super::super::operations::diagnostics_logs_args_valid(&args),
            parse_args(&args).is_ok(),
            "{args}"
        );
    }
}

#[test]
fn update_source_never_returns_the_msi_log() {
    let root = fixture("update-vs-msi");
    let logs = root.join("logs");
    std::fs::create_dir_all(logs.join("update")).unwrap();
    std::fs::create_dir_all(logs.join("msi-update")).unwrap();
    std::fs::write(
        logs.join("update").join("rustdesk_rCURRENT.log"),
        b"update run\n",
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        logs.join("msi-update").join("msi-update.log"),
        b"newest msi\n",
    )
    .unwrap();
    let transactions = root.join("transactions");
    let update = run(
        &serde_json::json!({"source":"update"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(update["file"], "rustdesk_rCURRENT.log");
    assert_eq!(update["lines"], serde_json::json!(["update run"]));
    let msi = run(
        &serde_json::json!({"source":"msi-update"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(msi["lines"], serde_json::json!(["newest msi"]));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_msi_source_reads_the_fixed_agent_log() {
    let root = fixture("recovery-msi");
    let logs = root.join("logs");
    let transactions = root.join("transactions");
    let agent_logs = root.join("agent-logs");
    assert_eq!(
        run(
            &serde_json::json!({"source":"recovery-msi"}),
            &logs,
            &transactions
        )
        .unwrap()["missing"],
        true
    );
    std::fs::create_dir_all(&agent_logs).unwrap();
    std::fs::write(agent_logs.join("recovery-msi.log"), b"recovery msi\n").unwrap();
    std::fs::write(agent_logs.join("other.log"), b"other\n").unwrap();
    let value = run(
        &serde_json::json!({"source":"recovery-msi"}),
        &logs,
        &transactions,
    )
    .unwrap();
    assert_eq!(value["source"], "recovery-msi");
    assert_eq!(value["file"], "recovery-msi.log");
    assert_eq!(value["lines"], serde_json::json!(["recovery msi"]));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ansi_msi_logs_decode_lossily_without_failing() {
    let root = fixture("ansi");
    let path = root.join("installer.log");
    let ansi = b"=== Logging started \xb2\xe2\xca\xd4 ===\r\nMSI (s) done\r\n";
    std::fs::write(&path, ansi).unwrap();
    let tail = read_tail(&path).unwrap();
    assert_eq!(tail.lines.len(), 2);
    assert!(tail.lines[0].starts_with("=== Logging started "));
    assert!(tail.lines[0].contains('\u{fffd}'));
    assert_eq!(tail.lines[1], "MSI (s) done");
    std::fs::remove_dir_all(root).unwrap();
}
