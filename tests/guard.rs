//! The dashboard's request guards over real HTTP: Host, Origin and the token.

use std::sync::Arc;
use stdf_cypher::api::Registry;
use stdf_cypher::server::{router, Guard};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Keep tests away from the user's real settings/cache folders.
fn isolate() {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let h = HOME.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("STDF_CYPHER_HOME", h.path());
}

/// Starts the dashboard on a free loopback port; returns the port and the token.
async fn start() -> (u16, String) {
    isolate();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let guard = Guard::new("127.0.0.1", port);
    let token = guard.token().to_string();
    let reg = Arc::new(Registry::app());
    tokio::spawn(async move {
        axum::serve(listener, router(reg, guard)).await.unwrap();
    });
    (port, token)
}

/// One raw HTTP/1.1 request; returns the status code and the body.
async fn request(port: u16, lines: &[&str], body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut req = lines.join("\r\n");
    req.push_str(&format!(
        "\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = out
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
        .to_string();
    (status, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_is_served_to_the_own_host_only() {
    let (port, token) = start().await;
    let own = format!("Host: 127.0.0.1:{port}");
    let (s, body) = request(port, &["GET / HTTP/1.1", &own], "").await;
    assert_eq!(s, 200);
    assert!(body.contains(&format!("window.STDF_TOKEN = \"{token}\"")));
    let (s, _) = request(
        port,
        &["GET / HTTP/1.1", &format!("Host: localhost:{port}")],
        "",
    )
    .await;
    assert_eq!(s, 200, "localhost is this server too");
    // a DNS-rebound name reaches the same port: refused, so the page and token stay unread
    let (s, body) = request(
        port,
        &["GET / HTTP/1.1", &format!("Host: evil.example:{port}")],
        "",
    )
    .await;
    assert_eq!(s, 403);
    assert!(!body.contains(&token));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_needs_the_token_and_a_same_site_origin() {
    let (port, token) = start().await;
    let own = format!("Host: 127.0.0.1:{port}");
    let api = "POST /api/retest_trace HTTP/1.1";
    let json = "Content-Type: application/json";
    let (s, _) = request(port, &[api, &own, json], "{}").await;
    assert_eq!(s, 401, "no token");
    let (s, _) = request(port, &[api, &own, json, "X-STDF-Token: nope"], "{}").await;
    assert_eq!(s, 401, "wrong token");
    let good = format!("X-STDF-Token: {token}");
    let (s, _) = request(port, &[api, &own, json, &good], "{}").await;
    assert_eq!(s, 200, "right token reaches the op");
    // a page from another site is refused even with the token
    let (s, _) = request(
        port,
        &[api, &own, "Origin: http://evil.example", json, &good],
        "{}",
    )
    .await;
    assert_eq!(s, 403);
    let (s, _) = request(port, &[api, &own, "Origin: null", json, &good], "{}").await;
    assert_eq!(s, 403, "sandboxed or file pages send Origin: null");
    // the same origin passes
    let (s, _) = request(
        port,
        &[
            api,
            &own,
            &format!("Origin: http://127.0.0.1:{port}"),
            json,
            &good,
        ],
        "{}",
    )
    .await;
    assert_eq!(s, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn links_carry_the_token_in_the_query() {
    let (port, token) = start().await;
    let own = format!("Host: 127.0.0.1:{port}");
    let (s, _) = request(
        port,
        &[&format!("GET /api/export.csv?t={token} HTTP/1.1"), &own],
        "",
    )
    .await;
    assert!(
        s != 401 && s != 403,
        "the token in the query passes the guard"
    );
    let (s, _) = request(port, &["GET /api/export.csv?t=bad HTTP/1.1", &own], "").await;
    assert_eq!(s, 401);
    let (s, _) = request(port, &["GET /api/export.csv HTTP/1.1", &own], "").await;
    assert_eq!(s, 401);
}

#[test]
fn guard_accepts_the_address_it_was_started_on() {
    let lan = Guard::new("192.168.1.5", 8765);
    assert!(lan.hosts().contains(&"192.168.1.5:8765".to_string()));
    assert!(lan.hosts().contains(&"localhost:8765".to_string()));
    let any = Guard::new("0.0.0.0", 8765);
    assert!(!any.hosts().iter().any(|h| h.starts_with("0.0.0.0")));
    let port80 = Guard::new("127.0.0.1", 80);
    assert!(
        port80.hosts().contains(&"127.0.0.1".to_string()),
        "browsers send Host without :80"
    );
    assert_ne!(
        Guard::new("127.0.0.1", 1).token(),
        Guard::new("127.0.0.1", 1).token(),
        "a new token per launch"
    );
    assert_eq!(Guard::new("127.0.0.1", 1).token().len(), 32);
}
