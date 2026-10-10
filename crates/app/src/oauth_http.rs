//! OpenAI OAuth 的网络实现（reqwest blocking + rustls），给 core 的 `CodexOAuthManager`
//! 用。请求形状和 cc-switch `codex_oauth_auth.rs` 一致：
//! - 申请设备码：POST JSON `{client_id}`；
//! - 轮询：POST JSON `{device_auth_id, user_code}`，403/404 = 用户还没确认，410 = 过期；
//! - 换 token / 刷新：POST 表单（`grant_type=authorization_code` / `refresh_token`）；
//!   刷新返回 401/403 或 `refresh_token_expired|reused|invalidated` 时是 refresh token 失效。
//!
//! 地址可替换（测试里指向本地的假服务器）。

use std::time::Duration;

use ccsl_core::codex::oauth::transport::{
    endpoints, DeviceCode, DevicePoll, OAuthTransport, TokenResponse, TransportError,
};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

pub const USER_AGENT: &str = concat!("ccswitch-lite/", env!("CARGO_PKG_VERSION"));
/// 轮询间隔的安全余量（秒），和 cc-switch 一致。
const POLLING_SAFETY_MARGIN_SECS: u64 = 3;
const DEFAULT_DEVICE_EXPIRES_IN: u64 = 900;

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub usercode_url: String,
    pub device_token_url: String,
    pub oauth_token_url: String,
    pub redirect_uri: String,
    pub client_id: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            usercode_url: endpoints::DEVICE_AUTH_USERCODE_URL.into(),
            device_token_url: endpoints::DEVICE_AUTH_TOKEN_URL.into(),
            oauth_token_url: endpoints::OAUTH_TOKEN_URL.into(),
            redirect_uri: endpoints::DEVICE_REDIRECT_URI.into(),
            client_id: endpoints::CLIENT_ID.into(),
        }
    }
}

impl Endpoints {
    /// 全部指向 `base`（测试用）。
    pub fn at(base: &str) -> Self {
        Self {
            usercode_url: format!("{base}/api/accounts/deviceauth/usercode"),
            device_token_url: format!("{base}/api/accounts/deviceauth/token"),
            oauth_token_url: format!("{base}/oauth/token"),
            ..Self::default()
        }
    }
}

pub struct HttpTransport {
    client: Client,
    endpoints: Endpoints,
}

impl HttpTransport {
    pub fn new(endpoints: Endpoints) -> Result<Self, String> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client, endpoints })
    }
}

fn other(e: impl std::fmt::Display) -> TransportError {
    TransportError::Other(e.to_string())
}

fn parse_interval(value: Option<&Value>) -> u64 {
    let raw = match value {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(5),
        Some(Value::String(s)) => s.trim().parse::<u64>().unwrap_or(5),
        _ => 5,
    };
    raw.max(1) + POLLING_SAFETY_MARGIN_SECS
}

