pub mod lib {
    use serde::{Deserialize, Serialize};
    use std::{time::SystemTime};

    // TODO: Display name and avatar
    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct User {
        pub id: String,
        pub queue: Vec<Track>
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Track {
        pub id: String,
        pub name: String,
        pub uri: String,
        pub duration_ms: u32,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Playing {
        pub name: String,
        pub uri: String,
        pub duration_ms: u32,
        pub started: u64,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Zipper<T> {
        pub before: Vec<T>,
        pub current: T,
        pub after: Vec<T>
    }

    impl<T> Zipper<T> {
        pub fn len(&self) -> usize {
            self.before.len() + self.after.len() + 1
        }
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Room {
        pub id: String,
        pub users: Vec<User>,
        pub djs: Option<Zipper<User>>,
        pub playing: Option<Playing>,
        pub next_up: Option<Track>,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Input {
        Authenticate(String),
        JoinRoom(User),
        BecomeDj(String),
        UnbecomeDj(String),
        AddTrack(String, Track),
        RemoveTrack(String, String),
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Output {
        RoomState(Room),
        TrackPlayed(Playing),
        NextTrackQueued(Track),
    }

    pub fn now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
