use std::io::Read;
use std::time::Duration;

use super::HostConfig;
use reqwest::blocking::{Client as HttpClient, RequestBuilder, Response, multipart};
use serde_json::{Value, json};

pub struct Client {
    http: HttpClient,
    config: HostConfig,
}

impl Client {
    pub fn new(config: HostConfig) -> Result<Self, String> {
        let http = HttpClient::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot initialize HTTP client".to_string())?;
        Ok(Self { http, config })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        let request = self
            .http
            .request(method, format!("{}{path}", self.config.host));
        if self.config.login.is_empty() {
            request
        } else {
            request.basic_auth(&self.config.login, Some(&self.config.password))
        }
    }

    fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        let response = request.send().map_err(|e| {
            if e.is_timeout() {
                "ComfyUI request timed out".to_string()
            } else {
                "Cannot reach ComfyUI server".to_string()
            }
        })?;
        if !response.status().is_success() {
            return Err(format!("ComfyUI HTTP {}", response.status().as_u16()));
        }
        Ok(response)
    }

    fn json(&self, request: RequestBuilder) -> Result<Value, String> {
        self.send(request)?
            .json()
            .map_err(|_| "Invalid ComfyUI JSON response".into())
    }

    pub fn upload(&self, bytes: Vec<u8>) -> Result<String, String> {
        self.upload_named(bytes, "mega-image-editor-input.png")
    }

    pub fn upload_mask(&self, bytes: Vec<u8>) -> Result<String, String> {
        self.upload_named(bytes, "mega-image-editor-mask.png")
    }

    fn upload_named(&self, bytes: Vec<u8>, name: &'static str) -> Result<String, String> {
        let part = multipart::Part::bytes(bytes)
            .file_name(name)
            .mime_str("image/png")
            .map_err(|e| e.to_string())?;
        let form = multipart::Form::new()
            .part("image", part)
            .text("overwrite", "true");
        let response = self.json(
            self.request(reqwest::Method::POST, "/upload/image")
                .multipart(form),
        )?;
        let filename = response["name"]
            .as_str()
            .ok_or("Upload response has no filename")?;
        let folder = response["subfolder"].as_str().unwrap_or("");
        Ok(if folder.is_empty() {
            filename.into()
        } else {
            format!("{folder}/{filename}")
        })
    }

    pub fn prompt(&self, graph: Value, client_id: &str) -> Result<String, String> {
        let request = self
            .request(reqwest::Method::POST, "/prompt")
            .json(&json!({"prompt": graph, "client_id": client_id}));
        // Validation errors are useful to the user, but avoid logging the full request/graph.
        let response = request
            .send()
            .map_err(|_| "Cannot submit ComfyUI workflow".to_string())?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|_| format!("ComfyUI HTTP {}", status.as_u16()))?;
        if !status.is_success() || body.get("error").is_some() {
            return Err(format!(
                "ComfyUI rejected workflow: {}",
                body.get("node_errors")
                    .filter(|v| v.as_object().is_some_and(|v| !v.is_empty()))
                    .or(body.get("error"))
                    .unwrap_or(&body)
            ));
        }
        body["prompt_id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| "Missing prompt_id".into())
    }

    pub fn history(&self, id: &str) -> Result<Value, String> {
        self.json(self.request(reqwest::Method::GET, &format!("/history/{id}")))
    }

    pub fn object_info(&self, class: &str) -> Result<Value, String> {
        self.json(self.request(reqwest::Method::GET, &format!("/object_info/{class}")))
    }

    pub fn download(&self, output: &Value) -> Result<Vec<u8>, String> {
        self.download_limited(output, 128 * 1024 * 1024, Duration::from_secs(30))
    }

    pub fn download_video(&self, output: &Value) -> Result<Vec<u8>, String> {
        self.download_limited(output, 1024 * 1024 * 1024, Duration::from_secs(600))
    }

    fn download_limited(&self, output: &Value, limit: u64, timeout: Duration) -> Result<Vec<u8>, String> {
        let filename = output["filename"]
            .as_str()
            .ok_or("Output has no filename")?;
        let request = self.request(reqwest::Method::GET, "/view").timeout(timeout).query(&[
            ("filename", filename),
            ("subfolder", output["subfolder"].as_str().unwrap_or("")),
            ("type", output["type"].as_str().unwrap_or("output")),
        ]);
        let mut bytes = Vec::new();
        self.send(request)?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot download output".to_string())?;
        if bytes.len() as u64 > limit {
            return Err(format!("ComfyUI output exceeds {} MiB", limit / 1024 / 1024));
        }
        Ok(bytes)
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        // Remove this prompt if still queued; never clear another client's queue.
        self.send(
            self.request(reqwest::Method::POST, "/queue")
                .timeout(Duration::from_secs(5))
                .json(&json!({"delete": [id]})),
        )?;
        let queue = self.json(
            self.request(reqwest::Method::GET, "/queue")
                .timeout(Duration::from_secs(5)),
        )?;
        if !queue["queue_running"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item[1] == id))
        {
            return Ok(());
        }
        let response = self
            .request(reqwest::Method::POST, "/api/interrupt")
            .timeout(Duration::from_secs(5))
            .json(&json!({"prompt_id": id}))
            .send()
            .map_err(|_| "Cannot interrupt ComfyUI".to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.send(
                self.request(reqwest::Method::POST, "/interrupt")
                    .timeout(Duration::from_secs(5))
                    .json(&json!({"prompt_id": id})),
            )?;
        } else if !response.status().is_success() {
            return Err("ComfyUI interrupt failed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_uses_save_node_and_reports_failures() {
        let history = json!({"job": {"outputs": {"preview": {"images": [{"filename": "wrong.png"}]}, "save": {"images": [{"filename": "right.webp"}]}}, "status": {"completed": true}}});
        assert_eq!(
            output_from_history(&history, "job", "save")
                .unwrap()
                .unwrap()["filename"],
            "right.webp"
        );
        assert!(
            output_from_history(&history, "missing", "save")
                .unwrap()
                .is_none()
        );
        assert!(
            output_from_history(&history, "job", "missing")
                .unwrap_err()
                .contains("without")
        );
        let error = json!({"job": {"status": {"status_str": "error", "messages": [["execution_error", {"node_type": "Sampler", "exception_message": "Out of memory"}]]}}});
        assert!(
            output_from_history(&error, "job", "save")
                .unwrap_err()
                .contains("Out of memory")
        );
    }
}

