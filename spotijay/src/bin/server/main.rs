mod db;

use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fs::File,
    io::{self, BufReader},
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use shared::lib::{
    current_unix_epoch, next_djs, prune_djs_without_queue, Input, Output, Playing, Room, Track, Zipper,
};

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
use db::{update_room, get_room, remove_user_from_room};

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Peer>>>;
type Pool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;
type Connection = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

#[derive(Debug)]
struct Peer {
    tx: Tx,
    user_id: Option<String>,
    last_heartbeat: u64,
}

const CONNECTION_TIMEOUT: u64 = 5_000;
const ROOM_TIMEOUT: u64 = 60_000;

async fn handle_connection(
    peers_wrap: PeerMap,
    raw_stream: TcpStream,
    acceptor: &Option<TlsAcceptor>,
    addr: SocketAddr,
    pool: Pool,
) -> Result<(), Box<dyn Error>> {
    println!("Incoming TCP connection from: {}", addr);

    let stream = if let Some(acceptor) = acceptor {
        let tls_stream = acceptor.accept(raw_stream).await?;

        async_tungstenite::stream::Stream::Tls(tls_stream)
    } else {
        async_tungstenite::stream::Stream::Plain(raw_stream)
    };

    let ws_stream = async_tungstenite::accept_async(stream).await?;

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peers_wrap.lock().unwrap().insert(
        addr,
        Peer {
            tx: tx.clone(),
            user_id: None,
            last_heartbeat: current_unix_epoch()?,
        },
    );

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        // Heartbeat check with a custom protocol as browsers don't support the standard WebSocket ping protocol.
        if msg == Message::text("ping") {
            tx.unbounded_send(Message::text("pong")).unwrap();
            peers_wrap
                .lock()
                .unwrap()
                .get_mut(&addr)
                .unwrap()
                .last_heartbeat = current_unix_epoch().unwrap();
            future::ok(())
        } else {
            handle_message(pool.clone(), peers_wrap.clone(), tx.clone(), msg, addr).unwrap();

            future::ok(())
        }
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);

    let conn = &pool.get()?;
    let user_id = peers_wrap
        .lock()
        .unwrap()
        .get(&addr)
        .and_then(|x| x.user_id.clone());

    peers_wrap.lock().unwrap().remove(&addr);

    // Add a last_disconnect for user to time them out for inactivity if their last connection has been disconnected
    if let Some(user_id) = user_id {
        if !peers_wrap
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| v.user_id == Some(user_id.clone()))
        {
            let mut room = get_room("the_room", conn)?.unwrap();
            let now = current_unix_epoch()?;

            for user in room
                .users
                .iter_mut()
                .chain(room.djs.iter_mut().flat_map(|x| x.iter_mut()))
            {
                if user_id == user.id {
                    user.last_disconnect = Some(now);
                }
            }

            update_room(room, conn)?;
        }
    }

    Ok(())
}

