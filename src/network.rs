use crate::{fetch_all, now_seconds, quota_json, summary};
use gauge::{calendar, config, stocks};
use serde_json::{json, Value};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    thread,
    time::Duration,
};

const MAX_REQUEST_SIZE: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run_wifi_server(bind: String, port: u16, token: String) -> Result<(), String> {
    let listener = TcpListener::bind((bind.as_str(), port))
        .map_err(|error| format!("failed to bind {bind}:{port}: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read listener address: {error}"))?;

    println!("Gauge Wi-Fi API running at http://{address}");

    loop {
        let (client, peer) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Wi-Fi listener error: {error}")),
        };
        let token = token.clone();

        thread::Builder::new()
            .name("gauge-http".into())
            .spawn(move || {
                if let Err(error) = handle_wifi_client(client, &token) {
                    eprintln!("Wi-Fi request from {peer} failed: {error}");
                }
            })
            .map_err(|error| format!("failed to start Wi-Fi handler: {error}"))?;
    }
}

pub fn run_udp_server(bind: String, port: u16, token: String) -> Result<(), String> {
    let socket = UdpSocket::bind((bind.as_str(), port))
        .map_err(|error| format!("failed to bind {bind}:{port}: {error}"))?;
    let address = socket
        .local_addr()
        .map_err(|error| format!("failed to read UDP address: {error}"))?;

    println!("Gauge BLE-style UDP API running at {address}");

    let mut buffer = [0_u8; MAX_REQUEST_SIZE + 1];
    loop {
        let (size, peer) = match socket.recv_from(&mut buffer) {
            Ok(message) => message,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("UDP receive error: {error}")),
        };
        let response = if size > MAX_REQUEST_SIZE {
            Response::error("request_too_large")
        } else {
            std::str::from_utf8(&buffer[..size])
                .map(|message| handle_ble_message(message, &token))
                .unwrap_or_else(|_| Response::error("invalid_utf8"))
        };

        if let Err(error) = socket.send_to(response.body.as_bytes(), peer) {
            eprintln!("UDP response to {peer} failed: {error}");
        }
    }
}

fn handle_wifi_client(mut stream: TcpStream, token: &str) -> Result<(), String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|error| format!("failed to set socket timeout: {error}"))?;

    let request_bytes = match read_http_request(&mut stream) {
        Ok(bytes) => bytes,
        Err(ReadRequestError::TooLarge) => {
            return send_http_response(
                &mut stream,
                Response::with_status(431, json!({"error": "request_too_large"})),
            )
        }
        Err(ReadRequestError::TimedOut) => {
            return send_http_response(
                &mut stream,
                Response::with_status(408, json!({"error": "request_timeout"})),
            )
        }
        Err(ReadRequestError::Invalid(error)) => return Err(error),
    };
    let request = match std::str::from_utf8(&request_bytes) {
        Ok(request) => request,
        Err(_) => return send_http_response(&mut stream, Response::error("invalid_utf8")),
    };
    let request = match Request::parse(request, true) {
        Ok(request) => request,
        Err(_) => return send_http_response(&mut stream, Response::error("bad_request")),
    };
    let response = dispatch(&request, token, Transport::Http);
    send_http_response(&mut stream, response)
}

fn handle_ble_message(message: &str, token: &str) -> Response {
    Request::parse(message, false)
        .map(|request| dispatch(&request, token, Transport::LegacyUdp))
        .unwrap_or_else(|_| Response::error("bad_request"))
}

#[derive(Clone, Copy)]
enum Transport {
    Http,
    LegacyUdp,
}

