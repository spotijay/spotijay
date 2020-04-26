use std::time::Duration;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, Error as IoError},
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use shared::lib::{current_unix_epoch, Input, Output, Playing, Room, Track, User, Zipper};

use futures::{
    channel::mpsc::{unbounded, UnboundedSender},
    future, pin_mut,
    stream::TryStreamExt,
    StreamExt,
};

use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use async_tls::TlsAcceptor;
use tungstenite::protocol::Message;

use rustls::{
    internal::pemfile::{certs, rsa_private_keys},
    Certificate, NoClientAuth, PrivateKey, ServerConfig,
};

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{self, params, OptionalExtension};

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Peer>>>;
type Pool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
type Connection = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

struct Peer {
    pub tx: Tx,
    pub user_id: Option<String>,
}

async fn handle_connection(
    peers_wrap: PeerMap,
    raw_stream: TcpStream,
    acceptor: &Option<TlsAcceptor>,
    addr: SocketAddr,
    pool: Pool,
) {
    println!("Incoming TCP connection from: {}", addr);

    let stream = if let Some(acceptor) = acceptor {
        let tls_stream = acceptor.accept(raw_stream).await.unwrap();

        async_tungstenite::stream::Stream::Tls(tls_stream)
    } else {
        async_tungstenite::stream::Stream::Plain(raw_stream)
    };

    let ws_stream = async_tungstenite::accept_async(stream)
        .await
        .expect("Error during the websocket handshake occurred");

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peers_wrap.lock().unwrap().insert(
        addr,
        Peer {
            tx: tx.clone(),
            user_id: None,
        },
    );

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        handle_message(pool.clone(), peers_wrap.clone(), tx.clone(), msg, addr)
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);
    peers_wrap.lock().unwrap().remove(&addr);
}

fn handle_message(
    pool: Pool,
    peers_wrap: PeerMap,
    tx: UnboundedSender<Message>,
    msg: Message,
    addr: SocketAddr,
) -> futures::future::Ready<Result<(), tungstenite::error::Error>> {
    let input: Result<Input, serde_json::Error> = serde_json::from_str(msg.to_text().unwrap());
    let conn = pool.get().unwrap();

    if let Err(e) = input {
        println!(
            "Couldn't deserialize: {:?}, serde error: {:?}",
            msg.to_text(),
            e
        );
    } else if let Ok(val) = input {
        match val {
            Input::Authenticate(user_id) => {
                peers_wrap.lock().unwrap().get_mut(&addr).unwrap().user_id = Some(user_id.clone());

                tx.unbounded_send(
                    serde_json::to_string_pretty(&Output::Authenticated(user_id))
                        .unwrap()
                        .into(),
                )
                .unwrap();
            }
            Input::JoinRoom(user) => {
                let mut room = get_room("the_room", &conn).unwrap().unwrap();

                if !room.users.iter().any(|x| x.id == user.id) {
                    if let Some(djs) = &room.djs {
                        if !djs.iter().any(|x| x.id == user.id) {
                            room.users.push(user.clone());
                        }
                    } else {
                        room.users.push(user.clone());
                    }
                }

                update_room(room.clone(), &conn).unwrap();

                tx.unbounded_send(
                    serde_json::to_string_pretty(&Output::RoomState(room))
                        .unwrap()
                        .into(),
                )
                .unwrap();
            }
            Input::BecomeDj(user_id) => {
                let mut room = get_room("the_room", &conn).unwrap().unwrap();

                let user = room.users.iter().find(|x| x.id == user_id);

                if let Some(user) = user {
                    let user = user.clone();

                    if user.queue.len() > 0 {
                        room.users.retain(|x| x.id != user_id);

                        if let Some(djs) = &mut room.djs {
                            djs.after.push(user);
                        } else {
                            room.djs = Some(Zipper::singleton(user))
                        }
                    }
                }

                start_playing_if_theres_a_dj(
                    &mut room,
                    &pool,
                    peers_wrap.clone(),
                    current_unix_epoch(),
                );

                update_room(room, &conn).unwrap();

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn).unwrap().unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                        .unwrap();
                }
            }
            Input::UnbecomeDj(_user_id) => {
                unimplemented!();
            }
            Input::AddTrack(user_id, track) => {
                let mut room = get_room("the_room", &conn).unwrap().unwrap();

                if let Some(this_user) = room.users.iter_mut().find(|x| x.id == user_id) {
                    this_user.queue.push(track.clone());
                } else if let Some(djs) = &mut room.djs {
                    let this_dj = djs.iter_mut().find(|x| x.id == user_id);

                    if let Some(dj) = this_dj {
                        dj.queue.push(track.clone());
                    }
                }

                start_playing_if_theres_a_dj(
                    &mut room,
                    &pool,
                    peers_wrap.clone(),
                    current_unix_epoch(),
                );

                update_room(room, &conn).unwrap();

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn).unwrap().unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                        .unwrap();
                }
            }
            Input::RemoveTrack(user_id, track_id) => {
                let mut room = get_room("the_room", &conn).unwrap().unwrap();

                if let Some(this_user) = room.users.iter_mut().find(|x| x.id == user_id) {
                    this_user.queue.retain(|x| x.id != track_id);
                } else if let Some(djs) = &mut room.djs {
                    let this_dj = djs.iter_mut().find(|x| x.id == user_id);

                    if let Some(dj) = this_dj {
                        dj.queue.retain(|x| x.id != track_id);
                    }
                }

                prune_djs_without_queue(&mut room);

                start_playing_if_theres_a_dj(
                    &mut room,
                    &pool,
                    peers_wrap.clone(),
                    current_unix_epoch(),
                );

                update_room(room, &conn).unwrap();

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn).unwrap().unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                        .unwrap();
                }
            }
        }
    }

    future::ok(())
}

