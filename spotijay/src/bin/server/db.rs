use rusqlite::{params};
use shared::lib::{next_djs, Room};
use crate::Connection;

pub type Result<T, E = Error> = std::result::Result<T, E>;

trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(Error::Rusqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Rusqlite(rusqlite::Error),
    Serde(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapperError is here!")
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Rusqlite(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e)
    }
}

pub fn get_room(room_id: &str, conn: &Connection) -> Result<Option<Room>, Error> {
    let res: rusqlite::Result<Room, Error> = conn.query_row_and_then(
        "SELECT id, users, playing, djs, next_up FROM room WHERE room.id = ?",
        params![room_id],
        |row| {
            let id: String = row.get(0)?;
            let users = serde_json::from_value(row.get(1)?)?;
            let playing = serde_json::from_value(row.get(2)?)?;
            let djs = serde_json::from_value(row.get(3)?)?;
            let next_up = serde_json::from_value(row.get(4)?)?;

            Ok(Room {
                id: id,
                users: users,
                playing: playing,
                djs: djs,
                next_up: next_up,
            })
        },
    );

    Ok(res.optional()?)
}

pub fn update_room(room: Room, conn: &Connection) -> rusqlite::Result<()> {
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

pub fn remove_user_from_room(room: &mut Room, user_ids: Vec<String>) {
    room.users.retain(|x| !user_ids.contains(&x.id));

    if let Some(djs) = &mut room.djs {
        djs.before.retain(|x| !user_ids.contains(&x.id));
        djs.after.retain(|x| !user_ids.contains(&x.id));

        if user_ids.contains(&djs.current.id) {
            if djs.before.len() == 0 && djs.after.len() == 0 {
                room.djs = None;
            } else {
                next_djs(djs);

                djs.before.pop();
                room.djs = Some(djs.to_owned());
            }
        }
    }
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
