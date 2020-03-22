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
use std::time::SystemTime;
use tungstenite::protocol::Message;

use spotijay::types::{Input, Output, Room, Track, User};

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Peer>>>;
type RoomDb = Arc<Mutex<Room>>;

struct Peer {
    pub tx: Tx,
    pub user_id: Option<String>,
}

async fn handle_connection(peers: PeerMap, db: RoomDb, raw_stream: TcpStream, addr: SocketAddr) {
    println!("Incoming TCP connection from: {}", addr);

    let ws_stream = async_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");
    println!("WebSocket connection established: {}", addr);

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peers.lock().unwrap().insert(
        addr,
        Peer {
            tx: tx.clone(),
            user_id: None,
        },
    );

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        let input: Input = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        println!("Received a message from {}: {:?}", addr, input);
        let mut peers = peers.lock().unwrap();
        let mut db = db.lock().unwrap();

        match input {
            Input::Authenticate(user) => {
                peers.get_mut(&addr).unwrap().user_id = Some(user.id);

                // Send complete room state.
                tx.unbounded_send(
                    serde_json::to_string(&Output::RoomState(db.clone()))
                        .unwrap()
                        .into(),
                );
            }
            Input::JoinRoom(user) => {
                // Just one room for now.
                db.users.push(user.clone());

                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomJoined(user.clone());
                    recp.unbounded_send(serde_json::to_string(&output).unwrap().into())
                        .unwrap();
                }
            }
            Input::AddTrack(track) => {
                // Spotify's `/player/play` API silently drops everything on dupes, so let's just ignore those.
                if !db.queue.iter().any(|x| x.uri == track.uri) {
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;

                    let offset = now - db.started;
                    let queue_length: u32 = db.queue.iter().map(|x| x.duration_ms).sum();

                    // Old queue is done, make a new one.
                    if offset > (queue_length as u64) {
                        db.queue = vec![track.clone()];
                        db.started = now;
                    } else {
                        db.queue.push(track.clone());
                    }
                }

                let recipients = peers.iter().map(|(_, peer)| peer.tx.clone());

                for recp in recipients {
                    let output = Output::RoomState(db.clone());
                    recp.unbounded_send(serde_json::to_string(&output).unwrap().into())
                        .unwrap();
                }
            }
        }

        // For debug purposes we broadcast every message to everyone except ourselves.
        let broadcast_recipients = peers
            .iter()
            .filter(|(peer_addr, _)| peer_addr != &&addr)
            .map(|(_, peer)| peer.tx.clone());

        for recp in broadcast_recipients {
            recp.unbounded_send(msg.clone()).unwrap();
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);
    peers.lock().unwrap().remove(&addr);
}

async fn run() -> Result<(), IoError> {
    env_logger::init();

    // In memory DB to begin with.
    let room = Room {
        id: "the_room".into(),
        users: Vec::new(),
        djs: vec![User {
            id: "DropDuck".into(),
        }],
        queue: vec![Track {
            uri: "spotify:track:6yIjtVtnOBeC8SwdVHzAuF".into(),
            duration_ms: 203733,
        }],
        started: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 180000,
    };
    let db = Arc::new(Mutex::new(room));

    let peers = PeerMap::new(Mutex::new(HashMap::new()));

    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind("127.0.0.1:3012").await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: 127.0.0.1:3012");

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().await {
        task::spawn(handle_connection(peers.clone(), db.clone(), stream, addr));
    }

    Ok(())
}

fn main() -> Result<(), IoError> {
    task::block_on(run())
}
