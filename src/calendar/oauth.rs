use http_client::{HttpClient, Request};
use oauth2::http::Error;
use oauth2::{basic::BasicClient, TokenResponse};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl,
    RefreshToken, RevocationUrl, Scope, TokenUrl,
};
use url::Url;

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub async fn access_token(http_client: &impl HttpClient, mut token_path: PathBuf) -> String {
    token_path.push(".rtoken");

    // Set up the config for the Google OAuth2 process.
    let client = BasicClient::new(
        ClientId::new(
            env::var("GOOGLE_OAUTH_CLIENT_ID")
                .expect("Missing the GOOGLE_OAUTH_CLIENT_ID environment variable."),
        ),
        Some(ClientSecret::new(
            env::var("GOOGLE_OAUTH_CLIENT_SECRET")
                .expect("Missing the GOOGLE_OAUTH_CLIENT_SECRET environment variable."),
        )),
        AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string())
            .expect("Invalid authorization endpoint URL"),
        Some(
            TokenUrl::new("https://www.googleapis.com/oauth2/v3/token".to_string())
                .expect("Invalid token endpoint URL"),
        ),
    )
    .set_redirect_uri(
        RedirectUrl::new("http://localhost:8080".to_string()).expect("Invalid redirect URL"),
    )
    .set_revocation_uri(
        RevocationUrl::new("https://oauth2.googleapis.com/revoke".to_string())
            .expect("Invalid revocation endpoint URL"),
    );

    let req = move |request: oauth2::HttpRequest| async move {
        let mut req = Request::new(
            match request.method {
                oauth2::http::Method::GET => http_client::http_types::Method::Get,
                oauth2::http::Method::POST => http_client::http_types::Method::Post,
                oauth2::http::Method::PUT => http_client::http_types::Method::Put,
                oauth2::http::Method::DELETE => http_client::http_types::Method::Delete,
                oauth2::http::Method::HEAD => http_client::http_types::Method::Head,
                oauth2::http::Method::OPTIONS => http_client::http_types::Method::Options,
                oauth2::http::Method::CONNECT => http_client::http_types::Method::Connect,
                oauth2::http::Method::PATCH => http_client::http_types::Method::Patch,
                oauth2::http::Method::TRACE => http_client::http_types::Method::Trace,
                _ => http_client::http_types::Method::Get,
            },
            request.url.as_str(),
        ); // TODO: handle other methods
        req.set_body(request.body);
        for (name, value) in &request.headers {
            let Ok(values) = value.to_str() else {
                eprintln!("Failed to convert header value to string: {:?}", value);
                continue;
            };
            req.insert_header(name.as_str(), values); // TODO Allow values that have not ASCII characters
        }
        let mut response = http_client.send(req).await.unwrap();
        let status_code = oauth2::http::StatusCode::from_u16(response.status() as u16)
            .expect("Should be a valid status code!");
        let body = response.take_body().into_bytes().await.unwrap();
        let headers = response
            .into_iter()
            .map(|(name, value)| {
                (
                    oauth2::http::header::HeaderName::from_str(name.as_str()).unwrap(),
                    oauth2::http::header::HeaderValue::from_str(value.as_str()).unwrap(),
                )
            })
            .collect();
        Ok::<_, Error>(oauth2::HttpResponse {
            status_code,
            headers,
            body,
        })
    };

    if let Some(rtoken) = check_rtoken(&token_path) {
        return client
            .exchange_refresh_token(&RefreshToken::new(rtoken))
            .request_async(req)
            .await
            .expect("Failed to exchange refresh token.")
            .access_token()
            .secret()
            .clone();
    }

    // Google supports Proof Key for Code Exchange (PKCE - https://oauth.net/2/pkce/).
    // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
    let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate the authorization URL to which we'll redirect the user.
    let (authorize_url, csrf_state) = client
        .authorize_url(CsrfToken::new_random)
        // This example is requesting access to the "calendar" features and the user's profile.
        .add_scope(Scope::new(
            "https://www.googleapis.com/auth/calendar".to_string(),
        ))
        .add_scope(Scope::new(
            "https://www.googleapis.com/auth/plus.me".to_string(),
        ))
        .set_pkce_challenge(pkce_code_challenge)
        .url();

    println!("🔗 Open this URL in your browser:\n{authorize_url}\n");

    let (code, state) = {
        // A very naive implementation of the redirect server.
        let listener = TcpListener::bind("127.0.0.1:8080").unwrap();

        // The server will terminate itself after collecting the first code.
        let Some(mut stream) = listener.incoming().flatten().next() else {
            panic!("listener terminated without accepting a connection");
        };

        let mut reader = BufReader::new(&stream);

        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();

        let redirect_url = request_line.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, code)| AuthorizationCode::new(code.into_owned()))
            .unwrap();

        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, state)| CsrfToken::new(state.into_owned()))
            .unwrap();

        let message = "Go back to your terminal :)";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{}",
            message.len(),
            message
        );
        stream.write_all(response.as_bytes()).unwrap();

        (code, state)
    };

    // Ensure the `state` in the response matches the `state` in the request.
    if csrf_state.secret() != state.secret() {
        panic!("CSRF token mismatch");
    }

    // Exchange the code with a token.
    let token_response = client
        .exchange_code(code)
        .set_pkce_verifier(pkce_code_verifier)
        .request_async(req)
        .await;

    let Ok(tokens) = token_response else {
        panic!("Token exchange failed: {:?}", token_response);
    };
    std::fs::write(&token_path, tokens.refresh_token().unwrap().secret())
        .expect("Failed to write refresh token to file.");

    tokens.access_token().secret().clone()
}

fn check_rtoken(token_path: &Path) -> Option<String> {
    std::fs::read_to_string(token_path).ok()
}
