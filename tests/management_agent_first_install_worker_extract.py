"""Build-only extraction: preserve the actual production worker statements verbatim."""
import hashlib
import json
import pathlib
import sys

repo = pathlib.Path(__file__).resolve().parents[1]
source = repo / 'src/management_agent/rescue_first_install.rs'
text = source.read_text(encoding='utf-8')
worker = text[text.index('pub struct Worker {'):]
out = pathlib.Path(sys.argv[1])
out.mkdir(exist_ok=False)
prefix = r'''
mod worker_module {
    pub mod first_install {
        use std::{path::{Path, PathBuf}, sync::mpsc, time::{Duration, Instant}};
        struct Layout { agent: PathBuf, destination: PathBuf }
        fn fixed_layout() -> Result<Layout, String> {
            let agent = crate::ROOT.lock().unwrap().clone().ok_or("fixture absent")?;
            Ok(Layout { destination: agent.join("rescue"), agent })
        }
        fn absent(path: &Path) -> Result<bool, String> {
            path.try_exists().map(|v| !v).map_err(|e| e.to_string())
        }
        fn registered() {}
        fn live() {}
        fn prepare(layout: &Layout, _: Option<&[u8]>, _: fn(), _: fn()) -> Result<serde_json::Value, String> {
            crate::PREPARES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if !layout.agent.join("release-cache/first-install.json").exists() {
                Ok(serde_json::json!({"state":"unavailable","error_code":"waiting_approval","ready":false}))
            } else { Ok(serde_json::json!({"state":"approval_delivered","ready":false})) }
        }
        mod workspace {
            pub fn create(path: &std::path::Path) -> Result<(), String> { std::fs::create_dir(path).map_err(|e|e.to_string()) }
            pub fn verify(path: &std::path::Path) -> Result<(), String> { if path.is_dir(){Ok(())}else{Err("fixture directory absent".into())} }
        }
'''
(out / 'worker.rs').write_text(prefix + worker + '\n}}\n', encoding='utf-8')
(out / 'extraction.json').write_text(json.dumps({
    'source': str(source), 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
    'worker_sha256': hashlib.sha256(worker.encode()).hexdigest(),
    'boundary': 'verbatim production Worker/start_checked; layout/prepare/workspace isolated substitutes; no real installation E2E',
}, indent=2), encoding='utf-8')
