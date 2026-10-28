use super::{handoff::{ObservedProcess, Owner}, workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::{Read, Write}, path::{Path, PathBuf}, time::{Duration, Instant}};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u32,
    pub transaction_id: String,
    pub installation_id: String,
    pub product_code: String,
    pub helper_sha256: String,
    pub parent: Owner,
    pub nonce: String,
}

fn token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn directory(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !token(id) { return Err("purge handoff transaction invalid".into()); }
    Ok(root.join(id))
}

fn locked(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    workspace::verify_file(path)?;
    std::fs::OpenOptions::new().read(true).share_mode(1).open(path).map_err(|e| e.to_string())
}

fn digest(file: &mut File) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn save_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())
}

// Called only with identity obtained from Approved; no command/path is recorded.
pub(super) fn prepare(id: &str, installation: &str, product: &str) -> Result<PathBuf, String> {
    prepare_at(&super::system_dirs::get().program_data.join("RustDeskPurge"), id, installation, product)
}

fn prepare_at(root: &Path, id: &str, installation: &str, product: &str) -> Result<PathBuf, String> {
    let directory = directory(root, id)?;
    if !token(installation) || product.len() != 38 || uuid::Uuid::parse_str(product.trim_matches(['{','}'])).is_err() {
        return Err("purge handoff product identity invalid".into());
    }
    match std::fs::symlink_metadata(root) {
        Ok(_) => workspace::verify(root)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => workspace::create(root)?,
        Err(error) => return Err(format!("purge handoff root unknown: {error}")),
    }
    if directory.exists() { return Err("purge handoff occupied".into()); }
    workspace::create(&directory)?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut source = locked(&current)?;
    let hash = digest(&mut source)?;
    use std::io::{Seek, SeekFrom};
    source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let helper = directory.join("purge-helper.exe");
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&helper).map_err(|e| e.to_string())?;
    std::io::copy(&mut source, &mut output).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    if digest(&mut locked(&helper)?)? != hash { return Err("purge helper copy hash mismatch".into()); }
    let plan = Plan { schema: 1, transaction_id: id.into(), installation_id: installation.into(),
        product_code: product.into(), helper_sha256: hash,
        parent: ObservedProcess::open(std::process::id())?.owner().clone(),
        nonce: uuid::Uuid::new_v4().simple().to_string() };
    save_new(&directory.join("plan.json"), &plan)?;
    Ok(directory)
}

pub struct Checked {
    pub plan: Plan,
    directory: PathBuf,
    _plan: File,
    _image: File,
    owner: Owner,
    claimed: bool,
    plan_sha256: String,
}

pub(super) fn open(id: &str) -> Result<Checked, String> { open_at(&super::system_dirs::get().program_data.join("RustDeskPurge"), id) }

fn open_at(root: &Path, id: &str) -> Result<Checked, String> {
    let directory = directory(root, id)?;
    workspace::verify(&directory)?;
    let mut plan_file = locked(&directory.join("plan.json"))?;
    let mut bytes = Vec::new();
    (&mut plan_file).take(8193).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 8192 { return Err("purge handoff plan too large".into()); }
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|_| "purge handoff plan malformed")?;
    if plan.schema != 1 || plan.transaction_id != id || !token(&plan.installation_id) || !token(&plan.nonce) {
        return Err("purge handoff plan identity mismatch".into());
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    if current != directory.join("purge-helper.exe") { return Err("purge helper executable outside transaction".into()); }
    let mut image = locked(&current)?;
    if digest(&mut image)? != plan.helper_sha256 { return Err("purge helper hash mismatch".into()); }
    let owner = ObservedProcess::open(std::process::id())?.owner().clone();
    Ok(Checked { plan, directory, _plan: plan_file, _image: image, owner, claimed: false,
        plan_sha256: hex::encode(Sha256::digest(&bytes)) })
}

impl Checked {
    pub fn wait_parent(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if !ObservedProcess::owner_alive(&self.plan.parent)? { return Ok(()); }
            if Instant::now() >= deadline { return Err("purge parent still alive; Pending".into()); }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    pub fn claim(&mut self) -> Result<(), String> {
        if self.claimed { return Err("purge helper already claimed".into()); }
        self.wait_parent(Duration::ZERO)?;
        match std::fs::symlink_metadata(self.directory.join("result.json")) {
            Ok(_) => return Err("purge handoff terminal exists".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(format!("purge handoff result unknown: {error}")),
        }
        save_new(&self.directory.join("claim.json"), &serde_json::json!({"schema":1,
            "transaction_id":self.plan.transaction_id,"nonce":self.plan.nonce,"helper":self.owner,
            "plan_sha256":self.plan_sha256}))
            .map_err(|error| format!("purge handoff claim refused: {error}"))?;
        self.claimed = true;
        Ok(())
    }

    pub fn pending(&self) -> Result<(), String> {
        if self.directory.join("result.json").try_exists().map_err(|e| e.to_string())? {
            return Err("purge handoff terminal exists".into());
        }
        let path = self.directory.join(format!("progress-{}-{}.json", self.owner.pid, self.owner.created_at));
        save_new(&path, &serde_json::json!({"schema":1,"transaction_id":self.plan.transaction_id,
            "nonce":self.plan.nonce,"helper":self.owner,"outcome":"Pending"}))
    }

    pub fn execute_once(&mut self, action: impl FnOnce() -> Result<super::purge_policy::Outcome, String>) -> Result<super::purge_policy::Outcome, String> {
        self.claim()?;
        match action() {
            Ok(outcome) => { self.record(outcome, None)?; Ok(outcome) },
            Err(error) => {
                if let Err(record_error) = self.record(super::purge_policy::Outcome::CleanupFailed, Some(&error)) {
                    return Err(format!("{error}; purge result persistence failed: {record_error}"));
                }
                Err(error)
            }
        }
    }

    fn record(&self, outcome: super::purge_policy::Outcome, error: Option<&str>) -> Result<(), String> {
        if !self.claimed { return Err("purge terminal requires claim".into()); }
        if outcome == super::purge_policy::Outcome::Pending { return self.pending(); }
        save_new(&self.directory.join("result.json"), &serde_json::json!({"schema":1,
            "transaction_id":self.plan.transaction_id,"installation_id":self.plan.installation_id,
            "product_code":self.plan.product_code,"helper_sha256":self.plan.helper_sha256,
            "nonce":self.plan.nonce,"helper":self.owner,"outcome":format!("{outcome:?}"),"error":error}))
    }
}

#[cfg(test)]
pub fn prepare_fixture(root: &Path, id: &str) -> Result<PathBuf, String> {
    prepare_at(root, id, &"c".repeat(32), "{71A6889B-6003-42DB-989F-B7AE68260DDB}")
}
#[cfg(test)]
pub fn open_fixture(root: &Path, id: &str) -> Result<Checked, String> { open_at(root,id) }
