//! Process-level regression coverage for read-only diagnostic commands.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use codewhale_secrets::{FileKeyringStore, KeyringStore};
use tempfile::TempDir;

#[test]
fn doctor_text_leaves_a_sealed_home_untouched() {
    let output = run_sealed_diagnostic(["doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("codewhale Doctor"), "stdout:\n{stdout}");
}

#[test]
fn doctor_json_leaves_a_sealed_home_untouched() {
    let output = run_sealed_diagnostic(["doctor", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "doctor --json must remain machine-readable: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    assert_eq!(report["api_connectivity"]["checked"], false);
}

#[test]
fn doctor_context_json_leaves_a_sealed_home_untouched() {
    let output = run_sealed_diagnostic(["doctor", "--context-json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "doctor --context-json must remain machine-readable: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    assert!(
        report["entries"].is_array(),
        "doctor --context-json must emit a source map\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn setup_status_leaves_a_sealed_home_untouched() {
    let output = run_sealed_diagnostic(["setup", "--status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Codewhale Status"), "stdout:\n{stdout}");
}

#[test]
fn diagnostics_read_home_legacy_settings_without_migrating_them() {
    for args in [
        &["doctor"][..],
        &["doctor", "--json"][..],
        &["setup", "--status"][..],
    ] {
        let fixture = TempDir::new().expect("fixture root");
        let workspace = fixture.path().join("workspace");
        let home = fixture.path().join("home");
        let legacy = home.join(".deepseek").join("settings.toml");
        let primary_home = home.join(".codewhale");
        let legacy_bytes = b"default_mode = \"plan\"\nprefer_external_pdftotext = true\n";
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy directory");
        fs::write(&legacy, legacy_bytes).expect("legacy settings");

        let output = diagnostic_command(&workspace, &home)
            .args(args)
            .output()
            .expect("run diagnostic against legacy settings");
        assert!(
            output.status.success(),
            "diagnostic {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        match args {
            ["doctor"] => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    stdout.contains("default_mode=plan (settings)"),
                    "doctor must report the legacy default mode\nstdout:\n{stdout}"
                );
                assert!(
                    stdout.contains("prefer_external_pdftotext = true"),
                    "doctor must report the legacy PDF preference\nstdout:\n{stdout}"
                );
            }
            ["doctor", "--json"] => {
                let report: serde_json::Value =
                    serde_json::from_slice(&output.stdout).expect("machine-readable doctor report");
                assert_eq!(
                    report["setup"]["runtime_posture"]["default_mode"]["value"],
                    "plan"
                );
                assert_eq!(
                    report["setup"]["runtime_posture"]["default_mode"]["source"],
                    "settings"
                );
            }
            ["setup", "--status"] => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    stdout.contains("default_mode: plan (settings)"),
                    "setup status must report the legacy default mode\nstdout:\n{stdout}"
                );
            }
            _ => unreachable!("fixed diagnostic command list"),
        }

        assert_eq!(
            fs::read(&legacy).expect("legacy settings after diagnostic"),
            legacy_bytes,
            "diagnostic {args:?} must not rewrite legacy settings"
        );
        assert!(
            !primary_home.exists(),
            "diagnostic {args:?} must not create a primary Codewhale home"
        );
    }
}

#[test]
fn doctor_json_does_not_inherit_an_ambient_legacy_secret_from_an_explicit_home() {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let home = fixture.path().join("home");
    let codewhale_home = fixture.path().join("isolated-codewhale-home");
    fs::create_dir_all(&workspace).expect("workspace");
    let legacy = home.join(".deepseek").join("secrets").join("secrets.json");
    FileKeyringStore::new(&legacy)
        .set("deepseek", "synthetic-ambient-legacy-value")
        .expect("seed ambient legacy secret");
    let legacy_before = fs::read(&legacy).expect("read legacy secret before doctor");

    let mut command = Command::new(codewhale_tui_binary());
    command
        .current_dir(&workspace)
        .args(["doctor", "--json"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").expect("PATH"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CODEWHALE_HOME", &codewhale_home)
        .env("CODEWHALE_SECRET_BACKEND", "file")
        .env(
            "CODEWHALE_RELEASE_BASE_URL",
            "https://example.invalid/releases",
        )
        .env("DEEPSEEK_TUI_VERSION", env!("CARGO_PKG_VERSION"));
    preserve_host_rustup_home(&mut command);

    let output = command.output().expect("run isolated doctor --json");
    assert!(
        output.status.success(),
        "isolated doctor --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable doctor report");
    assert_eq!(
        report["api_key"]["source"], "missing",
        "doctor must not report an ambient legacy secret from outside an explicit home"
    );
    assert_eq!(
        fs::read(&legacy).expect("read legacy secret after doctor"),
        legacy_before,
        "doctor must not rewrite the ambient legacy secret"
    );
    assert!(
        !codewhale_home.exists(),
        "doctor must not create an isolated Codewhale home or secret store"
    );
}

#[test]
fn doctor_text_probe_uses_a_legacy_key_without_migrating_it() {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let home = fixture.path().join("home");
    fs::create_dir_all(&workspace).expect("workspace");
    let legacy = home.join(".deepseek").join("secrets").join("secrets.json");
    let primary = home.join(".codewhale").join("secrets").join("secrets.json");
    FileKeyringStore::new(&legacy)
        .set("deepseek", "diagnostic-legacy-key")
        .expect("seed legacy secret");
    let legacy_before = fs::read(&legacy).expect("read legacy secret before doctor");
    let (base_url, request) = one_request_completion_server();
    let config = workspace.join("doctor.toml");
    fs::write(
        &config,
        format!(
            "provider = \"deepseek\"\n[providers.deepseek]\nbase_url = \"{base_url}\"\nmodel = \"deepseek-chat\"\nauth_mode = \"api_key\"\n"
        ),
    )
    .expect("write doctor config");

    let output = diagnostic_command(&workspace, &home)
        .args([
            "--config",
            config.to_str().expect("config path"),
            "doctor",
            "--probe-local",
        ])
        .output()
        .expect("run doctor probe");
    assert!(
        output.status.success(),
        "doctor probe failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("API connection successful"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let request = request
        .recv_timeout(Duration::from_secs(5))
        .expect("doctor must make one local probe request")
        .expect("local probe request");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer diagnostic-legacy-key"),
        "doctor probe must use the legacy credential without printing it; request:\n{request}"
    );
    assert!(
        !primary.exists(),
        "doctor's text connectivity probe must not create a migrated primary secret store"
    );
    assert_eq!(
        fs::read(&legacy).expect("read legacy secret after doctor"),
        legacy_before,
        "doctor must not rewrite the legacy secret store"
    );
}

#[test]
fn doctor_json_auth_scheme_reads_a_legacy_key_without_migrating_it() {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let home = fixture.path().join("home");
    fs::create_dir_all(&workspace).expect("workspace");
    let legacy = home.join(".deepseek").join("secrets").join("secrets.json");
    let primary = home.join(".codewhale").join("secrets").join("secrets.json");
    FileKeyringStore::new(&legacy)
        .set("xiaomi-mimo", "tp-diagnostic-legacy-key")
        .expect("seed legacy Xiaomi secret");
    let legacy_before = fs::read(&legacy).expect("read legacy secret before doctor");
    let config = workspace.join("doctor.toml");
    fs::write(
        &config,
        "provider = \"xiaomi-mimo\"\n[providers.xiaomi_mimo]\nmode = \"standard\"\n",
    )
    .expect("write doctor config");

    let output = diagnostic_command(&workspace, &home)
        .args([
            "--config",
            config.to_str().expect("config path"),
            "doctor",
            "--json",
        ])
        .output()
        .expect("run doctor json");
    assert!(
        output.status.success(),
        "doctor --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable doctor report");
    assert_eq!(report["api_key"]["source"], "keyring");
    assert_eq!(report["route"]["auth"]["scheme"], "api-key");
    assert_eq!(report["route"]["auth"]["source"], "keyring");
    assert!(
        !primary.exists(),
        "doctor --json must not migrate a legacy secret while classifying auth"
    );
    assert_eq!(
        fs::read(&legacy).expect("read legacy secret after doctor"),
        legacy_before,
        "doctor --json must not rewrite the legacy secret store"
    );
}

#[test]
fn setup_status_reads_a_legacy_key_without_migrating_it() {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let home = fixture.path().join("home");
    fs::create_dir_all(&workspace).expect("workspace");
    let legacy = home.join(".deepseek").join("secrets").join("secrets.json");
    let primary = home.join(".codewhale").join("secrets").join("secrets.json");
    FileKeyringStore::new(&legacy)
        .set("deepseek", "setup-status-legacy-key")
        .expect("seed legacy secret");
    let legacy_before = fs::read(&legacy).expect("read legacy secret before setup");

    let output = diagnostic_command(&workspace, &home)
        .args(["setup", "--status"])
        .output()
        .expect("run setup status");
    assert!(
        output.status.success(),
        "setup --status failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("api_key: set via OS keyring"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !primary.exists(),
        "setup --status must not create a migrated primary secret store"
    );
    assert_eq!(
        fs::read(&legacy).expect("read legacy secret after setup"),
        legacy_before,
        "setup --status must not rewrite the legacy secret store"
    );
}

#[test]
fn doctor_json_stash_honors_an_explicit_codewhale_home() {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let home = fixture.path().join("home");
    let codewhale_home = fixture.path().join("isolated-codewhale-home");
    fs::create_dir_all(&workspace).expect("workspace");
    let ambient_stash = home.join(".codewhale").join("composer_stash.jsonl");
    fs::create_dir_all(ambient_stash.parent().expect("ambient stash parent"))
        .expect("ambient stash parent");
    fs::write(
        &ambient_stash,
        r#"{"text":"ambient draft must not be inspected"}"#,
    )
    .expect("ambient stash");
    let ambient_before = fs::read(&ambient_stash).expect("read ambient stash before doctor");

    let mut command = diagnostic_command(&workspace, &home);
    command
        .args(["doctor", "--json"])
        .env("CODEWHALE_HOME", &codewhale_home);
    let output = command.output().expect("run isolated doctor json");
    assert!(
        output.status.success(),
        "isolated doctor --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable doctor report");
    assert_eq!(
        report["storage"]["stash"]["path"],
        codewhale_home
            .join("composer_stash.jsonl")
            .display()
            .to_string()
    );
    assert_eq!(report["storage"]["stash"]["present"], false);
    assert_eq!(report["storage"]["stash"]["count"], 0);
    assert!(report["storage"]["stash"]["error"].is_null());
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("ambient draft must not be inspected"),
        "doctor must not inspect an ambient stash outside explicit CODEWHALE_HOME"
    );
    assert_eq!(
        fs::read(&ambient_stash).expect("read ambient stash after doctor"),
        ambient_before,
        "doctor must not rewrite the ambient stash"
    );
    assert!(
        !codewhale_home.exists(),
        "a diagnostic must not create an explicit stash home"
    );
}

fn run_sealed_diagnostic<const N: usize>(args: [&str; N]) -> Output {
    let fixture = TempDir::new().expect("fixture root");
    let workspace = fixture.path().join("workspace");
    let sealed_home = fixture.path().join("sealed-home");
    let codewhale_home = fixture.path().join("sealed-codewhale-home");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let mut command = Command::new(codewhale_tui_binary());
    command
        .current_dir(&workspace)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").expect("PATH"))
        .env("HOME", &sealed_home)
        .env("USERPROFILE", &sealed_home)
        .env("CODEWHALE_HOME", &codewhale_home)
        .env("CODEWHALE_SECRET_BACKEND", "file")
        // Keep the text doctor command offline: the release crate treats this
        // as a pinned mirror version and does not issue a metadata request.
        .env(
            "CODEWHALE_RELEASE_BASE_URL",
            "https://example.invalid/releases",
        )
        .env("DEEPSEEK_TUI_VERSION", env!("CARGO_PKG_VERSION"));
    preserve_host_rustup_home(&mut command);

    let output = command.output().expect("run sealed diagnostic");
    assert!(
        output.status.success(),
        "diagnostic {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !sealed_home.exists(),
        "diagnostic {args:?} must not create a HOME tree at {}",
        sealed_home.display()
    );
    assert!(
        !codewhale_home.exists(),
        "diagnostic {args:?} must not create CODEWHALE_HOME or a secrets store at {}",
        codewhale_home.display()
    );
    output
}

fn diagnostic_command(workspace: &std::path::Path, home: &std::path::Path) -> Command {
    let mut command = Command::new(codewhale_tui_binary());
    command
        .current_dir(workspace)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").expect("PATH"))
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEWHALE_SECRET_BACKEND", "file")
        .env(
            "CODEWHALE_RELEASE_BASE_URL",
            "https://example.invalid/releases",
        )
        .env("DEEPSEEK_TUI_VERSION", env!("CARGO_PKG_VERSION"));
    preserve_host_rustup_home(&mut command);
    command
}

fn one_request_completion_server() -> (String, mpsc::Receiver<Result<String, String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local probe server");
    let address = listener.local_addr().expect("local probe server address");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| error.to_string())?;
            let request = read_http_request(&mut stream)?;
            let body = "{\"id\":\"doctor\",\"object\":\"chat.completion\",\"created\":0,\"model\":\"deepseek-chat\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            Ok(request)
        })();
        let _ = sender.send(result);
    });
    (format!("http://{address}/v1"), receiver)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("connection closed before HTTP headers arrived".to_string());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 32 * 1024 {
            return Err("HTTP headers exceeded test limit".to_string());
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = http_content_length(&headers);
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("connection closed before HTTP body arrived".to_string());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn http_content_length(headers: &str) -> usize {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

#[test]
fn local_probe_server_finds_content_length_after_other_headers() {
    let headers = "POST /v1/chat/completions HTTP/1.1\r\n\
        authorization: Bearer redacted\r\n\
        content-type: application/json\r\n\
        Content-Length: 37\r\n\r\n";

    assert_eq!(http_content_length(headers), 37);
}

/// A rustup shim may initialize its own toolchain state below `$HOME` when
/// `doctor` asks `rustc --version`. Preserve an already-configured toolchain
/// root so this test isolates Codewhale's own state contract.
fn preserve_host_rustup_home(command: &mut Command) {
    let rustup_home = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".rustup"))
                .filter(|path| path.is_dir())
        });
    if let Some(rustup_home) = rustup_home {
        command.env("RUSTUP_HOME", rustup_home);
    }
}

fn codewhale_tui_binary() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_codewhale-tui") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_codewhale-tui") {
        return PathBuf::from(path);
    }

    let mut path = std::env::current_exe().expect("current test executable path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push(format!("codewhale-tui{}", std::env::consts::EXE_SUFFIX));
    path
}
