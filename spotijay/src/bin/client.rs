use async_std::task::block_on;
use rspotify::blocking::{
    client::Spotify,
    oauth2::{SpotifyClientCredentials, SpotifyOAuth},
    util::get_token,
};
use rspotify::model::offset::for_position;
use shared::lib::{now, Input, Output, Room};
use std::sync::{Arc, Mutex};
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
            match spotify.start_playback(
                None,
                None,
                Some(vec![uri]),
                for_position(0),
                Some(offset_ms),
            ) {
                Ok(()) => println!("start playback successful"),
                Err(e) => eprintln!("start playback failed: {:?}", e),
            }
        }
        None => println!("auth failed"),
    };
}

async fn queue_song(uri: String) {
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
            match spotify.add_item_to_queue(uri, None) {
                Ok(()) => println!("add to queue successful"),
                Err(e) => eprintln!("add to queue failed: {:?}", e),
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
            // Comment in to test "auth"
            serde_json::to_string(&Input::Authenticate("test_user_id".into())).unwrap(),
            // serde_json::to_string(&Input::AddTrack(
            //     "DropDuck".into(),
            //     Track {
            //         duration_ms: 17000,
            //         uri: "spotify:track:7qLJdsmsNsMpUpqoTj7g9p".into(),
            //     },
            // ))
            // .unwrap(),
            //serde_json::to_string(&Input::BecomeDj("DropDuck".into())).unwrap(),
        ))
        .unwrap();
    loop {
        let msg = socket.read_message().expect("Error reading message");
        println!("Received: {}", msg);

        let response: Output = serde_json::from_str(msg.to_text().unwrap()).unwrap();

        match response.clone() {
            Output::RoomState(room) => {
                // First RoomState, lets become a DJ.
                if let None = *db.lock().unwrap() {
                    socket
                        .write_message(Message::Text(
                            serde_json::to_string(&Input::BecomeDj("DropDuck".into())).unwrap(),
                        ))
                        .unwrap();
                }
                db.lock().unwrap().replace(room.clone());
            }
            Output::TrackPlayed(playing) => {
                let offset = now() - playing.started;
                block_on(play_song(playing.uri, offset as u32));
            }
            Output::NextTrackQueued(track) => {
                block_on(queue_song(track.uri));
            }
        }
    }
}
