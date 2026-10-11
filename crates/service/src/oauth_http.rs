//! OAuth HTTP adapter. Errors contain status codes, never response bodies or tokens.
use ccsl_core::codex::oauth::transport::{
    endpoints, DeviceCode, DevicePoll, OAuthTransport, TokenResponse, TransportError,
};
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};
pub const USER_AGENT: &str = concat!("LeanFlip/", env!("CARGO_PKG_VERSION"));
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
    pub fn at(base: &str) -> Self {
        Self {
            usercode_url: format!(
                "{}/api/accounts/deviceauth/usercode",
                base.trim_end_matches('/')
            ),
            device_token_url: format!(
                "{}/api/accounts/deviceauth/token",
                base.trim_end_matches('/')
            ),
            oauth_token_url: format!("{}/oauth/token", base.trim_end_matches('/')),
            ..Default::default()
        }
    }
}
pub struct HttpTransport {
    client: Client,
    endpoints: Endpoints,
}
fn invalid(message: &str) -> TransportError {
    TransportError::Other(message.into())
}
fn field<'a>(body: &'a Value, key: &str) -> Result<&'a str, TransportError> {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("OAuth response is missing a required field"))
}
impl HttpTransport {
    pub fn new(endpoints: Endpoints) -> Result<Self, String> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Unable to initialize OAuth HTTP client".to_string())?;
        Ok(Self { client, endpoints })
    }
    fn send(&self, request: RequestBuilder) -> Result<(u16, Value), TransportError> {
        let response = request
            .send()
            .map_err(|_| invalid("OAuth network request failed"))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid("Unable to read OAuth response"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("OAuth response exceeds the size limit"));
        }
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Ok((status, value))
    }
    fn token(
        &self,
        fields: &[(&str, &str)],
        refresh: bool,
    ) -> Result<TokenResponse, TransportError> {
        let (status, body) = self.send(
            self.client
                .post(&self.endpoints.oauth_token_url)
                .form(fields),
        )?;
        if !(200..300).contains(&status) {
            let code = body
                .get("error")
                .and_then(|v| v.as_str().or_else(|| v.get("code").and_then(Value::as_str)))
                .or_else(|| body.get("code").and_then(Value::as_str));
            if refresh
                && ([401, 403].contains(&status)
                    || code.is_some_and(|c| {
                        c == "invalid_grant" || endpoints::INVALID_REFRESH_CODES.contains(&c)
                    }))
            {
                return Err(TransportError::RefreshTokenInvalid);
            }
            return Err(invalid(&format!(
                "OAuth token request returned HTTP {status}"
            )));
        }
        let token: TokenResponse =
            serde_json::from_value(body).map_err(|_| invalid("Invalid OAuth token response"))?;
        if token.access_token.trim().is_empty() {
            return Err(invalid("OAuth response has no access token"));
        }
        Ok(token)
    }
}
impl OAuthTransport for HttpTransport {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError> {
        let (status, body) = self.send(
            self.client
                .post(&self.endpoints.usercode_url)
                .json(&json!({"client_id":self.endpoints.client_id})),
        )?;
        if !(200..300).contains(&status) {
            return Err(invalid(&format!(
                "OAuth device request returned HTTP {status}"
            )));
        }
        let interval = body
            .get("interval")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
            .unwrap_or(5)
            .max(1)
            .saturating_add(3);
        Ok(DeviceCode {
            device_auth_id: field(&body, "device_auth_id")?.into(),
            user_code: field(&body, "user_code")?.into(),
            expires_in: body
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(900),
            interval,
        })
    }
    fn poll_device_code(
        &self,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<DevicePoll, TransportError> {
        let (status, body) = self.send(
            self.client
                .post(&self.endpoints.device_token_url)
                .json(&json!({"device_auth_id":device_auth_id,"user_code":user_code})),
        )?;
        match status {
            403 | 404 => Ok(DevicePoll::Pending),
            410 => Err(TransportError::Expired),
            200..=299 => Ok(DevicePoll::Authorized {
                authorization_code: field(&body, "authorization_code")?.into(),
                code_verifier: field(&body, "code_verifier")?.into(),
            }),
            _ => Err(invalid(&format!("OAuth poll returned HTTP {status}"))),
        }
    }
    fn exchange_code(
        &self,
        authorization_code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, TransportError> {
        self.token(
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &self.endpoints.client_id),
                ("redirect_uri", &self.endpoints.redirect_uri),
                ("code", authorization_code),
                ("code_verifier", code_verifier),
            ],
            false,
        )
    }
    fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TransportError> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &self.endpoints.client_id),
                ("refresh_token", refresh_token),
                ("scope", endpoints::REFRESH_SCOPE),
            ],
            true,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    fn response(status: u16, body: &str) -> (HttpTransport, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = vec![0; 16384];
            let n = stream.read(&mut data).unwrap();
            let request = String::from_utf8_lossy(&data[..n]).into_owned();
            write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            request
        });
        (
            HttpTransport::new(Endpoints::at(&format!("http://{address}"))).unwrap(),
            handle,
        )
    }
    #[test]
    fn failures_never_echo_server_secrets_and_invalid_grant_is_rejected() {
        let (t, h) = response(500, r#"{"error":"credential-secret-sentinel"}"#);
        let error = t.request_device_code().unwrap_err().to_string();
        assert!(!error.contains("sentinel"));
        assert!(error.contains("500"));
        h.join().unwrap();
        let (t, h) = response(400, r#"{"error":"invalid_grant"}"#);
        assert_eq!(
            t.refresh("fictional-refresh").unwrap_err(),
            TransportError::RefreshTokenInvalid
        );
        let request = h.join().unwrap();
        assert!(request.contains("refresh_token"));
    }
    #[test]
    fn device_flow_accepts_interval_strings_and_expiration() {
        let (t, h) = response(
            200,
            r#"{"device_auth_id":"fictional-device","user_code":"ABCD","interval":"7","expires_in":42}"#,
        );
        let result = t.request_device_code().unwrap();
        assert_eq!(result.interval, 10);
        assert_eq!(result.expires_in, 42);
        let request = h.join().unwrap();
        assert!(request.to_ascii_lowercase().contains("leanflip/"));
        let (t, h) = response(410, "{}");
        assert_eq!(
            t.poll_device_code("fictional", "ABCD").unwrap_err(),
            TransportError::Expired
        );
        h.join().unwrap();
    }
}
