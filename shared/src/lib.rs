pub mod lib {
    use iter::{Chain, Once};
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashSet,
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
        pub uri: String,
        pub duration_ms: u32,
        pub artwork: TrackArt,
    }

    #[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
    pub struct Playing {
        pub id: String,
        pub name: String,
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
                uri: track.uri,
                duration_ms: track.duration_ms,
                started: started,
                downvotes: HashSet::new(),
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

        pub fn iter<'a>(&'a self) -> ZipperIter<T> {
            self.before
                .iter()
                .chain(iter::once(&self.current))
                .chain(self.after.iter())
        }

        pub fn iter_mut<'a>(&mut self) -> ZipperIterMut<T> {
            self.before
                .iter_mut()
                .chain(iter::once(&mut self.current))
                .chain(self.after.iter_mut())
        }
    }

    pub type ZipperIter<'a, T> = Chain<Chain<Iter<'a, T>, Once<&'a T>>, Iter<'a, T>>;
    pub type ZipperIterMut<'a, T> = Chain<Chain<IterMut<'a, T>, Once<&'a mut T>>, IterMut<'a, T>>;

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

    pub fn current_unix_epoch() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    pub fn next_djs(djs: &mut Zipper<User>) {
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

    pub fn prune_djs_without_queue(room: &mut Room) {
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
}