fn get_room(room_id: &str, conn: &Connection) -> rusqlite::Result<Option<Room>> {
    let mut stmt =
        conn.prepare("SELECT id, users, playing, djs, next_up FROM room WHERE room.id = ?")?;
    let res = stmt.query_row(params![room_id], |row| {
        let id: String = row.get(0)?;
        let users = serde_json::from_value(row.get(1)?).unwrap();
        let playing = serde_json::from_value(row.get(2)?).unwrap();
        let djs = serde_json::from_value(row.get(3)?).unwrap();
        let next_up = serde_json::from_value(row.get(4)?).unwrap();

        Ok(Room {
            id: id,
            users: users,
            playing: playing,
            djs: djs,
            next_up: next_up,
        })
    });

    res.optional()
}

fn update_room(room: Room, conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE room SET id = ?1, users = ?2, playing = ?3, djs = ?4, next_up = ?5",
        params![
            room.id,
            serde_json::to_value(room.users).unwrap(),
            serde_json::to_value(room.playing).unwrap(),
            serde_json::to_value(room.djs).unwrap(),
            serde_json::to_value(room.next_up).unwrap(),
        ],
    )
    .map(|_| ())
}

fn _insert_room(room: Room, conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO room (id, users, playing, djs, next_up) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            room.id,
            serde_json::to_value(room.users).unwrap(),
            serde_json::to_value(room.playing).unwrap(),
            serde_json::to_value(room.djs).unwrap(),
            serde_json::to_value(room.next_up).unwrap(),
        ],
    )
    .map(|_| ())
}

fn send_to_all(peers: PeerMap, data: &String) {
    let peers = peers.lock().unwrap();
    let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

    for recp in recipients {
        recp.unbounded_send(Message::text(data)).unwrap();
    }
}

