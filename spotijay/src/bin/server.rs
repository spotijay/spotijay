use std::{
    collections::HashMap,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use futures::{
    channel::mpsc::{unbounded, UnboundedSender},
    future, pin_mut,
    stream::TryStreamExt,
    StreamExt,
};

use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use std::time::Duration;
use tungstenite::protocol::Message;

use shared::lib::{now, Input, Output, Playing, Room, Track, User, Zipper};

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Peer>>>;
type RoomDb = Arc<Mutex<Room>>;

struct Peer {
    pub tx: Tx,
    pub user_id: Option<String>,
}

async fn handle_connection(
    peers_wrap: PeerMap,
    db_wrap: RoomDb,
    raw_stream: TcpStream,
    addr: SocketAddr,
) {
    println!("Incoming TCP connection from: {}", addr);

    let ws_stream = async_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");
    println!("WebSocket connection established: {}", addr);

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
        let input: Result<Input, serde_json::Error> = serde_json::from_str(msg.to_text().unwrap());
        println!("Received a message from {}: {:?}", addr, input);

        if let Err(e) = input {
            println!(
                "Couldn't deserialize: {:?}, serde error: {:?}",
                msg.to_text(),
                e
            );
        } else if let Ok(val) = input {
            match val {
                Input::Authenticate(user_id) => {
                    peers_wrap.lock().unwrap().get_mut(&addr).unwrap().user_id = Some(user_id);

                    // Send complete room state.
                    tx.unbounded_send(
                        serde_json::to_string_pretty(&Output::RoomState(
                            db_wrap.lock().unwrap().clone(),
                        ))
                        .unwrap()
                        .into(),
                    )
                    .unwrap();
                }
                Input::JoinRoom(user) => {
                    let mut db = db_wrap.lock().unwrap();

                    // Just one room for now.
                    if !db.users.iter().any(|x| x.id == user.id) {
                        if let Some(djs) = db.djs.clone() {
                            if !djs.iter().any(|x| x.id == user.id) {
                                db.users.push(user.clone());
                            }
                        } else {
                            db.users.push(user.clone());
                        }
                    }

                    // Send complete room state.
                    tx.unbounded_send(
                        serde_json::to_string_pretty(&Output::RoomState(db.clone()))
                            .unwrap()
                            .into(),
                    )
                    .unwrap();
                    println!("wat {:?}", db);
                }
                Input::BecomeDj(user_id) => {
                    {
                        let mut db = db_wrap.lock().unwrap();

                        let user = db.users.iter().find(|x| x.id == user_id);

                        if let Some(user) = user {
                            let user = user.clone();

                            if user.queue.len() > 0 {
                                db.users.retain(|x| x.id != user_id);

                                if let Some(djs) = &mut db.djs {
                                    djs.after.push(user);
                                } else {
                                    db.djs = Some(Zipper::singleton(user))
                                }
                            }
                        }
                    }

                    start_playing_if_theres_a_dj(db_wrap.clone(), peers_wrap.clone());

                    let peers = peers_wrap.lock().unwrap();
                    let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                    for recp in recipients {
                        let output = Output::RoomState(db_wrap.lock().unwrap().clone());
                        recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                            .unwrap();
                    }
                }
                Input::UnbecomeDj(_user_id) => {
                    unimplemented!();
                }
                Input::AddTrack(user_id, track) => {
                    {
                        let mut db = db_wrap.lock().unwrap();

                        if let Some(this_user) = db.users.iter_mut().find(|x| x.id == user_id) {
                            this_user.queue.push(track.clone());
                        } else if let Some(djs) = &mut db.djs {
                            let this_dj = djs.iter_mut().find(|x| x.id == user_id);

                            if let Some(dj) = this_dj {
                                dj.queue.push(track.clone());
                            }
                        }
                    }

                    start_playing_if_theres_a_dj(db_wrap.clone(), peers_wrap.clone());

                    let peers = peers_wrap.lock().unwrap();
                    let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                    for recp in recipients {
                        let output = Output::RoomState(db_wrap.lock().unwrap().clone());
                        recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                            .unwrap();
                    }
                }
                Input::RemoveTrack(user_id, track_id) => {
                    {
                        let mut db = db_wrap.lock().unwrap();

                        if let Some(this_user) = db.users.iter_mut().find(|x| x.id == user_id) {
                            this_user.queue.retain(|x| x.id != track_id);
                        } else if let Some(djs) = &mut db.djs {
                            let this_dj = djs.iter_mut().find(|x| x.id == user_id);

                            if let Some(dj) = this_dj {
                                dj.queue.retain(|x| x.id != track_id);
                            }
                        }
                    }

                    {
                        prune_djs_without_queue(&mut db_wrap.lock().unwrap());
                    }

                    start_playing_if_theres_a_dj(db_wrap.clone(), peers_wrap.clone());

                    let peers = peers_wrap.lock().unwrap();
                    let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                    for recp in recipients {
                        let output = Output::RoomState(db_wrap.lock().unwrap().clone());
                        recp.unbounded_send(serde_json::to_string_pretty(&output).unwrap().into())
                            .unwrap();
                    }
                }
            }
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);
    peers_wrap.lock().unwrap().remove(&addr);
}

fn send_to_all(peers: PeerMap, data: &String) {
    let peers = peers.lock().unwrap();
    let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

    for recp in recipients {
        recp.unbounded_send(Message::text(data)).unwrap();
    }
}

fn start_playing_if_theres_a_dj(db_wrap: RoomDb, peers: PeerMap) {
    let play_new_track = {
        let mut db = db_wrap.lock().unwrap();

        if let None = db.playing {
            if let Some(djs) = &mut db.djs {
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
        play(db_wrap.clone(), peers.clone(), track.clone());

        let playing = Playing {
            name: track.name.clone(),
            uri: track.uri.clone(),
            duration_ms: track.duration_ms.clone(),
            started: now(),
        };

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

fn play(db_wrap: RoomDb, peers: PeerMap, track: Track) -> () {
    let playing = Playing {
        name: track.name.clone(),
        uri: track.uri.clone(),
        duration_ms: track.duration_ms.clone(),
        started: now(),
    };
    db_wrap.clone().lock().unwrap().playing = Some(playing);
    println!("Now playing: {:?}", track);

    let later_peers = peers.clone();
    let later_peers2 = peers.clone();
    let later_db = db_wrap.clone();
    let later_db2 = db_wrap.clone();
    task::spawn(async move {
        let playing_duration = Duration::from_millis(track.duration_ms as u64);
        let next_up_duration = Duration::from_millis(5000);

        async_std::task::sleep(playing_duration - next_up_duration).await;

        let mut db = later_db.lock().unwrap();

        if let Some(djs) = &mut db.djs {
            next_djs(djs);

            if djs.current.queue.len() > 0 {
                db.next_up = Some(djs.current.queue.remove(0));
            } else {
                db.next_up = None;
            }
        }
        println!("Next up queued: {:?}", db.next_up);

        if let Some(next) = db.next_up.clone() {
            send_to_all(
                later_peers,
                &serde_json::to_string_pretty(&Output::NextTrackQueued(next)).unwrap(),
            );
        }

        task::spawn(async move {
            async_std::task::sleep(Duration::from_millis(5000)).await;

            let next_track_wrap = { later_db2.lock().unwrap().next_up.take() };

            if let Some(next_track) = next_track_wrap {
                play(db_wrap, later_peers2.clone(), next_track);
            } else {
                later_db2.lock().unwrap().playing = None;
            }

            prune_djs_without_queue(&mut later_db2.lock().unwrap());

            send_to_all(
                later_peers2,
                &serde_json::to_string_pretty(&Output::RoomState(
                    later_db2.lock().unwrap().clone(),
                ))
                .unwrap(),
            );
        });
    });
}

async fn run() -> Result<(), IoError> {
    env_logger::init();

    // In memory DB to begin with.
    let room = Room {
        id: "the_room".into(),
        users: vec![],
        djs: None,
        playing: None,
        next_up: None,
    };

    let db = Arc::new(Mutex::new(room));

    let peers = PeerMap::new(Mutex::new(HashMap::new()));

    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind("127.0.0.1:3012").await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: 127.0.0.1:3012");

    start_playing_if_theres_a_dj(db.clone(), peers.clone());

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        task::spawn(handle_connection(peers.clone(), db.clone(), stream, addr));
    }

    Ok(())
}

fn main() -> Result<(), IoError> {
    task::block_on(run())
}
