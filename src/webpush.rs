//! Web Push for the phone web app (`/phone`): RFC 8030 delivery, RFC 8291
//! `aes128gcm` payload encryption and RFC 8292 VAPID authentication.
//!
//! Nothing here needs an Apple developer account: an installed (Home Screen)
//! web app on iOS 16.4+ subscribes through the browser, and the push service
//! endpoint it hands us is what we POST encrypted payloads to. The VAPID key
//! is a P-256 key in `~/.config/sjbis/vapid.pem` (PKCS#8), generated on first
//! use; its public half is what the page passes as `applicationServerKey`.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::rand_core::OsRng;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use p256::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Env var overriding the VAPID private key path.
pub const VAPID_KEY_ENV: &str = "SJBIS_VAPID_KEY";
/// Env var overriding the VAPID `sub` claim (a `mailto:` or `https:` URI the
/// push service may use to contact the operator).
pub const VAPID_SUBJECT_ENV: &str = "SJBIS_VAPID_SUBJECT";
const DEFAULT_SUBJECT: &str = "mailto:sjbis@localhost";
/// Record size written into the aes128gcm header; one record carries the
/// whole payload, so payloads are capped below it.
const RECORD_SIZE: u32 = 4096;
pub const MAX_PAYLOAD: usize = 3800;
/// VAPID tokens are good for 12h (the spec caps them at 24h).
const JWT_LIFETIME: Duration = Duration::from_secs(12 * 3600);

/// One browser push subscription (what `PushManager.subscribe` returned).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    /// base64url, the client's P-256 public key (65 bytes uncompressed).
    pub p256dh: String,
    /// base64url, the client's 16-byte auth secret.
    pub auth: String,
}

/// What happened to one push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The push service accepted it (2xx).
    Sent,
    /// The subscription is dead (404/410): forget it.
    Gone,
    /// Anything else; the message is the status and body.
    Failed(String),
}

pub struct WebPush {
    key: SecretKey,
    subject: String,
    http: reqwest::Client,
}

