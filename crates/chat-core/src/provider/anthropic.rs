use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::json;

use super::{DeltaSink, Provider, ProviderError};
use crate::ChatRequest;
use crate::sse::SseDecoder;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

/// Anthropic Messages API with `stream: true`.
#[derive(Clone, Debug)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, DEFAULT_BASE_URL.to_owned())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url,
        }
    }

    async fn run(&self, request: ChatRequest, sink: DeltaSink) -> Result<(), ProviderError> {
        let messages: Vec<_> = request
            .messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();
        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
            "stream": true,
        });
        if !request.system_prompt.trim().is_empty() {
            body["system"] = json!(request.system_prompt);
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiEvent>(&text)
                .ok()
                .and_then(|e| e.error)
                .map_or(text, |e| e.message);
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let mut decoder = SseDecoder::default();
        let mut bytes = response.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            for data in decoder.push(&chunk?) {
                let event: ApiEvent = serde_json::from_str(&data)
                    .map_err(|e| ProviderError::Protocol(format!("{e}: {data}")))?;
                match event.kind.as_str() {
                    "content_block_delta" => {
                        if let Some(text) = event.delta.and_then(|d| d.text) {
                            sink.send(text);
                        }
                    }
                    "message_stop" => return Ok(()),
                    "error" => {
                        let message = event.error.map(|e| e.message).unwrap_or_default();
                        return Err(ProviderError::Api {
                            status: 200,
                            message,
                        });
                    }
                    _ => {}
                }
            }
        }
        Err(ProviderError::Protocol(
            "stream ended before message_stop".to_owned(),
        ))
    }
}

impl Provider for AnthropicProvider {
    fn stream(
        &self,
        request: ChatRequest,
        sink: DeltaSink,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(self.run(request, sink))
    }
}

#[derive(Deserialize)]
struct ApiEvent {
    #[serde(rename = "type")]
    kind: String,
    delta: Option<Delta>,
    error: Option<ApiErrorBody>,
}

#[derive(Deserialize)]
struct Delta {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{Message, StreamEvent};

    /// Serves one canned HTTP response and returns the raw request it received.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            // Read until the JSON body (which ends with `}`) has arrived.
            while !request.ends_with(b"}") {
                let n = socket.read(&mut buf).await.unwrap();
                request.extend_from_slice(&buf[..n]);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, handle)
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "test-model".into(),
            system_prompt: "be brief".into(),
            max_tokens: 16,
            messages: vec![Message::user("hi")],
        }
    }

    fn collect(rx: &mut mpsc::UnboundedReceiver<StreamEvent>) -> String {
        let mut out = String::new();
        while let Ok(StreamEvent::Delta(text)) = rx.try_recv() {
            out.push_str(&text);
        }
        out
    }

    #[tokio::test]
    async fn streams_text_deltas() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let (url, server) = serve_once("200 OK", body).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let provider = AnthropicProvider::with_base_url("secret".into(), url);

        provider.stream(request(), DeltaSink(tx)).await.unwrap();

        assert_eq!(collect(&mut rx), "Hello");
        let raw = server.await.unwrap();
        assert!(raw.contains("x-api-key: secret"));
        assert!(raw.contains("anthropic-version: 2023-06-01"));
        assert!(raw.contains(r#""system":"be brief""#));
        assert!(raw.contains(r#""stream":true"#));
    }

    #[tokio::test]
    async fn reports_api_errors() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        let (url, _server) = serve_once("401 Unauthorized", body).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let provider = AnthropicProvider::with_base_url("bad".into(), url);

        let err = provider.stream(request(), DeltaSink(tx)).await.unwrap_err();
        assert_eq!(err.to_string(), "API error (401): invalid x-api-key");
    }

    #[tokio::test]
    async fn truncated_stream_is_an_error() {
        let body = "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"x\"}}\n\n";
        let (url, _server) = serve_once("200 OK", body).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let provider = AnthropicProvider::with_base_url("k".into(), url);

        let err = provider.stream(request(), DeltaSink(tx)).await.unwrap_err();
        assert!(matches!(err, ProviderError::Protocol(_)));
    }
}
