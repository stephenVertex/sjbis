use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ApnsClient {
    team_id: String,
    key_id: String,
    private_key: String,
    http_client: reqwest::Client,
    jwt_cache: Option<CachedJwt>,
}

struct CachedJwt {
    token: String,
    expires_at: u64,
}

#[derive(Serialize)]
struct ApsPayload {
    aps: Aps,
}

#[derive(Serialize)]
struct Aps {
    alert: ApsAlert,
    sound: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    badge: Option<i32>,
}

#[derive(Serialize)]
struct ApsAlert {
    title: String,
    body: String,
}

#[derive(Serialize)]
struct PushPayload {
    aps: Aps,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct JwtClaims {
    iss: String,
    iat: u64,
}

impl ApnsClient {
    pub fn new(team_id: &str, key_id: &str, private_key_pem: &str) -> Result<Self> {
        Ok(Self {
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            private_key: private_key_pem.to_string(),
            http_client: reqwest::Client::builder()
                .build()?,
            jwt_cache: None,
        })
    }

    pub async fn send(&mut self, device_token: &str, title: &str, body: &str, notification_id: Option<&str>) -> Result<()> {
        let jwt = self.get_jwt()?;

        let payload = PushPayload {
            aps: Aps {
                alert: ApsAlert {
                    title: title.to_string(),
                    body: body.to_string(),
                },
                sound: "default".to_string(),
                badge: Some(1),
            },
            notification_id: notification_id.map(|s| s.to_string()),
        };

        let url = format!("https://api.push.apple.com/3/device/{}", device_token);

        let resp = self.http_client
            .post(&url)
            .header("authorization", format!("bearer {}", jwt))
            .header("apns-topic", &format!("{}.Sjbis", self.team_id))
            .header("apns-push-type", "alert")
            .json(&payload)
            .send()
            .await
            .context("APNs request failed")?;

        if resp.status().is_success() {
            tracing::info!("APNs push sent to device token (first 8: {}…)", &device_token[..8.min(device_token.len())]);
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!("APNs push failed: {} — {}", status, body);
            Err(anyhow::anyhow!("APNs error: {} {}", status, body))
        }
    }

    fn get_jwt(&mut self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if let Some(ref cached) = self.jwt_cache {
            if cached.expires_at > now + 60 {
                return Ok(cached.token.clone());
            }
        }

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.key_id.clone());

        let claims = JwtClaims {
            iss: self.team_id.clone(),
            iat: now,
        };

        let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(self.private_key.as_bytes())
            .context("Failed to parse APNs private key as EC PEM")?;

        let token = jsonwebtoken::encode(&header, &claims, &encoding_key)
            .context("Failed to encode APNs JWT")?;

        self.jwt_cache = Some(CachedJwt {
            token: token.clone(),
            expires_at: now + 3600,
        });

        Ok(token)
    }
}
