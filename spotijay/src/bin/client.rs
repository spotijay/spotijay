use async_std::task::block_on;
use rspotify::blocking::{
    client::Spotify,
    oauth2::{SpotifyClientCredentials, SpotifyOAuth},
    util::get_token,
};
use rspotify::model::offset::for_position;
use spotijay::types::{Input, Output, Room, Track, User};
use std::{
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tungstenite::{connect, Message};
use url::Url;

pub struct ClientApp {}

type Db = Arc<Mutex<Option<Room>>>;

async fn play_song(uris: Vec<String>, track_offset: u32, offset_ms: u32) {
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
            let uris = uris;
            match spotify.start_playback(
                None,
                None,
                Some(uris.clone()),
                for_position(track_offset),
                Some(offset_ms),
            ) {
                Ok(()) => {
                    dbg!(uris);
                    println!(
                        "start playback successful, offset: {:?}, track_offset: {:?}",
                        offset_ms, track_offset
                    )
                }
                Err(e) => eprintln!("start playback failed: {:?}", e),
            }
        }
        None => println!("auth failed"),
    };
}

/// Play correct song from queue with correct offset based on current time and aggregated individual song times. All the other songs get queued.
fn play_queue(room: Room) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let room_start_offset = now - room.started;

    let mut queue_index = 0;
    let mut next_song_offset = 0;
    let mut current_song_offset = 0;

    while next_song_offset < room_start_offset && queue_index < room.queue.len() {
        current_song_offset = next_song_offset;
        next_song_offset += room.queue.get(queue_index).unwrap().duration_ms as u64;
        queue_index += 1;
    }

    // Only play if we're not past the last song.
    if room_start_offset < next_song_offset {
        let song_offset = room_start_offset - current_song_offset;
        let queue_offset: u32 = (queue_index as u32).saturating_sub(1);

        let uris = room.queue.into_iter().map(|x| x.uri.clone()).collect();

        block_on(play_song(uris, queue_offset, song_offset as u32));
    }
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
            // serde_json::to_string(&Input::Authenticate(User {
            //     id: "test_user_id".into(),
            // }))
            // .unwrap(),
            serde_json::to_string(&Input::AddTrack(Track {
                duration_ms: 234693,
                uri: "spotify:track:5cXg9IQS34FzLVdHhp7hu7".into(),
            }))
            .unwrap(),
        ))
        .unwrap();
    loop {
        let msg = socket.read_message().expect("Error reading message");
        println!("Received: {}", msg);

        let response: Output = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        match response {
            Output::RoomState(room) => {
                db.lock().unwrap().replace(room.clone());

                if room.queue.len() != 0 {
                    play_queue(room);
                }
            }
            _ => (),
        }
    }
}
