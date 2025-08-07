use futures::{SinkExt, StreamExt};
use reqwest::{Client, Response};
use serde::Serialize;
use std::time::Duration;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub struct TestHttpClient {
    pub client: Client,
    base_url: String,
}

impl TestHttpClient {
    pub(crate) fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        Self {
            client,
            base_url: base_url.to_string(),
        }
    }

    pub(crate) async fn post<T: Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, reqwest::Error> {
        let url = format!("{}{}", self.base_url, path);
        self.client.post(&url).json(body).send().await
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Response, reqwest::Error> {
        let url = format!("{}{}", self.base_url, path);
        self.client.get(&url).send().await
    }
}

pub struct TestWebSocketClient {
    ws_stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl TestWebSocketClient {
    pub(crate) async fn connect(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let (ws_stream, _) = connect_async(url).await?;
        Ok(Self { ws_stream })
    }

    pub(crate) async fn send_json<T: Serialize>(
        &mut self,
        data: &T,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(data)?;
        self.ws_stream.send(Message::Text(json)).await?;
        Ok(())
    }

    pub(crate) async fn receive_json<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Result<T, Box<dyn std::error::Error>> {
        while let Some(msg) = self.ws_stream.next().await {
            match msg? {
                Message::Text(text) => {
                    return Ok(serde_json::from_str(&text)?);
                }
                Message::Close(_) => {
                    return Err("WebSocket closed".into());
                }
                _ => continue,
            }
        }
        Err("No message received".into())
    }

    pub(crate) async fn close(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.ws_stream.close(None).await?;
        Ok(())
    }
}
