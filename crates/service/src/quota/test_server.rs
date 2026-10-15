//! 测试用的本机 HTTP 服务：每个连接按顺序回一份固定内容，记下请求行和请求头。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub(crate) type Seen = Arc<Mutex<Vec<Vec<String>>>>;

/// 返回 `http://127.0.0.1:端口` 和收到的请求。
pub(crate) fn serve(replies: Vec<(u16, &'static str, String)>) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for (status, extra, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim_end().is_empty() {
                    break;
                }
                lines.push(line.trim_end().to_string());
            }
            log.lock().unwrap().push(lines);
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (origin, seen)
}

pub(crate) fn header(request: &[String], name: &str) -> Option<String> {
    request
        .iter()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        })
        .map(|(_, v)| v.trim().to_string())
}
