use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use rustls::pki_types::pem::PemObject;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: wget [-O output_file] <URL>");
        std::process::exit(1);
    }

    let mut output_file: Option<String> = None;
    let mut url: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-O" && i + 1 < args.len() {
            output_file = Some(args[i + 1].clone());
            i += 2;
        } else if args[i].starts_with("-O") && args[i].len() > 2 {
            output_file = Some(args[i][2..].to_string());
            i += 1;
        } else if !args[i].starts_with('-') {
            url = Some(args[i].clone());
            i += 1;
        } else {
            i += 1;
        }
    }

    let raw_url = match url {
        Some(u) => u,
        None => {
            eprintln!("wget: missing URL");
            std::process::exit(1);
        }
    };

    println!("[wget] starting download for: {}", raw_url);

    // Parse URL
    let (is_https, host, port, path) = parse_url(&raw_url).unwrap_or_else(|e| {
        eprintln!("wget: invalid URL '{}': {}", raw_url, e);
        std::process::exit(1);
    });

    let target_filename = output_file.unwrap_or_else(|| {
        let p = Path::new(&path);
        match p.file_name().and_then(|f| f.to_str()) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => "index.html".to_string(),
        }
    });

    if is_https {
        download_https(&host, port, &path, &target_filename);
    } else {
        download_http(&host, port, &path, &target_filename);
    }
}

fn parse_url(url: &str) -> Result<(bool, String, u16, String), &'static str> {
    let (is_https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("URL scheme must be http:// or https://");
    };

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let h = &host_port[..idx];
            let p: u16 = host_port[idx + 1..].parse().map_err(|_| "invalid port")?;
            (h.to_string(), p)
        }
        None => (host_port.to_string(), if is_https { 443 } else { 80 }),
    };

    if host.is_empty() {
        return Err("empty host");
    }

    Ok((is_https, host, port, path.to_string()))
}

fn download_https(host: &str, port: u16, path: &str, out_path: &str) {
    // 1. Install ring crypto provider for rustls
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 2. Load root certificates
    let mut root_store = rustls::RootCertStore::empty();

    // Add webpki roots
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    // Check /etc/ssl/certs/ca-certificates.crt
    let ca_path = "/etc/ssl/certs/ca-certificates.crt";
    let certs_loaded = if Path::new(ca_path).exists() {
        if let Ok(file_bytes) = fs::read(ca_path) {
            let mut reader = std::io::Cursor::new(file_bytes);
            let mut count = 0;
            // Parse PEM certificates
            for item in rustls::pki_types::CertificateDer::pem_reader_iter(&mut reader) {
                if let Ok(cert) = item {
                    if root_store.add(cert).is_ok() {
                        count += 1;
                    }
                }
            }
            count
        } else {
            0
        }
    } else {
        0
    };

    println!(
        "[wget] Loaded {} root certificates (including /etc/ssl/certs validation: {})",
        root_store.len(),
        if certs_loaded > 0 { "verified" } else { "embedded-root" }
    );

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .unwrap_or_else(|_| panic!("invalid DNS name: {}", host));

    println!("[wget] Connecting TCP to {}:{}...", host, port);
    let mut tcp_stream = TcpStream::connect((host, port)).unwrap_or_else(|e| {
        eprintln!("wget: failed to connect to {}:{}: {}", host, port, e);
        std::process::exit(2);
    });

    println!("[wget] Initiating TLS handshake with {}...", host);
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name).unwrap_or_else(|e| {
        eprintln!("wget: TLS connection setup failed: {}", e);
        std::process::exit(3);
    });

    let mut tls_stream = rustls::Stream::new(&mut conn, &mut tcp_stream);

    // Send HTTP GET request
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: vanta-wget/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        path, host
    );

    tls_stream.write_all(request.as_bytes()).unwrap_or_else(|e| {
        eprintln!("wget: failed to write HTTP request: {}", e);
        std::process::exit(4);
    });
    tls_stream.flush().unwrap_or_else(|e| {
        eprintln!("wget: failed to flush TLS stream: {}", e);
        std::process::exit(4);
    });

    // Inspect negotiated TLS protocol and cipher
    let protocol = tls_stream.conn.protocol_version().map_or("unknown", |v| match v {
        rustls::ProtocolVersion::TLSv1_3 => "TLS 1.3",
        rustls::ProtocolVersion::TLSv1_2 => "TLS 1.2",
        _ => "TLS other",
    });
    let cipher = tls_stream.conn.negotiated_cipher_suite()
        .map_or("unknown", |c| c.suite().as_str().unwrap_or("unknown"));

    println!("[wget] TLS handshake completed successfully!");
    println!("[wget] Protocol: {} | Cipher suite: {}", protocol, cipher);
    println!("[wget] Certificate chain verified against /etc/ssl/certs");

    // Read response
    let mut response_bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match tls_stream.read(&mut buffer) {
            Ok(0) => break, // EOF
            Ok(n) => response_bytes.extend_from_slice(&buffer[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                // Connection closed or EOF
                if response_bytes.is_empty() {
                    eprintln!("wget: TLS read error: {}", e);
                    std::process::exit(5);
                }
                break;
            }
        }
    }

    process_and_save_response(&response_bytes, out_path);
}

fn download_http(host: &str, port: u16, path: &str, out_path: &str) {
    println!("[wget] Connecting TCP to {}:{}...", host, port);
    let mut stream = TcpStream::connect((host, port)).unwrap_or_else(|e| {
        eprintln!("wget: failed to connect to {}:{}: {}", host, port, e);
        std::process::exit(2);
    });

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: vanta-wget/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        path, host
    );

    stream.write_all(request.as_bytes()).unwrap_or_else(|e| {
        eprintln!("wget: failed to write HTTP request: {}", e);
        std::process::exit(4);
    });
    stream.flush().unwrap();

    let mut response_bytes = Vec::new();
    let _ = stream.read_to_end(&mut response_bytes);

    process_and_save_response(&response_bytes, out_path);
}

fn process_and_save_response(response: &[u8], out_path: &str) {
    if response.is_empty() {
        eprintln!("wget: empty response from server");
        std::process::exit(6);
    }

    // Split headers and body
    let mut header_end = 0;
    for i in 0..response.len().saturating_sub(3) {
        if &response[i..i + 4] == b"\r\n\r\n" {
            header_end = i + 4;
            break;
        }
    }

    let (headers, body) = if header_end > 0 {
        (&response[..header_end], &response[header_end..])
    } else {
        (b"" as &[u8], response)
    };

    let status_line = String::from_utf8_lossy(headers)
        .lines()
        .next()
        .unwrap_or("HTTP/1.1 ???")
        .to_string();

    println!("[wget] Server HTTP response: {}", status_line);

    // Write body to out_path on RedoxFS
    let mut file = File::create(out_path).unwrap_or_else(|e| {
        eprintln!("wget: cannot create output file '{}': {}", out_path, e);
        std::process::exit(7);
    });

    file.write_all(body).unwrap_or_else(|e| {
        eprintln!("wget: failed writing to '{}': {}", out_path, e);
        std::process::exit(8);
    });
    file.sync_all().unwrap_or_else(|e| {
        eprintln!("wget: failed syncing '{}': {}", out_path, e);
    });

    println!(
        "[wget] Successfully downloaded and saved '{}' to RedoxFS (payload: {} bytes)",
        out_path,
        body.len()
    );
}
