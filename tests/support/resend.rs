//! A stand-in for Resend's HTTP API: a local HTTP server, standard library
//! only, that records each request and gives every one the reply its test
//! chose. Point `THIRDSHIFT_RESEND_URL` at [`ResendStandIn::url`].

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One request the stand-in received.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// The `Authorization` header, if there was one.
    pub authorization: Option<String>,
    pub body: Value,
}

pub struct ResendStandIn {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl ResendStandIn {
    /// Start a stand-in that answers every request with `status` and `body`.
    /// It serves until the test process exits.
    pub fn replying(status: u16, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let body = body.to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &recorded, status, &body);
            }
        });
        ResendStandIn { url, requests }
    }

    /// The base URL, as `THIRDSHIFT_RESEND_URL` takes it.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

/// Read one HTTP/1.1 request from `stream`, record it in `recorded`, reply,
/// and close the connection. Recording comes first, so the request is there
/// by the time thirdshift has its reply.
fn serve(stream: TcpStream, recorded: &Mutex<Vec<Request>>, status: u16, body: &str) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut authorization = None;
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "authorization" => authorization = Some(value.to_string()),
            "content-length" => length = value.parse().ok()?,
            _ => {}
        }
    }
    let mut raw = vec![0; length];
    reader.read_exact(&mut raw).ok()?;
    recorded.lock().unwrap().push(Request {
        method,
        path,
        authorization,
        body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
    });
    let reply = format!(
        "HTTP/1.1 {status} Stand-in\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = stream;
    stream.write_all(reply.as_bytes()).ok()
}