pub fn key_path() -> PathBuf {
    if let Some(p) = std::env::var_os(VAPID_KEY_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/sjbis/vapid.pem")
}

/// Load the VAPID key, generating and writing one if the file is missing.
pub fn load_or_create_key(path: &std::path::Path) -> Result<SecretKey> {
    match std::fs::read_to_string(path) {
        Ok(pem) => SecretKey::from_pkcs8_pem(&pem)
            .with_context(|| format!("{} is not a PKCS#8 P-256 key", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::random(&mut OsRng);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("create {}", dir.display()))?;
            }
            let pem = key
                .to_pkcs8_pem(LineEnding::LF)
                .map_err(|e| anyhow!("encode VAPID key: {e}"))?;
            std::fs::write(path, pem.as_bytes())
                .with_context(|| format!("write {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
            tracing::info!("generated VAPID key at {}", path.display());
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

impl WebPush {
    pub fn new(key: SecretKey, subject: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { key, subject, http })
    }

    /// From the configured key path and subject env var.
    pub fn from_env() -> Result<Self> {
        let key = load_or_create_key(&key_path())?;
        let subject = std::env::var(VAPID_SUBJECT_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SUBJECT.to_string());
        Self::new(key, subject)
    }

    /// base64url (no padding) of the uncompressed public point: what the page
    /// passes as `applicationServerKey`.
    pub fn public_key_b64url(&self) -> String {
        B64URL.encode(self.key.public_key().to_encoded_point(false).as_bytes())
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Encrypt `payload` for `sub` and POST it. `ttl` is how long the push
    /// service may hold it; `topic` lets a later push replace an earlier one.
    pub async fn send(
        &self,
        sub: &Subscription,
        payload: &[u8],
        ttl: u32,
        topic: Option<&str>,
    ) -> Outcome {
        let body = match self.encrypt_for(sub, payload) {
            Ok(b) => b,
            Err(e) => return Outcome::Failed(format!("encrypt: {e:#}")),
        };
        let auth = match self.vapid_header(&sub.endpoint) {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(format!("vapid: {e:#}")),
        };
        let mut req = self
            .http
            .post(&sub.endpoint)
            .header("Authorization", auth)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", ttl.to_string())
            .header("Urgency", "high")
            .body(body);
        if let Some(t) = topic.filter(|t| !t.is_empty() && t.len() <= 32) {
            req = req.header("Topic", t);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => Outcome::Sent,
            Ok(resp) if resp.status().as_u16() == 404 || resp.status().as_u16() == 410 => {
                Outcome::Gone
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                Outcome::Failed(format!(
                    "HTTP {status}: {}",
                    text.chars().take(300).collect::<String>()
                ))
            }
            Err(e) => Outcome::Failed(format!("request: {e}")),
        }
    }

    fn encrypt_for(&self, sub: &Subscription, payload: &[u8]) -> Result<Vec<u8>> {
        let ua_public = B64URL
            .decode(sub.p256dh.trim_end_matches('='))
            .context("p256dh is not base64url")?;
        let auth = B64URL
            .decode(sub.auth.trim_end_matches('='))
            .context("auth is not base64url")?;
        let as_secret = SecretKey::random(&mut OsRng);
        let mut salt = [0u8; 16];
        p256::elliptic_curve::rand_core::RngCore::fill_bytes(&mut OsRng, &mut salt);
        encrypt(&ua_public, &auth, &as_secret, &salt, payload)
    }

    /// `vapid t=<jwt>, k=<pubkey>` for the push service at `endpoint`'s origin.
    fn vapid_header(&self, endpoint: &str) -> Result<String> {
        let url = reqwest::Url::parse(endpoint).context("endpoint is not a URL")?;
        let aud = format!(
            "{}://{}",
            url.scheme(),
            url.host_str()
                .ok_or_else(|| anyhow!("endpoint has no host"))?
        ) + &url.port().map(|p| format!(":{p}")).unwrap_or_default();
        let exp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + JWT_LIFETIME.as_secs();
        let jwt = vapid_jwt(&self.key, &aud, exp, &self.subject);
        Ok(format!("vapid t={jwt}, k={}", self.public_key_b64url()))
    }
}

/// A VAPID JWT: ES256 over `b64url(header).b64url(claims)`, raw `r||s` signature.
pub fn vapid_jwt(key: &SecretKey, aud: &str, exp: u64, sub: &str) -> String {
    let header = B64URL.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims =
        B64URL.encode(serde_json::json!({ "aud": aud, "exp": exp, "sub": sub }).to_string());
    let signing_input = format!("{header}.{claims}");
    let sig: Signature = SigningKey::from(key).sign(signing_input.as_bytes());
    format!("{signing_input}.{}", B64URL.encode(sig.to_bytes()))
}

/// RFC 8291 `aes128gcm` encryption of one record. `ua_public` is the
/// subscriber's uncompressed P-256 point, `auth` its 16-byte secret,
/// `as_secret` our ephemeral key, `salt` 16 random bytes. Output is the
/// self-describing body: header (salt, rs, keyid = our public point) then
/// the ciphertext of `payload || 0x02`.
pub fn encrypt(
    ua_public: &[u8],
    auth: &[u8],
    as_secret: &SecretKey,
    salt: &[u8; 16],
    payload: &[u8],
) -> Result<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD {
        bail!(
            "payload is {} bytes; the limit is {MAX_PAYLOAD}",
            payload.len()
        );
    }
    let ua_key = PublicKey::from_sec1_bytes(ua_public)
        .map_err(|e| anyhow!("p256dh is not a P-256 point: {e}"))?;
    let as_public = as_secret.public_key().to_encoded_point(false);
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_key.as_affine());

    // IKM = HKDF-Expand(HKDF-Extract(auth, ecdh), "WebPush: info" || 0 || ua_public || as_public, 32)
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public.as_bytes());
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|_| anyhow!("hkdf ikm"))?;

    // CEK and NONCE from HKDF-Extract(salt, IKM)
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| anyhow!("hkdf cek"))?;
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| anyhow!("hkdf nonce"))?;

    let mut record = payload.to_vec();
    record.push(0x02); // last-record padding delimiter
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| anyhow!("aes key"))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), record.as_slice())
        .map_err(|_| anyhow!("aes-gcm encrypt"))?;

    let mut out = Vec::with_capacity(16 + 4 + 1 + 65 + ciphertext.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.as_bytes().len() as u8);
    out.extend_from_slice(as_public.as_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier;

    fn b64(s: &str) -> Vec<u8> {
        B64URL.decode(s).unwrap()
    }

    /// RFC 8291 Appendix A: fixed keys and salt must give the published body.
    #[test]
    fn rfc8291_appendix_a_vector() {
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let as_secret =
            SecretKey::from_slice(&b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body = encrypt(
            &ua_public,
            &auth,
            &as_secret,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            B64URL.encode(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn payload_cap_is_enforced() {
        let ua = SecretKey::random(&mut OsRng);
        let ua_public = ua.public_key().to_encoded_point(false);
        let e = encrypt(
            ua_public.as_bytes(),
            &[0u8; 16],
            &SecretKey::random(&mut OsRng),
            &[0u8; 16],
            &vec![0u8; MAX_PAYLOAD + 1],
        )
        .unwrap_err();
        assert!(e.to_string().contains("limit"));
    }

    #[test]
    fn vapid_jwt_verifies_with_the_public_key() {
        let key = SecretKey::random(&mut OsRng);
        let jwt = vapid_jwt(
            &key,
            "https://web.push.apple.com",
            1_800_000_000,
            "mailto:x@y",
        );
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&b64(parts[0])).unwrap();
        assert_eq!(header["alg"], "ES256");
        let claims: serde_json::Value = serde_json::from_slice(&b64(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
        assert_eq!(claims["exp"], 1_800_000_000u64);
        let sig = Signature::from_slice(&b64(parts[2])).unwrap();
        let vk = VerifyingKey::from(key.public_key());
        vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .unwrap();
    }

    #[test]
    fn key_file_round_trips_and_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("vapid.pem");
        let a = load_or_create_key(&path).unwrap();
        let b = load_or_create_key(&path).unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        let wp = WebPush::new(a, "mailto:t@t".into()).unwrap();
        let pk = wp.public_key_b64url();
        assert_eq!(B64URL.decode(&pk).unwrap().len(), 65);
        assert_eq!(wp.subject(), "mailto:t@t");
    }
}
