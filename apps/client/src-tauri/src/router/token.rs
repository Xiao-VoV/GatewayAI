use std::time::{Duration, Instant};

use serde::Deserialize;

use super::RouterState;

const TOKEN_SKEW: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(crate) struct TokenCache {
    pub(crate) token: String,
    pub(crate) expires_at: Instant,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
}

impl RouterState {
    async fn refresh_token(&self) -> Result<String, String> {
        let url = format!("{}/api/client/token", self.gateway);
        let response = self
            .client
            .post(url)
            .json(&serde_json::json!({
                "device_id": self.device_id,
                "device_secret": self.device_secret,
            }))
            .send()
            .await
            .map_err(|error| format!("刷新设备 token 失败：{error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("刷新设备 token 失败（HTTP {status}）：{body}"));
        }
        let payload: TokenResponse = response
            .json()
            .await
            .map_err(|error| format!("解析 token 响应失败：{error}"))?;
        let expires_at = chrono_free_parse(&payload.expires_at)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(600));
        let token = payload.token;
        let mut guard = self.token.lock().await;
        *guard = Some(TokenCache {
            token: token.clone(),
            expires_at,
        });
        Ok(token)
    }

    pub(crate) async fn token(&self, force_refresh: bool) -> Result<String, String> {
        if !force_refresh {
            let guard = self.token.lock().await;
            if let Some(cache) = guard.as_ref() {
                if cache.expires_at.saturating_duration_since(Instant::now()) > TOKEN_SKEW {
                    return Ok(cache.token.clone());
                }
            }
        }
        self.refresh_token().await
    }
}

/// RFC3339 解析：避免引入 chrono；仅用于计算相对 TTL。
fn chrono_free_parse(raw: &str) -> Option<Instant> {
    let date = raw.trim_end_matches('Z');
    let (date, time) = date.split_once('T')?;
    let mut date_parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let year = date_parts.next()??;
    let month = date_parts.next()??;
    let day = date_parts.next()??;
    let mut time_parts = time.split(':').map(|part| part.parse::<f64>().ok());
    let hour = time_parts.next()??;
    let minute = time_parts.next()??;
    let second = time_parts.next()??;
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + (hour as i64) * 3_600 + (minute as i64) * 60 + second as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let delta = seconds - now;
    if delta <= 0 {
        return Some(Instant::now());
    }
    Some(Instant::now() + Duration::from_secs(delta.min(24 * 3600) as u64))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::{chrono_free_parse, days_from_civil};

    #[test]
    fn civil_days_are_stable() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
    }

    #[test]
    fn parses_future_rfc3339() {
        let parsed = chrono_free_parse("2999-01-01T00:00:00Z").expect("parse future");
        assert!(parsed > std::time::Instant::now());
        assert!(chrono_free_parse("2000-01-01T00:00:00Z").is_some());
    }
}
