use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub nonce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub schema: u32,
    pub nonce: String,
    pub pid: u32,
    pub process_created_at: u64,
    pub version: String,
    pub build_seq: u64,
    pub local_responsive: bool,
    pub registered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_active_sessions: Option<bool>,
}

fn nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Request {
    pub fn new(nonce: String) -> Result<Self, String> {
        if !self::nonce(&nonce) {
            return Err("health nonce invalid".into());
        }
        Ok(Self { schema: 1, nonce })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 4096 {
            return Err("health request too large".into());
        }
        let request: Self =
            serde_json::from_slice(bytes).map_err(|_| "health request malformed".to_owned())?;
        if request.schema != 1 || !nonce(&request.nonce) {
            return Err("health request contract invalid".into());
        }
        Ok(request)
    }
}

impl Response {
    pub fn new(
        request: &Request,
        pid: u32,
        created_at: u64,
        version: String,
        build_seq: u64,
        local_responsive: bool,
        registered: bool,
    ) -> Result<Self, String> {
        if request.schema != 1
            || !nonce(&request.nonce)
            || pid == 0
            || created_at == 0
            || build_seq == 0
            || version.is_empty()
        {
            return Err("health response identity invalid".into());
        }
        Ok(Self {
            schema: 1,
            nonce: request.nonce.clone(),
            pid,
            process_created_at: created_at,
            version,
            build_seq,
            local_responsive,
            registered,
            no_active_sessions: None,
        })
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 4096 {
            return Err("health response too large".into());
        }
        serde_json::from_slice(bytes).map_err(|_| "health response malformed".into())
    }
    pub fn verify(
        &self,
        request: &Request,
        pid: u32,
        created_at: u64,
        build_seq: u64,
    ) -> Result<(), String> {
        if self.schema != 1
            || self.nonce != request.nonce
            || self.pid != pid
            || self.process_created_at != created_at
            || self.build_seq != build_seq
            || !self.local_responsive
            || self.version.is_empty()
        {
            return Err("health response identity/readiness mismatch".into());
        }
        Ok(())
    }
}