#[cfg(test)]
pub fn output_from_history(
    history: &Value,
    id: &str,
    save_node: &str,
) -> Result<Option<Value>, String> {
    output_from_history_kind(history, id, save_node, false)
}

pub fn output_from_history_kind(history: &Value, id: &str, save_node: &str, video: bool) -> Result<Option<Value>, String> {
    let Some(item) = history.get(id) else {
        return Ok(None);
    };
    if item["status"]["status_str"] == "error" {
        let messages = item["status"]["messages"].as_array();
        let error = messages
            .and_then(|messages| messages.iter().find(|m| m[0] == "execution_error"))
            .map(|m| {
                format!(
                    "{}: {}",
                    m[1]["node_type"].as_str().unwrap_or("node"),
                    m[1]["exception_message"]
                        .as_str()
                        .unwrap_or("Execution failed")
                )
            })
            .unwrap_or_else(|| "ComfyUI execution failed or was interrupted".into());
        return Err(error);
    }
    let output = if video {
        ["gifs", "videos", "images"].iter().filter_map(|key| item["outputs"][save_node][key].as_array())
            .flatten().find(|v| v["filename"].as_str().is_some_and(|s| s.to_ascii_lowercase().ends_with(".mp4"))).cloned()
    } else {
        item["outputs"][save_node]["images"].as_array().and_then(|images| images.first()).cloned()
    };
    if output.is_none() && item["status"]["completed"] == true {
        return Err(format!("ComfyUI finished without a {} result", if video { "MP4 video" } else { "SaveImage" }));
    }
    Ok(output)
}

#[cfg(test)]
mod video_tests {
    use super::*;
    #[test]
    fn video_history_selects_mp4_from_save_node_and_reports_missing_video() {
        let history = json!({"job": {"outputs": {
            "preview": {"gifs": [{"filename": "wrong.mp4"}]},
            "save": {"gifs": [{"filename": "preview.png"}, {"filename": "right.mp4", "subfolder": "ltx", "type": "output"}]}
        }, "status": {"completed": true}}});
        let output = output_from_history_kind(&history, "job", "save", true).unwrap().unwrap();
        assert_eq!(output["filename"], "right.mp4");
        assert_eq!(output["subfolder"], "ltx");
        assert!(output_from_history_kind(&history, "job", "missing", true).unwrap_err().contains("MP4"));
        assert!(output_from_history_kind(&history, "pending", "save", true).unwrap().is_none());
    }
}
