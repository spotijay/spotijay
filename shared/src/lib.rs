pub mod lib {
    use serde::{Deserialize, Serialize};
    use std::{
        collections::{HashSet, VecDeque},
        time::{SystemTime, SystemTimeError},
    };

    // TODO: Display name and avatar
    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct User {
        pub id: String,
        pub queue: Vec<Track>,
        pub last_disconnect: Option<u64>,
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct TrackArt {
        pub uri: String,
        pub height: u32,
        pub width: u32,
    }

    impl Default for TrackArt {
        fn default() -> Self {
            TrackArt {
                uri: "".to_owned(),
                height: 0,
                width: 0,
            }
        }
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Track {
        pub id: String,
        pub name: String,
        pub artist: String,
        pub uri: String,
        pub duration_ms: u32,
        pub artwork: TrackArt,
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Playing {
        pub id: String,
        pub name: String,
        pub artist: String,
        pub uri: String,
        pub duration_ms: u32,
        pub started: u64,
        pub downvotes: HashSet<String>,
    }

    impl Playing {
        pub fn new(track: Track, started: u64) -> Self {
            Playing {
                id: track.id,
                name: track.name,
                artist: track.artist,
                uri: track.uri,
                duration_ms: track.duration_ms,
                started: started,
                downvotes: HashSet::new(),
            }
        }
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Room {
        pub id: String,
        pub users: Vec<User>,
        pub djs: VecDeque<User>,
        pub playing: Option<Playing>,
        pub next_up: Option<Track>,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Input {
        Authenticate(String),
        JoinRoom(User),
        BecomeDj(String),
        AddTrack(String, Track),
        RemoveTrack(String, String),
        Downvote(String),
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Output {
        Authenticated(String),
        RoomState(Room),
        Downvoted(String),
        TrackPlayed(Option<Playing>),
        NextTrackQueued(Track),
    }

    pub fn current_unix_epoch() -> Result<u64, SystemTimeError> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64;

        Ok(now)
    }

    pub fn next_djs(djs: &mut VecDeque<User>) {
        loop {
            if djs.len() == 0 {
                break;
            }

            djs.rotate_left(1);

            if let Some(dj) = djs.front() {
                if dj.queue.len() > 0 {
                    break;
                } else {
                    djs.pop_front();
                }
            } else {
                break;
            }
        }
    }

    pub fn prune_djs_without_queue(room: &mut Room) {
        let current_dj = room.djs.pop_front();
        room.djs.retain(|x| x.queue.len() > 0);

        if let Some(current_dj) = current_dj {
            if current_dj.queue.len() != 0 || room.playing.is_some() {
                room.djs.push_front(current_dj);
            }
        }
    }
}
