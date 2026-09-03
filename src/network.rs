//! Optional local-network service owned by the menu-bar app.
//!
//! There is no standalone server mode. When accessory sharing is enabled, the
//! tray process publishes this service with DNS-SD and serves its already
//! collected dashboard snapshot to independently authenticated devices.

use gauge::{
    devices::{
        DeviceStore, ACCESSORY_API_VERSION, ACCESSORY_PROTOCOL, DASHBOARD_PATH, SERVICE_TYPE,
    },
    now_seconds,
};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use serde_json::{json, Value};
use std::{
    io::{ErrorKind, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        mpsc::{self, Sender},
        Arc, RwLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_REQUEST_SIZE: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

// A UDP connect selects the interface macOS would use for ordinary LAN
// traffic without sending a packet. Publishing this one address avoids
// advertising loopback and virtual-interface addresses that an ESP32 cannot
// reach, while Bonjour still supplies the address dynamically by server ID.
fn active_lan_ipv4() -> Result<Ipv4Addr, String> {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|error| format!("could not inspect the active LAN address: {error}"))?;
    probe
        .connect((Ipv4Addr::new(192, 0, 2, 1), 9))
        .map_err(|error| format!("could not select the active LAN route: {error}"))?;
    match probe
        .local_addr()
        .map_err(|error| format!("could not read the active LAN address: {error}"))?
        .ip()
    {
        IpAddr::V4(address)
            if !address.is_loopback() && !address.is_unspecified() && !address.is_link_local() =>
        {
            Ok(address)
        }
        address => Err(format!(
            "no reachable IPv4 LAN address is active ({address})"
        )),
    }
}

pub struct AccessoryServer {
    address: SocketAddr,
    snapshot: Arc<RwLock<String>>,
    shutdown: Sender<()>,
    worker: Option<JoinHandle<()>>,
    mdns: ServiceDaemon,
    service_fullname: String,
}

impl AccessoryServer {
    pub fn start(
        port: u16,
        display_name: &str,
        devices: Arc<DeviceStore>,
        initial_dashboard: String,
    ) -> Result<Self, String> {
        if port == 0 {
            return Err("accessory server port must not be zero".into());
        }
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|error| {
            format!("could not start accessory sharing on port {port}: {error}")
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("could not configure accessory listener: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("could not inspect accessory listener: {error}"))?;

        let server_id = devices.server_id();
        let lan_address = active_lan_ipv4()?;
        let mdns = ServiceDaemon::new()
            .map_err(|error| format!("could not start Bonjour advertising: {error}"))?;
        let hostname = format!("gauge-{}.local.", &server_id[..12.min(server_id.len())]);
        let properties = [
            ("api", ACCESSORY_API_VERSION.to_string()),
            ("protocol", ACCESSORY_PROTOCOL.into()),
            ("path", DASHBOARD_PATH.into()),
            ("auth", "bearer".into()),
            ("pair", "ble-sc-numeric-v1".into()),
            ("id", server_id),
        ];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            display_name,
            &hostname,
            IpAddr::V4(lan_address),
            address.port(),
            &properties[..],
        )
        .map_err(|error| format!("could not describe the Bonjour service: {error}"))?;
        let service_fullname = service.get_fullname().to_string();
        mdns.register(service)
            .map_err(|error| format!("could not advertise Gauge with Bonjour: {error}"))?;

        let snapshot = Arc::new(RwLock::new(initial_dashboard));
        let worker_snapshot = Arc::clone(&snapshot);
        let (shutdown, stop) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("gauge-accessories".into())
            .spawn(move || loop {
                if stop.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((client, peer)) => {
                        let devices = Arc::clone(&devices);
                        let snapshot = Arc::clone(&worker_snapshot);
                        if let Err(error) = thread::Builder::new()
                            .name("gauge-accessory-request".into())
                            .spawn(move || {
                                if let Err(error) = handle_client(client, &devices, &snapshot) {
                                    eprintln!("accessory request from {peer} failed: {error}");
                                }
                            })
                        {
                            eprintln!("could not start accessory request handler: {error}");
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => {
                        eprintln!("accessory listener stopped: {error}");
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start accessory service: {error}"))?;

        Ok(Self {
            address,
            snapshot,
            shutdown,
            worker: Some(worker),
            mdns,
            service_fullname,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn update_dashboard(&self, dashboard: String) {
        *self
            .snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = dashboard;
    }
}

impl Drop for AccessoryServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = self.mdns.unregister(&self.service_fullname);
        let _ = self.mdns.shutdown();
    }
}

fn handle_client(
    mut stream: TcpStream,
    devices: &DeviceStore,
    snapshot: &RwLock<String>,
) -> Result<(), String> {
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
    let request = match Request::parse(request) {
        Ok(request) => request,
        Err(_) => return send_http_response(&mut stream, Response::error("bad_request")),
    };
    let response = dispatch(&request, devices, snapshot);
    send_http_response(&mut stream, response)
}

fn dispatch(request: &Request<'_>, devices: &DeviceStore, snapshot: &RwLock<String>) -> Response {
    if !matches!(request.path, "/health" | "/v1/accessory" | DASHBOARD_PATH) {
        return Response::with_status(404, json!({"error": "not_found", "path": request.path}));
    }

    match (request.method, request.path) {
        ("GET", "/health") => Response::ok(json!({
            "status": "ok",
            "protocol": ACCESSORY_PROTOCOL,
            "api_version": ACCESSORY_API_VERSION,
            "generated_at": now_seconds(),
        })),
        ("GET", "/v1/accessory") => Response::ok(json!({
            "protocol": ACCESSORY_PROTOCOL,
            "api_version": ACCESSORY_API_VERSION,
            "server_id": devices.server_id(),
            "service_type": SERVICE_TYPE,
            "dashboard_path": DASHBOARD_PATH,
            "authentication": "bearer",
            "pairing": "ble-sc-numeric-v1",
        })),
        ("GET", DASHBOARD_PATH) => {
            let Some(token) = bearer_token(request) else {
                return Response::with_status(401, json!({"error": "unauthorized"}));
            };
            let Some(device_id) = devices.authenticate(token) else {
                return Response::with_status(401, json!({"error": "unauthorized"}));
            };
            let body = snapshot
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            Response::json(200, body).with_header("X-Gauge-Device", device_id)
        }
        ("DELETE", "/v1/accessory") => {
            let Some(token) = bearer_token(request) else {
                return Response::with_status(401, json!({"error": "unauthorized"}));
            };
            let Some(device_id) = devices.authenticate(token) else {
                return Response::with_status(401, json!({"error": "unauthorized"}));
            };
            match devices.revoke(&device_id) {
                Ok(true) => Response::ok(json!({"ok": true, "device_id": device_id})),
                Ok(false) => Response::with_status(404, json!({"error": "not_found"})),
                Err(error) => Response::with_status(
                    500,
                    json!({"error": "revocation_failed", "detail": error}),
                ),
            }
        }
        _ => Response::with_status(405, json!({"error": "method_not_allowed"})),
    }
}

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    header_block: &'a str,
}

impl<'a> Request<'a> {
    fn parse(input: &'a str) -> Result<Self, ()> {
        let (request_line, header_block) = input.split_once('\n').unwrap_or((input, ""));
        let mut parts = request_line.trim_end_matches('\r').split_whitespace();
        let method = parts.next().ok_or(())?;
        let target = parts.next().ok_or(())?;
        let version = parts.next();

        if parts.next().is_some()
            || !version.is_some_and(|value| matches!(value, "HTTP/1.0" | "HTTP/1.1"))
            || !target.starts_with('/')
        {
            return Err(());
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        // Authentication in query strings leaks into logs and is deliberately
        // not part of the accessory protocol.
        if !query.is_empty() {
            return Err(());
        }
        for line in header_block.lines() {
            if line.trim_end_matches('\r').is_empty() {
                break;
            }
            let line = line.trim_end_matches('\r');
            let (name, value) = line.split_once(':').ok_or(())?;
            if name.is_empty() || name.trim() != name || value.contains(['\r', '\n']) {
                return Err(());
            }
        }

        Ok(Self {
            method,
            path,
            header_block,
        })
    }

    fn header(&self, expected: &str) -> Option<&'a str> {
        self.header_block
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| name.eq_ignore_ascii_case(expected).then(|| value.trim()))
    }
}

struct Response {
    status: u16,
    body: String,
    extra_headers: Vec<(String, String)>,
}

impl Response {
    fn ok(value: Value) -> Self {
        Self::with_status(200, value)
    }

    fn error(error: &str) -> Self {
        Self::with_status(400, json!({"error": error}))
    }

    fn with_status(status: u16, value: Value) -> Self {
        Self::json(status, value.to_string())
    }

    fn json(status: u16, body: String) -> Self {
        Self {
            status,
            body,
            extra_headers: Vec::new(),
        }
    }

    fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }
}

fn bearer_token<'a>(request: &'a Request<'_>) -> Option<&'a str> {
    let value = request.header("authorization")?;
    let mut parts = value.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(scheme), Some(token), None) if scheme.eq_ignore_ascii_case("bearer") => Some(token),
        _ => None,
    }
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
    let extra_headers = response
        .extra_headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let response_head = format!(
        "HTTP/1.1 {} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         {allow}{extra_headers}\r\n",
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
    fn parses_http_requests_and_bearer_authentication() {
        let request =
            Request::parse("GET /v1/dashboard HTTP/1.1\r\nAuthorization: bearer secret\r\n\r\n")
                .unwrap();
        assert_eq!(request.path, DASHBOARD_PATH);
        assert_eq!(bearer_token(&request), Some("secret"));
        let unpair =
            Request::parse("DELETE /v1/accessory HTTP/1.1\r\nAuthorization: bearer secret\r\n\r\n")
                .unwrap();
        assert_eq!(unpair.method, "DELETE");
        assert_eq!(unpair.path, "/v1/accessory");
        assert_eq!(bearer_token(&unpair), Some("secret"));
    }

    #[test]
    fn rejects_query_credentials_and_malformed_requests() {
        assert!(Request::parse("GET /v1/dashboard?token=secret HTTP/1.1\r\n\r\n").is_err());
        assert!(Request::parse("GET /v1/dashboard").is_err());
        assert!(Request::parse("POST").is_err());
        assert!(Request::parse("GET relative HTTP/1.1\r\n\r\n").is_err());
    }

    #[test]
    fn rejects_ambiguous_authorization_headers() {
        let raw =
            Request::parse("GET /v1/dashboard HTTP/1.1\r\nAuthorization: secret\r\n\r\n").unwrap();
        assert_eq!(bearer_token(&raw), None);
    }
}
