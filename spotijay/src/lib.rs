pub mod types {
    use serde::{Deserialize, Serialize};
    use std::{time::SystemTime, collections::HashMap};

    // TODO: Display name and avatar
    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct User {
        pub id: String,
        pub queue: Vec<Track>
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Track {
        pub uri: String,
        pub duration_ms: u32,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Playing {
        pub uri: String,
        pub duration_ms: u32,
        pub started: u64,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Djs {
        pub before: Vec<User>,
        pub current: User,
        pub after: Vec<User>
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Room {
        pub id: String,
        pub users: Vec<User>,
        pub djs: Option<Djs>,
        pub playing: Option<Playing>,
        pub next_up: Option<Track>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub enum Input {
        Authenticate(User),
        JoinRoom(User),
        AddTrack(String, String, u32),
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Output {
        RoomState(Room),
        RoomJoined(User),
        TrackAdded(Track),
    }
}
