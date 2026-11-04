//! Regression fixtures are local-only: no Discord IPC or external HTTP requests.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

fn fixture_root() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("dqc-safety-{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn directory_link(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"]).arg(link).arg(target)
            .creation_flags(0x08000000).status().unwrap();
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn rejects_junction_at_games_root_or_numeric_app_directory() {
    let base = fixture_root();
    let outside = base.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let linked_root = base.join("linked-root");
    directory_link(&outside, &linked_root);
    assert!(paths::resolve_confined_path(&linked_root, 123, "", "fixture.bin").is_err());
    let root = base.join("games");
    std::fs::create_dir(&root).unwrap();
    directory_link(&outside, &root.join("123"));
    assert!(paths::resolve_confined_path(&root, 123, "", "fixture.bin").is_err());
}

#[test]
fn refuses_launch_content_different_from_packaged_runner() {
    let root = fixture_root();
    let template = root.join("template.bin");
    let output = root.join("output.bin");
    std::fs::write(&template, b"trusted fixture").unwrap();
    std::fs::write(&output, b"unrelated fixture").unwrap();
    assert!(paths::verify_runner(&output, &template).is_err());
    std::fs::write(&output, b"trusted fixture").unwrap();
    assert!(paths::verify_runner(&output, &template).is_ok());
}

fn http_fixture(status: &str, body: &str) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut request = [0u8; 4096];
        socket.read(&mut request).unwrap();
        socket.write_all(response.as_bytes()).unwrap();
    });
    (url, worker)
}

#[test]
fn rejects_http_error_status_and_invalid_json() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    for (status, body) in [("503 Unavailable", "[]"), ("200 OK", "not-json")] {
        let (url, worker) = http_fixture(status, body);
        assert!(runtime.block_on(fetch_json_with_timeout(&url)).is_err());
        worker.join().unwrap();
    }
}

#[test]
fn accepts_valid_json_from_local_http_fixture() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (url, worker) = http_fixture("200 OK", "[{\"name\":\"fixture\"}]");
    let result = runtime.block_on(fetch_json_with_timeout(&url)).unwrap();
    assert_eq!(result[0]["name"], "fixture");
    worker.join().unwrap();
}