fn handle_message(
    pool: Pool,
    peers_wrap: PeerMap,
    tx: UnboundedSender<Message>,
    msg: Message,
    addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let input: Result<Input, serde_json::Error> = serde_json::from_str(msg.to_text()?);
    let conn = pool.get()?;

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
                    serde_json::to_string_pretty(&Output::Authenticated(user_id))?.into(),
                )?;
            }
            Input::JoinRoom(user) => {
                let mut room = get_room("the_room", &conn)?.unwrap();

                // Reset room timeout for user.
                room.users
                    .iter_mut()
                    .chain(room.djs.iter_mut().flat_map(|x| x.iter_mut()))
                    .for_each(|x| {
                        if x.id == user.id {
                            x.last_disconnect = None;
                        }
                    });

                if !room.users.iter().any(|x| x.id == user.id) {
                    if let Some(djs) = &room.djs {
                        if !djs.iter().any(|x| x.id == user.id) {
                            room.users.push(user.clone());
                        }
                    } else {
                        room.users.push(user.clone());
                    }
                }

                update_room(room.clone(), &conn)?;

                tx.unbounded_send(serde_json::to_string_pretty(&Output::RoomState(room))?.into())?;
            }
            Input::BecomeDj(user_id) => {
                let mut room = get_room("the_room", &conn)?.unwrap();

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
                    current_unix_epoch()?,
                );

                update_room(room, &conn)?;

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn)?.unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output)?.into())?;
                }
            }
            Input::Downvote(user_id) => {
                let now = current_unix_epoch()?;
                let peers = peers_wrap.clone();
                let mut room = get_room("the_room", &conn)?.unwrap();

                let song_changed = downvote(user_id.clone(), &mut room, now, pool, peers);
                update_room(room.clone(), &conn)?;

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                let output = if song_changed {
                    Output::TrackPlayed(room.playing)
                } else {
                    Output::Downvoted(user_id)
                };

                for recp in recipients {
                    recp.unbounded_send(serde_json::to_string_pretty(&output)?.into())?;
                }
            }
            Input::AddTrack(user_id, track) => {
                let mut room = get_room("the_room", &conn)?.unwrap();

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
                    current_unix_epoch()?,
                );

                update_room(room, &conn)?;

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn)?.unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output)?.into())?;
                }
            }
            Input::RemoveTrack(user_id, track_id) => {
                let mut room = get_room("the_room", &conn)?.unwrap();

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
                    current_unix_epoch()?,
                );

                update_room(room, &conn)?;

                let peers = peers_wrap.lock().unwrap();
                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(get_room("the_room", &conn)?.unwrap());
                    recp.unbounded_send(serde_json::to_string_pretty(&output)?.into())?;
                }
            }
        }
    }

    Ok(())
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
            &serde_json::to_string_pretty(&Output::TrackPlayed(Some(playing))).unwrap(),
        );
    }
}

async fn things(
    later_playing_id: String,
    track: Track,
    peers: PeerMap,
    pool: Pool,
) -> Result<(), Box<dyn Error>> {
    let playing_duration = Duration::from_millis(track.duration_ms as u64);
    let next_up_duration = Duration::from_millis(5000);

    async_std::task::sleep(playing_duration - next_up_duration).await;

    // Have we changed playing song because of a downvote? If so return from this function to stop queueing.
    let playing = get_room("the_room", &pool.get()?)?.unwrap().playing;
    if Some(&later_playing_id) != playing.as_ref().map(|x| &x.id) {
        return Ok(());
    }

    set_next_up_from_queue(pool.clone(), peers.clone())?;

    async_std::task::sleep(Duration::from_millis(5000)).await;

    // Have we changed playing song because of a downvote? If so return from this function to stop more playing.
    let playing = get_room("the_room", &pool.get()?)?.unwrap().playing;
    if Some(later_playing_id) != playing.map(|x| x.id) {
        return Ok(());
    }

    play_from_next_up(pool, peers)
}

fn play(room: &mut Room, pool: Pool, peers: PeerMap, track: Track, now: u64) {
    let playing = Playing {
        id: track.id.clone(),
        name: track.name.clone(),
        uri: track.uri.clone(),
        duration_ms: track.duration_ms.clone(),
        started: now,
        downvotes: HashSet::new(),
    };

    let later_playing_id = playing.id.clone();

    room.playing = Some(playing);

    println!("Now playing: {:?} in room {:?}", track, room);

    let later_peers = peers.clone();
    let later_pool = pool.clone();
    task::spawn(async move { things(later_playing_id, track, later_peers, later_pool) });
}

fn play_from_next_up(later_pool: Pool, later_peers: PeerMap) -> Result<(), Box<dyn Error>> {
    let conn = later_pool.get()?;
    let mut room = get_room("the_room", &conn)?.unwrap();

    let next_track_wrap = { room.next_up.take() };

    if let Some(next_track) = next_track_wrap {
        play(
            &mut room,
            later_pool,
            later_peers.clone(),
            next_track,
            current_unix_epoch()?,
        );
    } else {
        room.playing = None;
    }

    prune_djs_without_queue(&mut room);

    update_room(room.clone(), &conn)?;

    send_to_all(
        later_peers,
        &serde_json::to_string_pretty(&Output::RoomState(room))?,
    );

    Ok(())
}