fn start_playing_if_theres_a_dj(room: &mut Room, pool: &Pool, peers: PeerMap, now: u64) {
    let play_new_track = {
        if let None = room.playing {
            if let Some(djs) = &mut room.djs {
                if djs.current.queue.len() > 0 {
                    Some(djs.current.queue.remove(0))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(track) = play_new_track {
        play(room, pool.clone(), peers.clone(), track.clone(), now);

        let playing = Playing::new(track.clone(), now);

        send_to_all(
            peers,
            &serde_json::to_string_pretty(&Output::TrackPlayed(playing)).unwrap(),
        );
    }
}

fn prune_djs_without_queue(room: &mut Room) {
    if let Some(djs) = &mut room.djs {
        let mut djs_without_queue = djs
            .before
            .clone()
            .into_iter()
            .chain(djs.after.clone().into_iter())
            .filter(|x| x.queue.len() == 0)
            .collect::<Vec<User>>();

        room.users.append(&mut djs_without_queue);

        djs.before.retain(|x| x.queue.len() > 0);
        djs.after.retain(|x| x.queue.len() > 0);

        if djs.current.queue.len() == 0 && room.playing.is_none() {
            room.users.push(djs.current.clone());

            if djs.before.len() == 0 && djs.after.len() == 0 {
                room.djs = None;
            } else {
                next_djs(djs);

                djs.before.pop();
                room.djs = Some(djs.to_owned());
            }
        }
    };
}

fn next_djs(djs: &mut Zipper<User>) {
    if djs.before.len() == 0 && djs.after.len() == 0 {
        ()
    } else if djs.after.len() == 0 {
        let next = djs.before.remove(0);
        djs.after = djs.before.drain(..).collect();
        djs.after.push(djs.current.clone());
        djs.current = next;
    } else {
        let next = djs.after.remove(0);
        djs.before.push(djs.current.clone());
        djs.current = next;
    }
}

fn play(room: &mut Room, pool: Pool, peers: PeerMap, track: Track, now: u64) {
    let playing = Playing {
        id: track.id.clone(),
        name: track.name.clone(),
        uri: track.uri.clone(),
        duration_ms: track.duration_ms.clone(),
        started: now,
    };

    room.playing = Some(playing);
    println!("Now playing: {:?} in room {:?}", track, room);

    let later_peers = peers.clone();
    let later_peers2 = peers.clone();
    let later_pool = pool.clone();
    let later_pool2 = pool.clone();
    task::spawn(async move {
        let playing_duration = Duration::from_millis(track.duration_ms as u64);
        let next_up_duration = Duration::from_millis(5000);

        async_std::task::sleep(playing_duration - next_up_duration).await;

        set_next_up_from_queue(later_pool, later_peers);

        task::spawn(async move {
            async_std::task::sleep(Duration::from_millis(5000)).await;
            play_from_next_up(later_pool2, later_peers2)
        });
    });
}

fn play_from_next_up(later_pool2: Pool, later_peers2: PeerMap) {
    let conn = later_pool2.get().unwrap();
    let mut room = get_room("the_room", &conn).unwrap().unwrap();

    let next_track_wrap = { room.next_up.take() };

    if let Some(next_track) = next_track_wrap {
        play(
            &mut room,
            later_pool2,
            later_peers2.clone(),
            next_track,
            current_unix_epoch(),
        );
    } else {
        room.playing = None;
    }

    prune_djs_without_queue(&mut room);

    update_room(room.clone(), &conn).unwrap();

    send_to_all(
        later_peers2,
        &serde_json::to_string_pretty(&Output::RoomState(room)).unwrap(),
    );
}

fn set_next_up_from_queue(later_pool: Pool, later_peers: PeerMap) {
    let conn = later_pool.get().unwrap();
    let mut room = get_room("the_room", &conn).unwrap().unwrap();

    if let Some(djs) = &mut room.djs {
        next_djs(djs);

        if djs.current.queue.len() > 0 {
            room.next_up = Some(djs.current.queue.remove(0));
        } else {
            room.next_up = None;
        }
    }
    println!("Next up queued: {:?}", room.next_up);

    if let Some(next) = room.next_up.clone() {
        send_to_all(
            later_peers,
            &serde_json::to_string_pretty(&Output::NextTrackQueued(next)).unwrap(),
        );
    }

    update_room(room, &conn).unwrap();
}

fn load_certs(path: &Path) -> io::Result<Vec<Certificate>> {
    certs(&mut BufReader::new(File::open(path)?))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid cert"))
}

fn load_keys(path: &Path) -> io::Result<Vec<PrivateKey>> {
    rsa_private_keys(&mut BufReader::new(File::open(path)?))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid key"))
}

fn setup_db(mut conn: rusqlite::Connection) {
    embedded::migrations::runner().run(&mut conn).unwrap();
}

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

async fn run() -> Result<(), IoError> {
    env_logger::init();

    let manager = SqliteConnectionManager::file("./db.sqlite");
    //let manager = SqliteConnectionManager::memory();
    let pool = r2d2::Pool::new(manager).unwrap();

    let migrate_conn = rusqlite::Connection::open("./db.sqlite").unwrap();
    setup_db(migrate_conn);

    let peers = PeerMap::new(Mutex::new(HashMap::new()));

    let mut config = ServerConfig::new(NoClientAuth::new());

    let ssl_cert = dotenv::var("SSL_CERT");
    let ssl_key = dotenv::var("SSL_KEY");

    let acceptor = if ssl_cert.is_ok() && ssl_key.is_ok() {
        let certs = load_certs(Path::new(&ssl_cert.unwrap())).unwrap();
        let mut keys = load_keys(Path::new(&ssl_key.unwrap())).unwrap();
        config.set_single_cert(certs, keys.remove(0)).unwrap();

        Some(TlsAcceptor::from(Arc::new(config)))
    } else {
        None
    };

    let url = dotenv::var("SERVER_URL").unwrap();
    let try_socket = TcpListener::bind(&url).await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: {}", &url);

    let init_conn = &pool.get().unwrap();
    let mut room = get_room("the_room", init_conn).unwrap().unwrap();
    start_playing_if_theres_a_dj(&mut room, &pool, peers.clone(), current_unix_epoch());
    update_room(room, init_conn).unwrap();

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        let peers = peers.clone();
        let acceptor = acceptor.clone();
        let pool = pool.clone();

        task::spawn(async move { handle_connection(peers, stream, &acceptor, addr, pool).await });
    }

    Ok(())
}

fn main() -> Result<(), IoError> {
    task::block_on(run())
}

#[cfg(test)]
mod test {
    use crate::*;

    fn setup_test_db(
        db_name: &str,
    ) -> (
        Pool,
        r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
    ) {
        let path = &format!("file:{}?mode=memory&cache=shared", db_name);
        let manager = SqliteConnectionManager::file(path);
        let pool = r2d2::Pool::new(manager).unwrap();
        let conn = pool.get().unwrap();

        let migrate_conn = rusqlite::Connection::open(path).unwrap();

        setup_db(migrate_conn);

        (pool, conn)
    }

    #[test]
    fn test_insert_track() {
        let (_, conn) = &setup_test_db("test_insert_track");

        let test_track = Track {
            id: "sa67saf64s".into(),
            name: "Cool jazz stuff".into(),
            uri: "spotify:track:sa67saf64s".into(),
            duration_ms: 16345340,
        };

        let room = Room {
            id: "test_room".into(),
            users: Vec::new(),
            djs: None,
            playing: None,
            next_up: Some(test_track),
        };

        let res = update_room(room, conn);

        assert_eq!(res, Ok(()));
    }

    #[test]
    fn test_get_room() {
        let (_, conn) = &setup_test_db("test_get_room");

        let test_track = Track {
            id: "sa67saf64s".into(),
            name: "Cool jazz stuff".into(),
            uri: "spotify:track:sa67saf64s".into(),
            duration_ms: 16345340,
        };

        let room = Room {
            id: "test_room".into(),
            users: Vec::new(),
            djs: None,
            playing: None,
            next_up: Some(test_track.clone()),
        };

        update_room(room.clone(), conn).unwrap();

        let res = get_room("test_room", conn);

        assert_eq!(res, Ok(Some(room)));
    }

    #[test]
    fn test_start_playing_if_theres_a_dj() {
        let (pool, _) = &setup_test_db("test_start_playing_if_theres_a_dj");
        let peers = PeerMap::new(Mutex::new(HashMap::new()));

        let track = Track {
            id: "sa67saf64s".into(),
            name: "Cool jazz stuff".into(),
            uri: "spotify:track:sa67saf64s".into(),
            duration_ms: 16345340,
        };

        let dj = User {
            id: "test_dj".into(),
            queue: vec![track.clone()],
        };

        let mut room = Room {
            id: "test_room".into(),
            users: Vec::new(),
            djs: Some(Zipper::singleton(dj)),
            playing: None,
            next_up: None,
        };

        start_playing_if_theres_a_dj(&mut room, pool, peers, 5678);

        assert_eq!(room.playing, Some(Playing::new(track, 5678)));
    }
}
