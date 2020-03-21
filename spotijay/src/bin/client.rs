use async_std::task::block_on;
use rspotify::blocking::{
    client::Spotify,
    oauth2::{SpotifyClientCredentials, SpotifyOAuth},
    util::get_token,
};
use rspotify::{
    model::offset::for_position,
};
use spotijay::types::{Output, Room};
use std::{
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tungstenite::{connect, Message};
use url::Url;

pub struct ClientApp {}

type Db = Arc<Mutex<Option<Room>>>;

async fn play_song(uri: String, offset_ms: u32) {
    let mut oauth = SpotifyOAuth::default()
        .scope("user-modify-playback-state")
        .build();

    match get_token(&mut oauth) {
        Some(token_info) => {
            let client_credential = SpotifyClientCredentials::default()
                .token_info(token_info)
                .build();
            let spotify = Spotify::default()
                .client_credentials_manager(client_credential)
                .build();
            let uris = vec![uri];
            match spotify.start_playback(None, None, Some(uris), for_position(0), Some(offset_ms)) {
                Ok(()) => println!("start playback successful, offset: {:?}", offset_ms),
                Err(e) => eprintln!("start playback failed: {:?}", e),
            }
        }
        None => println!("auth failed"),
    };
}

fn main() {
    env_logger::init();

    let db: Db = Arc::new(Mutex::new(None));

    let _app = ClientApp {};

    let (mut socket, response) =
        connect(Url::parse("ws://localhost:3012/").unwrap()).expect("Can't connect");

    println!("Connected to the server");
    println!("Response HTTP code: {}", response.status());
    println!("Response contains the following headers:");
    for (ref header, _value) in response.headers() {
        println!("* {}", header);
    }

    socket
        .write_message(Message::Text(
            "{\"Authenticate\": {\"id\": \"test_user_id\"}}".into(),
        ))
        .unwrap();
    loop {
        let msg = socket.read_message().expect("Error reading message");
        println!("Received: {}", msg);

        let response: Output = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        match response {
            Output::RoomState(room) => {
                db.lock().unwrap().replace(room.clone());

                // Start playing first song from queue.
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;

                let offset = now - room.started;

                block_on(play_song(
                    room.queue.first().unwrap().uri.clone(),
                    offset as u32,
                ));
            }
            _ => (),
        }
    }
}
