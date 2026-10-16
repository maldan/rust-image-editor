use super::{HostConfig, client::Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::mpsc,
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub models: Vec<String>,
    pub loras: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct CachedCatalog {
    host: String,
    catalog: Catalog,
}

pub struct CatalogLoad(mpsc::Receiver<Result<(Catalog, String), String>>);

impl CatalogLoad {
    pub fn start(config: HostConfig, refresh: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load(config, refresh));
        });
        Self(rx)
    }

    pub fn poll(&self) -> Option<Result<(Catalog, String), String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Catalog worker stopped unexpectedly".into())),
        }
    }
}

fn cache_path(host: &str) -> PathBuf {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    host.hash(&mut hash);
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    root.join("mega-image-editor")
        .join(format!("comfyui-catalog-{:016x}.json", hash.finish()))
}

fn choices(info: &Value, class: &str, input: &str) -> Result<Vec<String>, String> {
    let items = info[class]["input"]["required"][input][0]
        .as_array()
        .ok_or_else(|| format!("ComfyUI has no {class}.{input} list"))?;
    let mut names = items
        .iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| format!("Invalid {class}.{input} list"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

fn load(config: HostConfig, refresh: bool) -> Result<(Catalog, String), String> {
    let path = cache_path(&config.host);
    load_at(config, refresh, &path)
}

fn load_at(
    config: HostConfig,
    refresh: bool,
    path: &std::path::Path,
) -> Result<(Catalog, String), String> {
    if !refresh {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(cached) = serde_json::from_slice::<CachedCatalog>(&bytes) {
                if cached.host == config.host {
                    return Ok((cached.catalog, "Models / LoRAs loaded from cache".into()));
                }
            }
        }
    }
    let client = Client::new(config.clone())?;
    let catalog = Catalog {
        models: choices(
            &client.object_info("CheckpointLoaderSimple")?,
            "CheckpointLoaderSimple",
            "ckpt_name",
        )?,
        loras: choices(
            &client.object_info("LoraLoader")?,
            "LoraLoader",
            "lora_name",
        )?,
    };
    let bytes = serde_json::to_vec(&CachedCatalog {
        host: config.host,
        catalog: catalog.clone(),
    })
    .map_err(|e| e.to_string())?;
    let saved =
        std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| std::fs::write(&path, bytes));
    let message = if saved.is_ok() {
        "Models / LoRAs loaded; cache updated"
    } else {
        "Models / LoRAs loaded; cannot save cache"
    };
    Ok((catalog, message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_survives_restart_and_failed_refresh_and_is_replaced_on_success() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let config = HostConfig {
            ws: String::new(),
            host,
            login: String::new(),
            password: String::new(),
        };
        let server = std::thread::spawn(move || {
            for revision in ["old", "new"] {
                for (class, input) in [
                    ("CheckpointLoaderSimple", "ckpt_name"),
                    ("LoraLoader", "lora_name"),
                ] {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = [0; 4096];
                    let n = stream.read(&mut request).unwrap();
                    assert!(
                        String::from_utf8_lossy(&request[..n])
                            .contains(&format!("GET /object_info/{class} "))
                    );
                    let body =
                        serde_json::json!({class: {"input": {"required": {input: [[revision]]}}}})
                            .to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        assert_eq!(
            load_at(config.clone(), false, &path).unwrap().0.models,
            ["old"]
        );
        assert_eq!(
            load_at(config.clone(), false, &path).unwrap().0.loras,
            ["old"]
        );
        assert_eq!(
            load_at(config.clone(), true, &path).unwrap().0.models,
            ["new"]
        );
        server.join().unwrap();
        assert!(load_at(config.clone(), true, &path).is_err());
        assert_eq!(load_at(config, false, &path).unwrap().0.loras, ["new"]);
    }

    #[test]
    fn parses_sorts_and_deduplicates_server_choices() {
        let info = serde_json::json!({"LoraLoader": {"input": {"required": {"lora_name": [["b", "a", "b"]]}}}});
        assert_eq!(
            choices(&info, "LoraLoader", "lora_name").unwrap(),
            vec!["a", "b"]
        );
        assert!(choices(&info, "CheckpointLoaderSimple", "ckpt_name").is_err());
        assert_ne!(cache_path("https://one"), cache_path("https://two"));
    }
}
