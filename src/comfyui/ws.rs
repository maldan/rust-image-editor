use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use base64::Engine;
use serde_json::Value;
use tungstenite::{Message, client::IntoClientRequest, stream::MaybeTlsStream};

use super::{HostConfig, job::Progress};

pub fn listen(
    config: HostConfig,
    client_id: String,
    prompt_id: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
) {
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let connected = connect(&config, &client_id);
        let _ = ready_tx.send(());
        let Ok(mut socket) = connected else { return };
        let mut ours = false;
        while !stop.load(Ordering::Relaxed) {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let Ok(event) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    let data = &event["data"];
                    let id = prompt_id.lock().unwrap().clone();
                    if let Some(event_id) = data["prompt_id"].as_str() {
                        // Events may arrive before POST /prompt has returned its id.
                        ours = id.as_deref().is_none_or(|id| id == event_id);
                    }
                    if !ours {
                        continue;
                    }
                    match event["type"].as_str().unwrap_or("") {
                        "execution_start" | "executing" => {
                            progress.lock().unwrap().fraction = None;
                        }
                        "progress" => {
                            if let (Some(value), Some(max)) =
                                (data["value"].as_f64(), data["max"].as_f64())
                            {
                                if max > 0.0 {
                                    progress.lock().unwrap().fraction =
                                        Some((value / max).clamp(0.0, 1.0) as f32);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Message::Binary(bytes)) if ours => {
                    // Legacy PREVIEW_IMAGE: event type u32 + image type u32, big endian.
                    if bytes.len() > 8 && bytes[..4] == 1u32.to_be_bytes() {
                        if let Ok(image) = super::decode_image(&bytes[8..]) {
                            // Only retain the newest preview, bounded independently of UI polling.
                            let image = image::DynamicImage::ImageRgba8(image)
                                .thumbnail(1024, 1024)
                                .to_rgba8();
                            progress.lock().unwrap().preview = Some(image);
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break, // History polling remains authoritative if WS disconnects.
                _ => {}
            }
        }
        let _ = socket.close(None);
    });
    // Establish WS before submitting; missing WS never prevents history-based completion.
    let _ = ready_rx.recv_timeout(Duration::from_secs(6));
}

fn connect(
    config: &HostConfig,
    client_id: &str,
) -> Result<tungstenite::WebSocket<MaybeTlsStream<TcpStream>>, ()> {
    let mut url = reqwest::Url::parse(&config.ws).map_err(|_| ())?;
    url.query_pairs_mut().append_pair("clientId", client_id);
    let host = url.host_str().ok_or(())?;
    let port = url.port_or_known_default().ok_or(())?;
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|_| ())?
        .next()
        .ok_or(())?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|_| ())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| ())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| ())?;
    let mut request = url.as_str().into_client_request().map_err(|_| ())?;
    if !config.login.is_empty() {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", config.login, config.password));
        let mut value = format!("Basic {token}")
            .parse::<tungstenite::http::HeaderValue>()
            .map_err(|_| ())?;
        value.set_sensitive(true);
        request.headers_mut().insert("Authorization", value);
    }
    let ws_config =
        tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(32 * 1024 * 1024));
    let (socket, _) = tungstenite::client_tls_with_config(request, stream, Some(ws_config), None)
        .map_err(|_| ())?;
    let stream = match socket.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => &s.sock,
        _ => return Err(()),
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|_| ())?;
    Ok(socket)
}