fn set_next_up_from_queue(later_pool: Pool, later_peers: PeerMap) -> Result<(), Box<dyn Error>> {
    let conn = later_pool.get()?;
    let mut room = get_room("the_room", &conn)?.unwrap();

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
            &serde_json::to_string_pretty(&Output::NextTrackQueued(next))?,
        );
    }

    update_room(room, &conn)?;

    Ok(())
}

/// Returns true if downvotes crossed threshold for skipping song.
fn downvote(user_id: String, room: &mut Room, now: u64, pool: Pool, peers: PeerMap) -> bool {
    if let Some(playing) = &mut room.playing {
        playing.downvotes.insert(user_id);
    }

    if let Some(playing) = &room.playing {
        let djs_len: usize = room.djs.as_ref().map(|x| x.len()).unwrap_or(0);

        if playing.downvotes.len() as f64 >= ((room.users.len() + djs_len) as f64 / 2f64) {
            room.playing = None;

            if let Some(djs) = &mut room.djs {
                next_djs(djs);
            }

            start_playing_if_theres_a_dj(room, &pool, peers.clone(), now);
            prune_djs_without_queue(room);

            true
        } else {
            false
        }
    } else {
        false
    }
}

fn load_certs(path: &Path) -> io::Result<Vec<Certificate>> {
    certs(&mut BufReader::new(File::open(path)?))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid cert"))
}

fn load_keys(path: &Path) -> io::Result<Vec<PrivateKey>> {
    rsa_private_keys(&mut BufReader::new(File::open(path)?))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid key"))
}

fn heartbeat_check(peers_db: PeerMap, pool: Pool) -> Result<(), Box<dyn Error>> {
    let now = current_unix_epoch()?;
    let conn = pool.get()?;
    let mut peers = peers_db.lock().unwrap();

    // Remove connections that haven't sent a heartbeat in 5 seconds.
    peers.retain(|_, v| {
        if now - v.last_heartbeat < CONNECTION_TIMEOUT {
            true
        } else {
            v.tx.close_channel();

            false
        }
    });

    let mut room = get_room("the_room", &conn)?.unwrap();
    // Remove users that have been disconnected for more than 15 seconds.
    let user_ids = room
        .users
        .iter_mut()
        .chain(room.djs.iter_mut().flat_map(|x| x.iter_mut()))
        .filter_map(|x| {
            if let Some(last_disconnect) = x.last_disconnect {
                if now - last_disconnect < ROOM_TIMEOUT {
                    None
                } else {
                    Some(x.id.clone())
                }
            } else {
                None
            }
        })
        .collect();

    remove_user_from_room(&mut room, user_ids);

    update_room(room, &conn)?;

    let pool = pool.clone();
    let peers_db = peers_db.clone();
    task::spawn(async move {
        async_std::task::sleep(Duration::from_millis(2000)).await;

        heartbeat_check(peers_db, pool).unwrap();
    });

    Ok(())
}

fn setup_db(mut conn: rusqlite::Connection) -> Result<(), refinery::Error> {
    embedded::migrations::runner().run(&mut conn)
}

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

