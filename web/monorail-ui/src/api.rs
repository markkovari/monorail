//! Thin fetch layer over the sink's HTTP API (ADR 0011). All request and
//! response bodies are `monorail-core` types — the exact structs the server
//! serializes, no codegen in between.

use serde::de::DeserializeOwned;
use serde::Serialize;

pub type ApiResult<T> = Result<T, String>;

pub async fn get_json<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(format!("{} on {path}", response.status()));
    }
    response.json::<T>().await.map_err(|e| e.to_string())
}

pub async fn send_json<B: Serialize, T: DeserializeOwned>(
    method: &str,
    path: &str,
    body: &B,
) -> ApiResult<T> {
    let builder = match method {
        "PUT" => gloo_net::http::Request::put(path),
        _ => gloo_net::http::Request::post(path),
    };
    let response = builder
        .json(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        // Surface the server's body (e.g. a nack reason) in the error.
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub async fn post_empty<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    let response = gloo_net::http::Request::post(path)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
