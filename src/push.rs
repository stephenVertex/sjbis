use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ApnsClient {
    team_id: String,
    key_id: String,
    topic: String,
    endpoint: &'static str,
    encoding_key: jsonwebtoken::EncodingKey,
    http_client: reqwest::Client,
    jwt_cache: Option<CachedJwt>,
}

struct CachedJwt {
    token: String,
    expires_at: u64,
}

#[derive(Serialize)]
struct Aps {
    alert: ApsAlert,
    sound: String,
}

#[derive(Serialize)]
struct ApsAlert {
    title: String,
    body: String,
}

#[derive(Serialize)]
struct PushPayload<'a> {
    aps: Aps,
    notification_id: &'a str,
}

#[derive(Serialize, Deserialize)]
struct JwtClaims {
    iss: String,
    iat: u64,
}

impl ApnsClient {
    pub fn new(team_id: &str, key_id: &str, private_key_pem: &str, topic: &str, environment: &str) -> Result<Self> {
        let endpoint = match environment {
            "development" => "https://api.development.push.apple.com",
            "production" => "https://api.push.apple.com",
            _ => anyhow::bail!("APNS_ENVIRONMENT must be development or production"),
        };
        Ok(Self {
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            topic: topic.to_string(),
            endpoint,
            encoding_key: jsonwebtoken::EncodingKey::from_ec_pem(private_key_pem.as_bytes())
                .context("failed to parse APNs private key as EC PEM")?,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            jwt_cache: None,
        })
    }

    pub async fn send(&mut self, device_token: &str, title: &str, body: &str, notification_id: &str, expiry: Option<i64>) -> Result<()> {
        let jwt = self.get_jwt()?;

        let payload = PushPayload {
            aps: Aps {
                alert: ApsAlert {
                    title: title.to_string(),
                    body: body.to_string(),
                },
                sound: "default".to_string(),
            },
            notification_id,
        };

        let url = format!("{}/3/device/{}", self.endpoint, device_token);

        let mut request = self.http_client
            .post(&url)
            .header("authorization", format!("bearer {}", jwt))
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .json(&payload);
        if let Some(expiry) = expiry {
            request = request.header("apns-expiration", expiry.to_string());
        }
        let resp = request
            .send()
            .await
            .context("APNs request failed")?;

        if resp.status().is_success() {
            tracing::debug!(%notification_id, "APNs accepted notification");
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
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

        let token = jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .context("Failed to encode APNs JWT")?;

        self.jwt_cache = Some(CachedJwt {
            token: token.clone(),
            expires_at: now + 3600,
        });

        Ok(token)
    }
}
