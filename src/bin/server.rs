use futures::{FutureExt, StreamExt};
use rusqlite::Connection;
use rustorm::EntityManager;
use std::collections::HashMap;
use warp::ws::Message;
use warp::Filter;

use serde::Deserialize;

use rspotify::client::Spotify;
use rspotify::oauth2::{SpotifyClientCredentials, SpotifyOAuth};

use rustorm::Pool;
use rustorm::{DbError, FromDao, ToColumnNames, ToDao};
use std::sync::Arc;
use tokio::sync::Mutex;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("./migrations");
}

fn db_conn() -> EntityManager {
    Pool::new().em("sqlite://db.sqlite").unwrap()
}

/// Our state of currently connected users.
///
/// - Key is their id
/// - Value is a sender of `warp::ws::Message`
type UserConns =
    Arc<Mutex<HashMap<usize, tokio::sync::mpsc::UnboundedSender<Result<Message, warp::Error>>>>>;

#[derive(Debug, PartialEq, Eq, FromDao, ToDao, ToColumnNames)]
struct Dj {
    dj_id: i32,
    room_id: i32,
    spotify_user_id: String,
    spotify_display_name: String,
}

#[derive(Debug, ToDao, ToColumnNames)]
struct NewDj {
    room_id: String,
    spotify_user_id: String,
    spotify_display_name: String,
}

impl rustorm::dao::ToTableName for NewDj {
    fn to_table_name() -> rustorm::TableName {
        rustorm::TableName {
            name: "djs".into(),
            schema: None,
            alias: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, FromDao, ToDao, ToColumnNames)]
struct Room {
    room_id: String,
    room_display_name: String,
}

impl rustorm::dao::ToTableName for Room {
    fn to_table_name() -> rustorm::TableName {
        rustorm::TableName {
            name: "rooms".into(),
            schema: None,
            alias: None,
        }
    }
}

fn migrate() {
    let mut conn = Connection::open("db.sqlite").unwrap();
    embedded::migrations::runner().run(&mut conn).unwrap();
}

fn load_fixtures() {
    let the_room = Room {
        room_id: "the_room".into(),
        room_display_name: "The Room".into(),
    };
    let _: Result<Vec<Room>, DbError> = db_conn().insert(&[&the_room]);
}

async fn user_message(user_id: i32, msg: Message, user_conns: &UserConns) {
    // Skip any non-Text messages...
    let msg = if let Ok(s) = msg.to_str() {
        s
    } else {
        return;
    };

    let new_msg = format!("<User#{}>: {}", user_id, msg);

    // New message from this user, send it to everyone else (except same uid)...
    //
    // We use `retain` instead of a for loop so that we can reap any user that
    // appears to have disconnected.
    for (&uid, tx) in user_conns.lock().await.iter_mut() {
        if user_id != (uid as i32) {
            if let Err(_disconnected) = tx.send(Ok(Message::text(new_msg.clone()))) {
                // The tx is disconnected, our `user_disconnected` code
                // should be happening in another task, nothing more to
                // do here.
            }
        }
    }
}

fn user_disconnected(user_id: i32, user_conns: &UserConns) {
    eprintln!("good bye user: {}", user_id);

    let remove_user_sql = "DELETE FROM djs WHERE dj_id = ?";

    db_conn()
        .db()
        .execute_sql_with_return(remove_user_sql, &[&user_id.into()])
        .unwrap();
}

async fn user_connected(
    room_id: String,
    websocket: warp::filters::ws::WebSocket,
    user_conns: UserConns,
) {
    println!("{:?}", room_id);
    let dj = NewDj {
        room_id: room_id.clone(),
        spotify_user_id: "agwgwg".into(),
        spotify_display_name: "Test tester".into(),
    };
    let user_id = db_conn()
        .insert::<NewDj, Dj>(&[&dj])
        .unwrap()
        .first()
        .unwrap()
        .dj_id;

    eprintln!("new user in room: {}", &room_id);

    // Split the socket into a sender and receive of messages.
    let (user_ws_tx, mut user_ws_rx) = websocket.split();

    // Use an unbounded channel to handle buffering and flushing of messages
    // to the websocket...
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("websocket send error: {}", e);
        }
    }));

    // Every time the user sends a message, broadcast it to
    // all other users...
    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", user_id, e);
                break;
            }
        };
        user_message(user_id, msg, &user_conns).await;
    }

    // user_ws_rx stream will keep processing as long as the user stays
    // connected. Once they disconnect, then...

    // Blocking in async is bad, I know this.
    user_disconnected(user_id, &user_conns);
}

fn spot_auth_url() -> String {
    let oauth = SpotifyOAuth::default().scope("user-top-read").build();

    oauth.get_authorize_url(None, None)
}

async fn spotify_callback(code: String) -> Result<String, warp::Rejection> {
    let oauth = SpotifyOAuth::default();
    match oauth.get_access_token(&code).await {
        Some(token) => {
            let client_credential = SpotifyClientCredentials::default()
                .token_info(token)
                .build();
            let artists = Spotify::default()
                .client_credentials_manager(client_credential)
                .search_artist("fleet", 3, None, None)
                .await;

            println!("artists: {:?}", artists);

            Ok("wut".into())
        }
        None => Err(warp::reject::not_found()),
    }
}

#[derive(Deserialize)]
struct SpotifyCallbackParams {
    code: String,
    state: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    // Migrate and load fixtures into SQLite
    migrate();
    load_fixtures();

    // Hello world
    let hello = warp::path!("rooms").map(|| format!("Hello world!"));

    // Spotify requests
    let auth = warp::path("auth")
        .map(|| warp::redirect(warp::http::Uri::from_maybe_shared(spot_auth_url()).unwrap()));
    let callback = warp::path("callback")
        .and(warp::query::<SpotifyCallbackParams>())
        .and_then(|params: SpotifyCallbackParams| spotify_callback(params.code));

    // Websockets
    // Keep track of all connected users, key is usize, value is a websocket sender.
    let users = Arc::new(Mutex::new(HashMap::new()));
    // Turn our "state" into a new Filter...
    let user_conns = warp::any().map(move || users.clone());
    let ws = warp::path!("rooms" / String)
        .and(warp::ws())
        .and(user_conns)
        .map(|room_id: String, ws: warp::filters::ws::Ws, conns| {
            ws.on_upgrade(move |socket| user_connected(room_id, socket, conns))
        });

    // Start server
    warp::serve(hello.or(ws).or(auth).or(callback))
        .run(([127, 0, 0, 1], 8888))
        .await;
}