pub(crate) fn refresh_error_code(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|error| match error {
            Value::Object(object) => object.get("code").and_then(Value::as_str),
            Value::String(code) => Some(code.as_str()),
            _ => None,
        })
        .or_else(|| value.get("code").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<Value>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct PollSuccess {
    authorization_code: String,
    code_verifier: String,
}

fn token_response(response: reqwest::blocking::Response) -> Result<TokenResponse, TransportError> {
    let body: TokenResponse = response.json().map_err(other)?;
    if body.access_token.trim().is_empty() {
        return Err(other("token response has no access_token"));
    }
    Ok(body)
}

impl OAuthTransport for HttpTransport {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError> {
        let response = self
            .client
            .post(&self.endpoints.usercode_url)
            .json(&serde_json::json!({ "client_id": self.endpoints.client_id }))
            .send()
            .map_err(other)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(other(format!(
                "device code request failed: {status} {text}"
            )));
        }
        let body: UserCodeResponse = response.json().map_err(other)?;
        Ok(DeviceCode {
            device_auth_id: body.device_auth_id,
            user_code: body.user_code,
            expires_in: body.expires_in.unwrap_or(DEFAULT_DEVICE_EXPIRES_IN),
            interval: parse_interval(body.interval.as_ref()),
        })
    }

    fn poll_device_code(
        &self,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<DevicePoll, TransportError> {
        let response = self
            .client
            .post(&self.endpoints.device_token_url)
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .map_err(other)?;
        let status = response.status();
        if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
            return Ok(DevicePoll::Pending);
        }
        if status == StatusCode::GONE {
            return Err(TransportError::Expired);
        }
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(other(format!("device poll failed: {status} {text}")));
        }
        let body: PollSuccess = response.json().map_err(other)?;
        Ok(DevicePoll::Authorized {
            authorization_code: body.authorization_code,
            code_verifier: body.code_verifier,
        })
    }

    fn exchange_code(
        &self,
        authorization_code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, TransportError> {
        let response = self
            .client
            .post(&self.endpoints.oauth_token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", authorization_code),
                ("redirect_uri", self.endpoints.redirect_uri.as_str()),
                ("client_id", self.endpoints.client_id.as_str()),
                ("code_verifier", code_verifier),
            ])
            .send()
            .map_err(other)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(other(format!("token exchange failed: {status} {text}")));
        }
        token_response(response)
    }

    fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TransportError> {
        let response = self
            .client
            .post(&self.endpoints.oauth_token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", self.endpoints.client_id.as_str()),
                ("scope", endpoints::REFRESH_SCOPE),
            ])
            .send()
            .map_err(other)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            let code = refresh_error_code(&text);
            if status == StatusCode::UNAUTHORIZED
                || status == StatusCode::FORBIDDEN
                || code
                    .as_deref()
                    .is_some_and(|c| endpoints::INVALID_REFRESH_CODES.contains(&c))
            {
                return Err(TransportError::RefreshTokenInvalid);
            }
            // 不把响应体原样带出去（可能回显 token）：只留状态码和错误码。
            return Err(other(format!(
                "refresh failed: {status}{}",
                code.map(|c| format!(" ({c})")).unwrap_or_default()
            )));
        }
        token_response(response)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    #[derive(Debug)]
    struct Seen {
        line: String,
        headers: String,
        body: String,
    }

    /// 按顺序回答 `responses`（状态码, JSON 正文），把收到的请求发回来。
    fn server(responses: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut headers = String::new();
                let mut length = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    headers.push_str(&h);
                }
                let mut buf = vec![0; length];
                reader.read_exact(&mut buf).unwrap();
                tx.send(Seen {
                    line: line.trim().to_string(),
                    headers,
                    body: String::from_utf8(buf).unwrap(),
                })
                .unwrap();
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (base, rx)
    }

    fn transport(base: &str) -> HttpTransport {
        HttpTransport::new(Endpoints::at(base)).unwrap()
    }

    #[test]
    fn device_code_request_and_interval_parsing() {
        let (base, rx) = server(vec![
            (
                200,
                r#"{"device_auth_id":"dev-1","user_code":"AB-CD","interval":"7"}"#,
            ),
            (
                200,
                r#"{"device_auth_id":"dev-2","user_code":"EF","interval":0,"expires_in":600}"#,
            ),
        ]);
        let t = transport(&base);
        let code = t.request_device_code().unwrap();
        assert_eq!(code.device_auth_id, "dev-1");
        assert_eq!(code.user_code, "AB-CD");
        assert_eq!(code.interval, 7 + POLLING_SAFETY_MARGIN_SECS);
        assert_eq!(code.expires_in, DEFAULT_DEVICE_EXPIRES_IN);
        let seen = rx.recv().unwrap();
        assert_eq!(seen.line, "POST /api/accounts/deviceauth/usercode HTTP/1.1");
        assert!(seen
            .headers
            .to_ascii_lowercase()
            .contains("content-type: application/json"));
        assert!(seen.headers.contains(USER_AGENT), "{}", seen.headers);
        let body: Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(body, serde_json::json!({"client_id": endpoints::CLIENT_ID}));

        let code = t.request_device_code().unwrap();
        assert_eq!(code.interval, 1 + POLLING_SAFETY_MARGIN_SECS);
        assert_eq!(code.expires_in, 600);
    }

    #[test]
    fn polling_maps_statuses() {
        let (base, rx) = server(vec![
            (403, r#"{}"#),
            (404, r#"{}"#),
            (200, r#"{"authorization_code":"ac","code_verifier":"cv"}"#),
            (410, r#"{}"#),
            (500, r#"{}"#),
        ]);
        let t = transport(&base);
        assert!(matches!(
            t.poll_device_code("d", "u"),
            Ok(DevicePoll::Pending)
        ));
        let seen = rx.recv().unwrap();
        assert_eq!(seen.line, "POST /api/accounts/deviceauth/token HTTP/1.1");
        let body: Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"device_auth_id": "d", "user_code": "u"})
        );
        assert!(matches!(
            t.poll_device_code("d", "u"),
            Ok(DevicePoll::Pending)
        ));
        match t.poll_device_code("d", "u").unwrap() {
            DevicePoll::Authorized {
                authorization_code,
                code_verifier,
            } => assert_eq!(
                (authorization_code.as_str(), code_verifier.as_str()),
                ("ac", "cv")
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            t.poll_device_code("d", "u"),
            Err(TransportError::Expired)
        ));
        assert!(matches!(
            t.poll_device_code("d", "u"),
            Err(TransportError::Other(_))
        ));
    }

    #[test]
    fn exchange_sends_the_pkce_form() {
        let (base, rx) = server(vec![(
            200,
            r#"{"access_token":"at","refresh_token":"rt","id_token":"it","expires_in":3600}"#,
        )]);
        let tokens = transport(&base).exchange_code("ac", "cv").unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        let seen = rx.recv().unwrap();
        assert_eq!(seen.line, "POST /oauth/token HTTP/1.1");
        assert!(seen
            .headers
            .to_ascii_lowercase()
            .contains("content-type: application/x-www-form-urlencoded"));
        for part in [
            "grant_type=authorization_code",
            "code=ac",
            "code_verifier=cv",
            &format!("client_id={}", endpoints::CLIENT_ID),
            "redirect_uri=",
        ] {
            assert!(seen.body.contains(part), "{part} in {}", seen.body);
        }
    }

    #[test]
    fn refresh_form_and_invalid_token_detection() {
        let (base, rx) = server(vec![
            (200, r#"{"access_token":"at2","refresh_token":"rt2"}"#),
            (401, r#"{}"#),
            (
                400,
                r#"{"error":{"code":"REFRESH_TOKEN_REUSED","message":"echo rt-secret"}}"#,
            ),
            (400, r#"{"error":"refresh_token_expired"}"#),
            (400, r#"{"code":"refresh_token_invalidated"}"#),
            (
                503,
                r#"{"error":"temporarily_unavailable","echo":"rt-secret"}"#,
            ),
            (200, r#"{"access_token":""}"#),
        ]);
        let t = transport(&base);
        assert_eq!(t.refresh("rt-secret").unwrap().access_token, "at2");
        let seen = rx.recv().unwrap();
        assert!(seen.body.contains("grant_type=refresh_token"));
        assert!(seen.body.contains("refresh_token=rt-secret"));
        assert!(
            seen.body.contains("scope=openid+profile+email"),
            "{}",
            seen.body
        );
        for _ in 0..4 {
            assert!(matches!(
                t.refresh("rt-secret"),
                Err(TransportError::RefreshTokenInvalid)
            ));
        }
        match t.refresh("rt-secret") {
            Err(TransportError::Other(message)) => {
                assert!(!message.contains("rt-secret"), "{message}");
                assert!(message.contains("temporarily_unavailable"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(t.refresh("rt"), Err(TransportError::Other(_))));
    }

    #[test]
    fn default_endpoints_are_the_official_ones() {
        let e = Endpoints::default();
        assert!(e.usercode_url.starts_with("https://auth.openai.com/"));
        assert!(e.oauth_token_url.starts_with("https://auth.openai.com/"));
    }
}
