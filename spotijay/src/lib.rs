pub mod types {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct User {
        pub id: String,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Track {
        pub uri: String,
        pub duration_ms: i32,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub struct Room {
        pub id: String,
        pub started: u64,
        pub users: Vec<User>,
        pub djs: Vec<User>,
        pub queue: Vec<Track>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub enum Input {
        Authenticate(User),
        JoinRoom(User),
        AddTrack(Track),
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    pub enum Output {
        RoomState(Room),
        RoomJoined(User),
        TrackAdded(Track),
    }
}
