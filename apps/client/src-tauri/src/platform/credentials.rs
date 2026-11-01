use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "com.aigateway.client";
const KEYRING_ACCOUNT: &str = "device-session";

/// 客户端持久化凭据：设备身份 + 虚拟 Key，仅写入系统钥匙串（绝不落普通文件）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCredentials {
    pub server_url: String,
    pub device_id: String,
    pub device_secret: String,
    pub api_key: String,
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|error| error.to_string())
}

pub fn save(payload: StoredCredentials) -> Result<(), String> {
    let serialized = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
    entry()?
        .set_password(&serialized)
        .map_err(|error| error.to_string())
}

pub fn load() -> Result<Option<StoredCredentials>, String> {
    match entry()?.get_password() {
        Ok(serialized) => serde_json::from_str::<StoredCredentials>(&serialized)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::StoredCredentials;

    #[test]
    fn credentials_roundtrip_camel_case() {
        let value = StoredCredentials {
            server_url: "https://gw.example.com".to_string(),
            device_id: "device".to_string(),
            device_secret: "secret".to_string(),
            api_key: "sk-gw-test".to_string(),
        };
        let json = serde_json::to_string(&value).unwrap();
        assert!(json.contains("\"serverUrl\""));
        assert!(json.contains("\"apiKey\""));
        let parsed: StoredCredentials = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device_id, "device");
    }
}
