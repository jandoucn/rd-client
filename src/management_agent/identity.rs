use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    config::Config,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct StoredIdentity {
    #[serde(default)]
    id: String,
    #[serde(default)]
    enc_id: String,
    key_pair: (Vec<u8>, Vec<u8>),
}

pub struct DeviceIdentity {
    pub id: String,
    pub uuid: String,
    pub secret: hbb_common::sodiumoxide::crypto::sign::SecretKey,
    pub public: hbb_common::sodiumoxide::crypto::sign::PublicKey,
}

pub fn load_existing() -> Result<DeviceIdentity, String> {
    let source = std::fs::read_to_string(Config::file())
        .map_err(|error| format!("identity read: {error}"))?;
    let machine_uuid = hbb_common::machine_uid::get().ok().map(String::into_bytes);
    parse_existing(&source, machine_uuid)
}

fn parse_existing(source: &str, machine_uuid: Option<Vec<u8>>) -> Result<DeviceIdentity, String> {
    hbb_common::sodiumoxide::init()
        .map_err(|_| "device crypto initialization failed".to_owned())?;
    let stored: StoredIdentity = hbb_common::toml::from_str(source)
        .map_err(|_| "identity configuration malformed".to_owned())?;
    use hbb_common::sodiumoxide::crypto::sign;
    let secret = sign::SecretKey::from_slice(&stored.key_pair.0)
        .ok_or_else(|| "identity secret invalid".to_owned())?;
    let public = sign::PublicKey::from_slice(&stored.key_pair.1)
        .ok_or_else(|| "identity public invalid".to_owned())?;
    let proof = b"rustdesk-agent-existing-key-check-v1";
    if !sign::verify_detached(&sign::sign_detached(proof, &secret), proof, &public) {
        return Err("identity key pair mismatch".into());
    }
    let uuid = machine_uuid
        .filter(|uuid| !uuid.is_empty())
        .unwrap_or_else(|| public.as_ref().to_vec());
    if uuid.is_empty() {
        return Err("identity UUID unavailable".into());
    }
    let id = if stored.enc_id.is_empty() {
        stored.id
    } else {
        decrypt_existing_id(&stored.enc_id, &uuid, public.as_ref())?
    };
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err("identity ID invalid".into());
    }
    Ok(DeviceIdentity {
        id,
        uuid: STANDARD.encode(uuid),
        secret,
        public,
    })
}

fn decrypt_existing_id(stored: &str, uuid: &[u8], public: &[u8]) -> Result<String, String> {
    use hbb_common::sodiumoxide::crypto::secretbox;
    let encoded = stored
        .strip_prefix("00")
        .ok_or_else(|| "identity encoding unsupported".to_owned())?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "identity decryption failed".to_owned())?;
    // Read the existing client format without get_uuid()/Config getters: their
    // machine-UID fallback can generate a replacement key when configuration is lost.
    for source in [uuid, public] {
        let mut key = [0; secretbox::KEYBYTES];
        let length = source.len().min(key.len());
        key[..length].copy_from_slice(&source[..length]);
        let key = secretbox::Key(key);
        if bytes.first() == Some(&1)
            && bytes.len() >= 1 + secretbox::NONCEBYTES + secretbox::MACBYTES
        {
            if let Some(nonce) = secretbox::Nonce::from_slice(&bytes[1..1 + secretbox::NONCEBYTES])
            {
                if let Ok(plain) =
                    secretbox::open(&bytes[1 + secretbox::NONCEBYTES..], &nonce, &key)
                {
                    return String::from_utf8(plain)
                        .map_err(|_| "identity ID encoding invalid".into());
                }
            }
        }
        if let Ok(plain) =
            secretbox::open(&bytes, &secretbox::Nonce([0; secretbox::NONCEBYTES]), &key)
        {
            return String::from_utf8(plain).map_err(|_| "identity ID encoding invalid".into());
        }
    }
    Err("identity decryption failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_key_and_id_are_read_without_generating_replacements() {
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let source = hbb_common::toml::to_string(&serde_json::json!({
            "id": "123456789", "key_pair": [secret.as_ref(), public.as_ref()]
        }))
        .unwrap();
        let parsed = parse_existing(&source, Some(vec![1; 16])).unwrap();
        assert_eq!(parsed.id, "123456789");
        assert_eq!(parsed.public, public);
        assert_eq!(parsed.secret, secret);
        for invalid in [
            "",
            "id = '123456789'",
            "id = ''\nkey_pair = [[], []]",
            "id = 123",
        ] {
            assert!(
                parse_existing(invalid, Some(vec![1; 16])).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn encrypted_or_mismatched_identity_fails_closed() {
        hbb_common::sodiumoxide::init().unwrap();
        let (_, secret) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let (public, _) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let source = hbb_common::toml::to_string(&serde_json::json!({
            "id": "123456789", "key_pair": [secret.as_ref(), public.as_ref()]
        }))
        .unwrap();
        assert!(parse_existing(&source, Some(vec![1; 16]))
            .err()
            .unwrap()
            .contains("mismatch"));
        // A valid pair is needed before the encrypted ID is examined.
        let (public, secret) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        let source = hbb_common::toml::to_string(&serde_json::json!({
            "enc_id": "00invalid", "key_pair": [secret.as_ref(), public.as_ref()]
        }))
        .unwrap();
        assert!(parse_existing(&source, Some(vec![1; 16]))
            .err()
            .unwrap()
            .contains("decryption"));
    }

    #[test]
    fn existing_legacy_and_random_nonce_ids_decrypt_with_machine_or_public_key() {
        use hbb_common::sodiumoxide::crypto::{secretbox, sign};
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = sign::gen_keypair();
        let machine = vec![9; 36];
        for use_public in [false, true] {
            let source_key = if use_public {
                public.as_ref()
            } else {
                machine.as_slice()
            };
            let mut key = [0; secretbox::KEYBYTES];
            let length = source_key.len().min(key.len());
            key[..length].copy_from_slice(&source_key[..length]);
            for random in [false, true] {
                let nonce = if random {
                    secretbox::gen_nonce()
                } else {
                    secretbox::Nonce([0; secretbox::NONCEBYTES])
                };
                let ciphertext = secretbox::seal(b"123456789", &nonce, &secretbox::Key(key));
                let encrypted = if random {
                    [vec![1], nonce.as_ref().to_vec(), ciphertext].concat()
                } else {
                    ciphertext
                };
                let stored = format!("00{}", STANDARD.encode(encrypted));
                let source = hbb_common::toml::to_string(&serde_json::json!({
                    "enc_id": stored, "key_pair": [secret.as_ref(), public.as_ref()]
                }))
                .unwrap();
                let parsed = parse_existing(&source, Some(machine.clone())).unwrap();
                assert_eq!(parsed.id, "123456789");
                assert_eq!(parsed.uuid, STANDARD.encode(&machine));
                if use_public {
                    assert_eq!(
                        parse_existing(&source, None).unwrap().uuid,
                        STANDARD.encode(public.as_ref())
                    );
                }
            }
        }
    }
}
