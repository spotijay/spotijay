pub mod lib {
    use iter::{Chain, Once};
    use serde::{Deserialize, Serialize};
    use std::{
        iter,
        slice::{Iter, IterMut},
        time::SystemTime,
        vec::IntoIter,
    };

    // TODO: Display name and avatar
    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct User {
        pub id: String,
        pub queue: Vec<Track>,
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Track {
        pub id: String,
        pub name: String,
        pub uri: String,
        pub duration_ms: u32,
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Playing {
        pub id: String,
        pub name: String,
        pub uri: String,
        pub duration_ms: u32,
        pub started: u64,
    }

    impl Playing {
        pub fn new(track: Track, started: u64) -> Self {
            Playing {
                id: track.id,
                name: track.name,
                uri: track.uri,
                duration_ms: track.duration_ms,
                started: started,
            }
        }
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Zipper<T> {
        pub before: Vec<T>,
        pub current: T,
        pub after: Vec<T>,
    }

    impl<T> Zipper<T> {
        pub fn singleton(item: T) -> Zipper<T> {
            Zipper {
                before: vec![],
                after: vec![],
                current: item,
            }
        }

        pub fn len(&self) -> usize {
            self.before.len() + self.after.len() + 1
        }

        pub fn iter<'a>(&'a self) -> Chain<Chain<Iter<'a, T>, Once<&T>>, Iter<'a, T>> {
            self.before
                .iter()
                .chain(iter::once(&self.current))
                .chain(self.after.iter())
        }

        pub fn iter_mut<'a>(&mut self) -> Chain<Chain<IterMut<T>, Once<&mut T>>, IterMut<T>> {
            self.before
                .iter_mut()
                .chain(iter::once(&mut self.current))
                .chain(self.after.iter_mut())
        }
    }

    impl<T> IntoIterator for Zipper<T> {
        type Item = T;
        type IntoIter = Chain<Chain<IntoIter<T>, Once<T>>, IntoIter<T>>;

        fn into_iter(self) -> Self::IntoIter {
            self.before
                .into_iter()
                .chain(iter::once(self.current))
                .chain(self.after.into_iter())
        }
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
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
        Authenticated(String),
        RoomState(Room),
        TrackPlayed(Playing),
        NextTrackQueued(Track),
    }

    pub fn current_unix_epoch() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
