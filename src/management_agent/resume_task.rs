use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
};

fn task_name(id: &str) -> Result<String, String> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("resume task transaction ID invalid".into());
    }
    Ok(format!("RustDeskManagementAgentResume-{id}"))
}

fn tool() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32/schtasks.exe")
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn element<'a>(source: &'a str, name: &str) -> Result<&'a str, String> {
    let opening = format!("<{name}>");
    let closing = format!("</{name}>");
    if source.matches(&opening).count() != 1 || source.matches(&closing).count() != 1 {
        return Err(format!("resume task {name} ambiguous or missing"));
    }
    let start = source.find(&opening).ok_or("resume task opening missing")? + opening.len();
    let end = source[start..]
        .find(&closing)
        .ok_or("resume task closing missing")?
        + start;
    Ok(source[start..end].trim())
}
fn enabled(source: &str) -> Result<bool, String> {
    if !source.contains("<Enabled") && !source.contains("</Enabled>") {
        Ok(true)
    } else {
        Ok(element(source, "Enabled")? == "true")
    }
}

pub fn definition(helper: &Path, id: &str) -> Result<String, String> {
    task_name(id)?;
    let path = helper
        .to_str()
        .ok_or("resume helper path encoding invalid")?;
    if !helper.is_absolute() || path.contains('"') || path.chars().any(char::is_control) {
        return Err("resume helper path invalid".into());
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger><TimeTrigger><Repetition><Interval>PT5M</Interval><Duration>P1D</Duration><StopAtDurationEnd>true</StopAtDurationEnd></Repetition><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
<Principals><Principal id="System"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT15M</ExecutionTimeLimit><Enabled>true</Enabled></Settings>
<Actions Context="System"><Exec><Command>{}</Command><Arguments>resume-install --transaction-id {id}</Arguments></Exec></Actions></Task>"#,
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        xml(path)
    ))
}

pub fn verify(source: &str, helper: &Path, id: &str) -> Result<(), String> {
    if source.matches("<BootTrigger").count() != 1 {
        return Err("resume task boot trigger missing or duplicated".into());
    }
    let normalized = source
        .replace("<BootTrigger />", "<BootTrigger></BootTrigger>")
        .replace("<BootTrigger/>", "<BootTrigger></BootTrigger>");
    let source = normalized.as_str();
    let expected = definition(helper, id)?;
    if source.contains("<!")
        || source.matches("<Exec>").count()
            + source.matches("<Exec/").count()
            + source.matches("<Exec ").count()
            != 1
        || source.matches("<Principal ").count() != 1
        || source.matches("<Actions ").count() != 1
        || !source.contains("<Actions Context=\"System\">")
    {
        return Err("resume task action/principal invalid".into());
    }
    for name in [
        "Command",
        "Arguments",
        "UserId",
        "RunLevel",
        "MultipleInstancesPolicy",
        "ExecutionTimeLimit",
    ] {
        if element(source, name)?.replace("&quot;", "\"")
            != element(&expected, name)?.replace("&quot;", "\"")
        {
            return Err(format!("resume task {name} mismatch"));
        }
    }
    let settings = element(source, "Settings")?;
    if !enabled(settings)?
        || !enabled(element(source, "BootTrigger")?)?
        || !enabled(element(source, "TimeTrigger")?)?
    {
        return Err("resume task disabled".into());
    }
    for name in [
        "DisallowStartIfOnBatteries",
        "StopIfGoingOnBatteries",
        "StartWhenAvailable",
    ] {
        if element(settings, name)? != element(element(&expected, "Settings")?, name)? {
            return Err(format!("resume task setting {name} mismatch"));
        }
    }
    let repetition = element(source, "Repetition")?;
    if element(repetition, "Interval")? != "PT5M"
        || element(repetition, "Duration")? != "P1D"
        || element(repetition, "StopAtDurationEnd")? != "true"
    {
        return Err("resume task retry contract mismatch".into());
    }
    Ok(())
}

pub fn register_definition(root: &Path, id: &str, definition: &str) -> Result<(), String> {
    let name = task_name(id)?;
    let path = root.join("resume-task.xml");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut bytes = vec![0xff, 0xfe];
    for unit in definition.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let output = Command::new(tool())
        .args(["/Create", "/TN", &name, "/XML"])
        .arg(&path)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "resume task registration failed: {}",
            output.status
        ));
    }
    Ok(())
}

pub fn read_back(id: &str) -> Result<String, String> {
    let name = task_name(id)?;
    let output = Command::new(tool())
        .args(["/Query", "/TN", &name, "/XML"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("resume task readback failed: {}", output.status));
    }
    let bytes = &output.stdout;
    if bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() > 1 && bytes[1] == 0) {
        let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        Ok(String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        ))
    } else {
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub fn remove(id: &str) -> Result<(), String> {
    let name = task_name(id)?;
    let output = Command::new(tool())
        .args(["/Delete", "/TN", &name, "/F"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("resume task removal failed: {}", output.status));
    }
    Ok(())
}
