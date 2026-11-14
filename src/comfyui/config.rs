use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct HostConfig {
    pub host: String,
    pub ws: String,
    pub login: String,
    pub password: String,
}

impl HostConfig {
    pub fn load() -> Result<Self, String> {
        let Ok(path) = Self::settings_path() else {
            return Self::from_env();
        };
        Self::load_saved(&path)?.map_or_else(Self::from_env, Ok)
    }

    pub fn settings_path() -> Result<PathBuf, String> {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("mega-image-editor").join("comfyui-settings.json"))
            .ok_or_else(|| "Cannot locate user settings directory (LOCALAPPDATA)".into())
    }

    fn load_saved(path: &Path) -> Result<Option<Self>, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot read ComfyUI settings".into()),
        };
        let saved: SavedConfig = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid ComfyUI settings file".to_string())?;
        let password = String::from_utf8(unprotect(&saved.password)?)
            .map_err(|_| "Invalid saved ComfyUI password".to_string())?;
        Self::parse(saved.host, saved.ws, saved.login, password).map(Some)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::settings_path()?)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let saved = SavedConfig {
            host: self.host.clone(),
            ws: self.ws.clone(),
            login: self.login.clone(),
            password: protect(self.password.as_bytes())?,
        };
        let bytes = serde_json::to_vec_pretty(&saved)
            .map_err(|_| "Cannot encode ComfyUI settings".to_string())?;
        let parent = path.parent().ok_or("Invalid settings path")?;
        std::fs::create_dir_all(parent).map_err(|_| "Cannot create settings directory")?;
        let mut file =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "Cannot create settings file")?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| "Cannot write ComfyUI settings")?;
        file.persist(path)
            .map_err(|_| "Cannot replace ComfyUI settings")?;
        Ok(())
    }

    pub fn from_fields(host: String, login: String, password: String) -> Result<Self, String> {
        Self::parse(host, String::new(), login, password)
    }

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
        let mut url = reqwest::Url::parse(host.trim())
            .map_err(|_| "Enter a valid ComfyUI HTTP(S) URL in AI > Settings".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Host must use HTTP(S), without credentials, query or fragment".into());
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
            return Err("Enter both Basic Auth login and password, or leave both empty".into());
        }
        Ok(Self {
            host,
            ws,
            login,
            password,
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedConfig {
    host: String,
    #[serde(default)]
    ws: String,
    login: String,
    password: Vec<u8>,
}

#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI allocates output; copy it before releasing it with LocalFree.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(format!(
                "Cannot protect/unprotect password (Windows error {})",
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
            ));
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(result)
    }
}

fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(windows)]
    {
        crypt(bytes, true)
    }
    #[cfg(not(windows))]
    {
        let _ = bytes;
        Err("Saving credentials currently requires Windows DPAPI".into())
    }
}

fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(windows)]
    {
        crypt(bytes, false)
    }
    #[cfg(not(windows))]
    {
        let _ = bytes;
        Err("Reading credentials currently requires Windows DPAPI".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_settings_roundtrip_replace_and_clear_auth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(HostConfig::load_saved(&path).unwrap().is_none());
        let mut config =
            HostConfig::from_fields("https://example.com/comfy/".into(), "".into(), "".into())
                .unwrap();
        #[cfg(windows)]
        {
            config.login = "user".into();
            config.password = "secret-unique-password".into();
        }
        config.save_to(&path).unwrap();
        let loaded = HostConfig::load_saved(&path).unwrap().unwrap();
        assert_eq!(loaded.host, "https://example.com/comfy");
        assert_eq!(loaded.ws, "wss://example.com/comfy/ws");
        assert_eq!(loaded.login, config.login);
        assert_eq!(loaded.password, config.password);
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("secret-unique-password")
        );
        config.login.clear();
        config.password.clear();
        config.save_to(&path).unwrap();
        let loaded = HostConfig::load_saved(&path).unwrap().unwrap();
        assert!(loaded.login.is_empty() && loaded.password.is_empty());
        std::fs::write(&path, "broken").unwrap();
        assert!(HostConfig::load_saved(&path).is_err());
    }

    #[test]
    fn settings_reject_invalid_hosts_and_partial_auth() {
        for host in [
            "",
            "example.com",
            "ftp://example.com",
            "https://u:p@example.com",
            "https://example.com?token=secret",
        ] {
            assert!(HostConfig::from_fields(host.into(), "".into(), "".into()).is_err());
        }
        assert!(
            HostConfig::from_fields("http://localhost:8188".into(), "user".into(), "".into())
                .is_err()
        );
        assert!(
            HostConfig::from_fields("http://localhost:8188".into(), "".into(), "secret".into())
                .is_err()
        );
    }
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