async fn run() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    dotenv::dotenv().ok();

    let manager = SqliteConnectionManager::file("./db.sqlite");
    //let manager = SqliteConnectionManager::memory();
    let pool = r2d2::Pool::new(manager)?;

    let migrate_conn = rusqlite::Connection::open("./db.sqlite")?;
    setup_db(migrate_conn)?;

    let peers = PeerMap::new(Mutex::new(HashMap::new()));

    let mut config = ServerConfig::new(NoClientAuth::new());

    let ssl_cert = env::var("SSL_CERT");
    let ssl_key = env::var("SSL_KEY");

    let acceptor = if ssl_cert.is_ok() && ssl_key.is_ok() {
        let certs = load_certs(Path::new(&ssl_cert?))?;
        let mut keys = load_keys(Path::new(&ssl_key?))?;
        config.set_single_cert(certs, keys.remove(0))?;

        Some(TlsAcceptor::from(Arc::new(config)))
    } else {
        None
    };

    let url = format!("{}:{}", env::var("SERVER_URL")?, env::var("PORT")?);
    let listener = TcpListener::bind(&url).await?;
    println!("Listening on: {}", &url);

    heartbeat_check(peers.clone(), pool.clone())?;

    let now = current_unix_epoch()?;
    let init_conn = &pool.get()?;
    let mut room = get_room("the_room", init_conn)?.unwrap();

    start_playing_if_theres_a_dj(&mut room, &pool, peers.clone(), now);

    for user in room
        .users
        .iter_mut()
        .chain(room.djs.iter_mut().flat_map(|x| x.iter_mut()))
    {
        user.last_disconnect = Some(now);
    }

    update_room(room, init_conn)?;

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        let peers = peers.clone();
        let acceptor = acceptor.clone();
        let pool = pool.clone();

        task::spawn(async move {
            handle_connection(peers, stream, &acceptor, addr, pool)
                .await
                .unwrap()
        });
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    task::block_on(run())
}

#[cfg(test)]
mod test {
    use crate::*;
    use shared::lib::{TrackArt, User};

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

        setup_db(migrate_conn).unwrap();

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
            artwork: TrackArt::default(),
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
            artwork: TrackArt::default(),
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

        assert_eq!(res.unwrap(), Some(room));
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
            artwork: TrackArt::default(),
        };

        let dj = User {
            id: "test_dj".into(),
            queue: vec![track.clone()],
            last_disconnect: Some(1234),
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

    #[test]
    fn test_downvote() {
        let (pool, _) = &setup_test_db("test_downvote");
        let peers = PeerMap::new(Mutex::new(HashMap::new()));

        let playing = Playing {
            id: "currenttrack".into(),
            name: "Cool jazz stuff".into(),
            uri: "spotify:track:currenttrack".into(),
            duration_ms: 16345340,
            started: 12345,
            downvotes: HashSet::new(),
        };

        let next_track = Track {
            id: "nexttrack".into(),
            name: "Rockin stuff".into(),
            uri: "spotify:track:nexttrack".into(),
            duration_ms: 7364629,
            artwork: TrackArt::default(),
        };

        let other_user = User {
            id: "other_user".into(),
            queue: vec![],
            last_disconnect: Some(1234),
        };

        let current_dj = User {
            id: "current_dj".into(),
            queue: vec![],
            last_disconnect: Some(1234),
        };

        let next_dj = User {
            id: "next_dj".into(),
            queue: vec![next_track],
            last_disconnect: Some(1234),
        };

        let mut room = Room {
            id: "test_room".into(),
            users: vec![other_user.clone()],
            djs: Some(Zipper {
                before: vec![],
                current: current_dj.clone(),
                after: vec![next_dj.clone()],
            }),
            playing: Some(playing.clone()),
            next_up: None,
        };

        downvote(
            "test_dj".into(),
            &mut room,
            734,
            pool.clone(),
            peers.clone(),
        );
        downvote("other_dj".into(), &mut room, 734, pool.clone(), peers);

        let expected_playing = Playing {
            id: "nexttrack".into(),
            name: "Rockin stuff".into(),
            uri: "spotify:track:nexttrack".into(),
            duration_ms: 7364629,
            started: 734,
            downvotes: HashSet::new(),
        };

        let expected_room = Room {
            id: "test_room".into(),
            users: vec![other_user, current_dj],
            djs: Some(Zipper {
                before: vec![],
                current: User {
                    queue: vec![],
                    ..next_dj
                },
                after: vec![],
            }),
            playing: Some(expected_playing.clone()),
            next_up: None,
        };

        assert_eq!(room, expected_room);
    }
}
