use std::collections::HashMap;
use std::path::Path;

#[derive(Clone)]
pub struct HostConfig {
    pub host: String,
    pub ws: String,
    pub login: String,
    pub password: String,
}

impl HostConfig {
    pub fn from_env() -> Result<Self, String> {
        let exe_env = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(".env")));
        let cwd_env = Path::new(".env");
        let path = exe_env
            .as_deref()
            .filter(|p| p.is_file())
            .unwrap_or(cwd_env);
        let values = if path.is_file() {
            dotenvy::from_path_iter(path)
                .map_err(|_| "Cannot read ComfyUI .env".to_string())?
                .collect::<Result<HashMap<_, _>, _>>()
                .map_err(|_| "Invalid ComfyUI .env syntax".to_string())?
        } else {
            HashMap::new()
        };
        let get = |key: &str| {
            std::env::var(key)
                .ok()
                .or_else(|| values.get(key).cloned())
                .unwrap_or_default()
        };
        Self::parse(
            get("COMFYUI_HOST"),
            get("COMFYUI_WS"),
            get("COMFYUI_LOGIN"),
            get("COMFYUI_PASSWORD"),
        )
    }

    fn parse(host: String, ws: String, login: String, password: String) -> Result<Self, String> {
        let mut url = reqwest::Url::parse(host.trim()).map_err(|_| {
            "Set COMFYUI_HOST in .env beside the executable (or in the working directory)"
                .to_string()
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "COMFYUI_HOST must be an HTTP(S) URL without credentials, query or fragment".into(),
            );
        }
        let host = url.as_str().trim_end_matches('/').to_string();
        let ws = if ws.trim().is_empty() {
            let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
            url.set_scheme(scheme)
                .map_err(|_| "Invalid ComfyUI scheme".to_string())?;
            url.set_path(&format!("{}/ws", url.path().trim_end_matches('/')));
            url.to_string()
        } else {
            ws.trim().to_string()
        };
        let ws_url = reqwest::Url::parse(&ws).map_err(|_| "Invalid COMFYUI_WS".to_string())?;
        if !matches!(ws_url.scheme(), "ws" | "wss")
            || ws_url.host_str().is_none()
            || !ws_url.username().is_empty()
            || ws_url.password().is_some()
        {
            return Err("COMFYUI_WS must be a WS(S) URL without embedded credentials".into());
        }
        if login.is_empty() != password.is_empty() {
            return Err("Set both COMFYUI_LOGIN and COMFYUI_PASSWORD, or leave both empty".into());
        }
        Ok(Self {
            host,
            ws,
            login,
            password,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_ws_and_preserves_prefix() {
        let c = HostConfig::parse(
            "https://example.com/comfy/".into(),
            "".into(),
            "".into(),
            "".into(),
        )
        .unwrap();
        assert_eq!(c.ws, "wss://example.com/comfy/ws");
        assert!(
            HostConfig::parse(
                "https://example.com".into(),
                "".into(),
                "user".into(),
                "".into()
            )
            .is_err()
        );
    }
}