fn dispatch(request: &Request<'_>, token: &str, transport: Transport) -> Response {
    if !known_path(request.path, transport) {
        return Response::with_status(404, json!({"error": "not_found", "path": request.path}));
    }
    if request.method != "GET" {
        return Response::with_status(405, json!({"error": "method_not_allowed"}));
    }
    if request.path != "/health" && !authorized(request, token) {
        return Response::with_status(401, json!({"error": "unauthorized"}));
    }

    match request.path {
        "/" if matches!(transport, Transport::Http) => Response::ok(json!({
            "protocol": "http-json",
            "endpoints": ["/v1/dashboard", "/v1/quota", "/v1/summary", "/health"]
        })),
        "/" => Response::ok(json!({"endpoints": ["/v1/quota", "/v1/summary", "/health"]})),
        "/health" => Response::ok(json!({"status": "ok", "generated_at": now_seconds()})),
        "/v1/dashboard" => dashboard_response(),
        "/v1/quota" => {
            let (usages, errors) = fetch_all();
            Response {
                status: 200,
                body: quota_json(&usages, &errors),
            }
        }
        "/v1/summary" => {
            let (usages, errors) = fetch_all();
            Response::ok(json!({
                "generated_at": now_seconds(),
                "summary": summary(&usages),
                "errors": errors,
            }))
        }
        _ => unreachable!(),
    }
}

fn known_path(path: &str, transport: Transport) -> bool {
    matches!(path, "/" | "/health" | "/v1/quota" | "/v1/summary")
        || path == "/v1/dashboard" && matches!(transport, Transport::Http)
}

fn dashboard_response() -> Response {
    let config = match config::load_or_create() {
        Ok(config) => config,
        Err(error) => {
            return Response::with_status(
                500,
                json!({"error": "settings_unavailable", "detail": error}),
            )
        }
    };

    let (usages, quota_errors) = fetch_all();
    let providers = usages
        .iter()
        .map(|usage| {
            json!({
                "name": usage.name,
                "remaining_percent": usage.remaining_percent(),
                "limits": usage.limits.iter().map(|limit| json!({
                    "label": limit.label,
                    "used_percent": limit.used_percent,
                    "remaining_percent": limit.remaining_percent(),
                    "resets_at": limit.resets_at,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    let (calendar_events, calendar_error) = match calendar::fetch(&config.calendar) {
        Ok(events) => (events, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let calendar_events = calendar_events
        .iter()
        .map(|event| {
            json!({
                "title": event.title,
                "starts_at": event.starts_at as i64,
                "ends_at": event.ends_at as i64,
                "all_day": event.all_day,
            })
        })
        .collect::<Vec<_>>();

    let (quotes, stock_errors) = stocks::fetch(&config.stocks);
    let quotes = quotes
        .iter()
        .map(|quote| {
            json!({
                "symbol": quote.symbol,
                "label": quote.label(),
                "price": quote.price,
                "change_percent": quote.change_percent,
            })
        })
        .collect::<Vec<_>>();

    let todos = config
        .todos
        .iter()
        .enumerate()
        .map(|(id, todo)| {
            json!({
                "id": id,
                "title": todo.title,
                "completed": todo.completed,
            })
        })
        .collect::<Vec<_>>();

    Response::ok(json!({
        "schema_version": 1,
        "generated_at": now_seconds(),
        "refresh_seconds": config.refresh_seconds,
        "quota": {
            "summary": summary(&usages),
            "providers": providers,
            "errors": quota_errors,
        },
        "calendar": {
            "enabled": config.calendar.enabled,
            "events": calendar_events,
            "error": calendar_error,
        },
        "tickers": {
            "enabled": config.stocks.enabled,
            "quotes": quotes,
            "errors": stock_errors,
        },
        "todos": todos,
    }))
}

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    header_block: &'a str,
}

impl<'a> Request<'a> {
    fn parse(input: &'a str, require_http_version: bool) -> Result<Self, ()> {
        let (request_line, header_block) = input.split_once('\n').unwrap_or((input, ""));
        let mut parts = request_line.trim_end_matches('\r').split_whitespace();
        let method = parts.next().ok_or(())?;
        let target = parts.next().ok_or(())?;
        let version = parts.next();

        if parts.next().is_some()
            || require_http_version
                && !version.is_some_and(|value| matches!(value, "HTTP/1.0" | "HTTP/1.1"))
            || !require_http_version && version.is_some()
            || !target.starts_with('/')
        {
            return Err(());
        }

        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        for line in header_block.lines() {
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').ok_or(())?;
            if name.is_empty() || name.trim() != name || value.contains(['\r', '\n']) {
                return Err(());
            }
        }

        Ok(Self {
            method,
            path,
            query,
            header_block,
        })
    }

    fn header(&self, expected: &str) -> Option<&'a str> {
        self.header_block
            .lines()
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| name.eq_ignore_ascii_case(expected).then(|| value.trim()))
    }
}

struct Response {
    status: u16,
    body: String,
}

impl Response {
    fn ok(value: Value) -> Self {
        Self::with_status(200, value)
    }

    fn error(error: &str) -> Self {
        Self::with_status(400, json!({"error": error}))
    }

    fn with_status(status: u16, value: Value) -> Self {
        Self {
            status,
            body: value.to_string(),
        }
    }
}

fn authorized(request: &Request<'_>, expected: &str) -> bool {
    let bearer = request.header("authorization").and_then(|value| {
        let mut parts = value.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(scheme), Some(token), None) if scheme.eq_ignore_ascii_case("bearer") => {
                Some(token)
            }
            _ => None,
        }
    });
    let supplied = bearer
        .or_else(|| request.header("x-gauge-token"))
        .or_else(|| query_token(request.query));

    supplied.is_some_and(|token| token == expected)
}

fn query_token(query: &str) -> Option<&str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token" && !value.is_empty()).then_some(value)
    })
}

enum ReadRequestError {
    TooLarge,
    TimedOut,
    Invalid(String),
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, ReadRequestError> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];

    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                return Err(ReadRequestError::Invalid(
                    "connection closed before request headers completed".into(),
                ))
            }
            Ok(size) => request.extend_from_slice(&buffer[..size]),
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err(ReadRequestError::TimedOut)
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(ReadRequestError::Invalid(format!("read error: {error}"))),
        }

        if request.len() > MAX_REQUEST_SIZE {
            return Err(ReadRequestError::TooLarge);
        }
        if request.windows(4).any(|chunk| chunk == b"\r\n\r\n")
            || request.windows(2).any(|chunk| chunk == b"\n\n")
        {
            return Ok(request);
        }
    }
}

