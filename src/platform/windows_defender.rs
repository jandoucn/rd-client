use hbb_common::{
    anyhow::anyhow,
    base64::{engine::general_purpose::STANDARD, Engine as _},
    ResultType,
};

#[path = "defender_policy.rs"]
pub mod policy;

pub fn configuration_commands(main: &str, options: &str) -> ResultType<String> {
    let choice = policy::choice(options).map_err(|error| anyhow!(error))?;
    if main.contains('\0') || main.chars().any(char::is_control) {
        return Err(anyhow!("Defender executable path invalid"));
    }
    let literal = main.replace('\'', "''");
    let script=format!("{}\ntry {{$agent=Join-Path $env:ProgramData 'RustDesk\\ManagementAgent\\rustdesk-management-agent.exe'; $result=Invoke-RdDefenderConfiguration -Choice '{choice}' -MainPath '{literal}' -AgentPath $agent; $result | ConvertTo-Json -Depth 5 -Compress | Write-Output; if($result.Status -notin @('verified','not_selected')){{Write-Warning ('Defender 未配置成功：'+$result.Status+' '+$result.Error)}}}}catch{{Write-Warning ('Defender 配置失败：'+$_.Exception.Message)}}; exit 0",include_str!("../../res/defender-exclusions.ps1"));
    Ok(embedded_commands(&script))
}

fn embedded_commands(script: &str) -> String {
    embedded_commands_with_timeout(script, 60000)
}

fn embedded_commands_with_timeout(script: &str, timeout_ms: u32) -> String {
    // The full EncodedCommand would exceed cmd.exe's 8191-character limit.
    // Stage only the embedded script beside the verified elevated installer BAT.
    let encoded = STANDARD.encode([&[0xef, 0xbb, 0xbf][..], script.as_bytes()].concat());
    let mut commands = String::new();
    for (index, chunk) in encoded.as_bytes().chunks(120).enumerate() {
        let redirect = if index == 0 { ">" } else { ">>" };
        commands.push_str(&format!(
            "{redirect} \"%~f0.defender.b64\" echo {}\r\n",
            String::from_utf8_lossy(chunk)
        ));
    }
    commands.push_str("certutil -f -decode \"%~f0.defender.b64\" \"%~f0.defender.ps1\" >nul\r\nif errorlevel 1 (echo Defender configuration script preparation failed.) else (\r\nset \"RUSTDESK_DEFENDER_SCRIPT=%~f0.defender.ps1\"\r\n");
    // Paths remain environment data, never interpolated into PowerShell syntax.
    let supervisor = r#"try {$ps=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe';$args='-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$env:RUSTDESK_DEFENDER_SCRIPT+'"';$p=Start-Process -FilePath $ps -ArgumentList $args -PassThru -NoNewWindow;if(!$p.WaitForExit(60000)){try{$p.Kill();$p.WaitForExit(5000)|Out-Null}catch{};Write-Warning 'Defender configuration timed out; unverified.'}}catch{Write-Warning ('Defender helper unavailable: '+$_.Exception.Message)};exit 0"#;
    let supervisor =
        supervisor.replace("WaitForExit(60000)", &format!("WaitForExit({timeout_ms})"));
    let encoded = STANDARD.encode(
        supervisor
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    commands.push_str(&format!("\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -EncodedCommand {encoded}\r\n)\r\nver >nul\r\n"));
    commands
}

pub fn saved_choice() -> Option<String> {
    use winreg::{
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
        RegKey,
    };
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags("SOFTWARE\\RustDesk\\Defender", KEY_READ | KEY_WOW64_64KEY)
        .ok()
        .and_then(|key| key.get_value::<String, _>("Consent").ok())
        .filter(|value| value == "0" || value == "1")
}

pub fn defender_install_option(arguments: &[String]) -> Result<&'static str, &'static str> {
    policy::silent_option(arguments.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_timeout_is_bounded_and_installation_continues() {
        let directory =
            std::env::temp_dir().join(format!("rustdesk-defender-timeout-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let marker = directory.join("continued.txt");
        let script = directory.join("fixture.bat");
        let child = directory.join("late.txt");
        let payload = format!(
            "Start-Sleep -Seconds 30; [IO.File]::WriteAllText('{}','late')",
            child.to_string_lossy().replace('\'', "''")
        );
        let batch = format!(
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\n{}\r\n> \"{}\" echo continued\r\n",
            embedded_commands_with_timeout(&payload, 250),
            marker.display()
        );
        std::fs::write(&script, batch).unwrap();
        let started = std::time::Instant::now();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C"])
            .arg(&script)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(15));
        assert!(marker.is_file());
        assert!(!child.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn embedded_configuration_uses_final_path_and_short_cmd_lines() {
        let main = r"D:\自定义 Folder\RustDesk.exe";
        let commands = configuration_commands(main, "defender=1").unwrap();
        assert!(commands.contains("%~f0.defender.ps1"));
        assert!(commands.contains("-EncodedCommand"));
        assert!(commands.contains("RUSTDESK_DEFENDER_SCRIPT="));
        let mut encoded = String::new();
        for line in commands.lines() {
            assert!(line.len() < 8191);
            if line.contains(".defender.b64\" echo ") {
                encoded.push_str(line.split_once(" echo ").unwrap().1);
            }
        }
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(&bytes[..3], &[0xef, 0xbb, 0xbf]);
        let script = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(script.contains(main));
        assert!(script.contains("-Choice '1'"));
        assert!(script.contains("ManagementAgent\\rustdesk-management-agent.exe"));
        assert!(!script.contains("Remove-MpPreference"));
    }

    #[test]
    fn embedded_command_preserves_apostrophes_and_inherits_without_opt_in() {
        let commands = configuration_commands(r"D:\Admin's Folder\RustDesk.exe", "").unwrap();
        let encoded = commands
            .lines()
            .filter(|line| line.contains(".defender.b64\" echo "))
            .filter_map(|line| line.split_once(" echo ").map(|parts| parts.1))
            .collect::<String>();
        let bytes = STANDARD.decode(encoded).unwrap();
        let script = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(script.contains("Admin''s Folder"));
        assert!(script.contains("-Choice 'inherit'"));
        assert!(configuration_commands("bad\npath", "defender=1").is_err());
        assert!(configuration_commands(r"D:\RustDesk.exe", "defender=bad").is_err());
    }

    #[test]
    fn actual_cmd_staging_roundtrips_unicode_and_continues_on_helper_failure() {
        let directory =
            std::env::temp_dir().join(format!("rustdesk-defender-cmd-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let marker = directory.join("continued.txt");
        let script = directory.join("fixture.bat");
        // No Defender or registry IO: only verifies the installer transport and
        // the nonfatal PowerShell failure boundary in the real command shell.
        let payload="$ErrorActionPreference='Stop'; $expected='D:\\Custom 中文 Folder\\RustDesk.exe'; if($expected -notmatch '中文'){throw 'encoding lost'}; exit 7";
        let batch = format!(
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\n{}\r\n> \"{}\" echo continued\r\n",
            embedded_commands(payload),
            marker.display()
        );
        std::fs::write(&script, batch).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C"])
            .arg(&script)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert!(marker.is_file());
        let decoded = std::fs::read(script.with_extension("bat.defender.ps1")).unwrap();
        assert_eq!(&decoded[..3], &[0xef, 0xbb, 0xbf]);
        assert_eq!(std::str::from_utf8(&decoded[3..]).unwrap(), payload);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