fn send_http_response(stream: &mut TcpStream, response: Response) -> Result<(), String> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let allow = if response.status == 405 {
        "Allow: GET\r\n"
    } else {
        ""
    };
    let response_head = format!(
        "HTTP/1.1 {} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         {allow}\r\n",
        response.status,
        response.body.len(),
    );

    stream
        .write_all(response_head.as_bytes())
        .and_then(|_| stream.write_all(response.body.as_bytes()))
        .and_then(|_| stream.flush())
        .map_err(|error| format!("response write error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_request_and_authentication() {
        let request = Request::parse(
            "GET /v1/quota?unused=true HTTP/1.1\r\nAuthorization: bearer secret\r\n\r\n",
            true,
        )
        .unwrap();

        assert_eq!(request.path, "/v1/quota");
        assert!(authorized(&request, "secret"));
    }

    #[test]
    fn only_accepts_documented_authentication_forms() {
        let raw = Request::parse("GET /v1/quota\nAuthorization: secret\n", false).unwrap();
        let query = Request::parse("GET /v1/quota?token=secret\n", false).unwrap();

        assert!(!authorized(&raw, "secret"));
        assert!(authorized(&query, "secret"));
    }

    #[test]
    fn rejects_malformed_requests() {
        assert!(Request::parse("GET /v1/quota", true).is_err());
        assert!(Request::parse("POST", false).is_err());
        assert!(Request::parse("GET relative", false).is_err());
    }

    #[test]
    fn dashboard_is_http_json_only() {
        assert!(known_path("/v1/dashboard", Transport::Http));
        assert!(!known_path("/v1/dashboard", Transport::LegacyUdp));
    }
}
