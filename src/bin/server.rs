use rustorm::EntityManager;
use rusqlite::Connection;
use std::net::TcpListener;
use std::thread::spawn;

use tungstenite::accept_hdr;
use tungstenite::handshake::server::{Request, Response};

use rustorm::Pool;
use rustorm::{DbError, FromDao, ToDao, ToColumnNames};

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("./migrations");
}

#[derive(Debug, PartialEq, FromDao, ToDao, ToColumnNames)]
struct Dj {
    dj_id: i32,
    room_id: i32,
    spotify_user_id: String,
    spotify_display_name: String
}

impl rustorm::dao::ToTableName for Dj {
    fn to_table_name() -> rustorm::TableName {
        rustorm::TableName {
            name: "djs".into(),
            schema: None,
            alias: None
        }
    }
}

fn migrate() {
    let mut conn = Connection::open("db.sqlite").unwrap();
    embedded::migrations::runner().run(&mut conn).unwrap();
}

fn load_fixtures(entity_manager: &mut EntityManager) {
    let dj_cool = Dj {
        dj_id: 1,
        room_id: 1,
        spotify_user_id: "awgwegwaeg".into(),
        spotify_display_name: "DJ Cool".into()
    };
    let _: Result<Vec<Dj>, DbError> = entity_manager.insert(&[&dj_cool]);

    let djs_all_query = "SELECT * FROM djs";
    let djs: Result<Vec<Dj>, DbError> = entity_manager.execute_sql_with_return(djs_all_query, &[]);

    println!("{:?}", djs);

    assert_eq!(djs.unwrap(), vec![dj_cool]);
}

fn main() {
    env_logger::init();

    migrate();

    let mut db_pool = Pool::new();
    let mut em = db_pool.em("sqlite://db.sqlite").unwrap();
    
    load_fixtures(&mut em);

    let server = TcpListener::bind("127.0.0.1:3012").unwrap();
    for stream in server.incoming() {
        spawn(move || {
            let callback = |req: &Request, mut response: Response| {
                println!("Received a new ws handshake");
                println!("The request's path is: {}", req.uri().path());
                println!("The request's headers are:");
                for (ref header, _value) in req.headers() {
                    println!("* {}", header);
                }

                // Let's add an additional header to our response to the client.
                let headers = response.headers_mut();
                headers.append("MyCustomHeader", ":)".parse().unwrap());
                headers.append("SOME_TUNGSTENITE_HEADER", "header_value".parse().unwrap());

                Ok(response)
            };
            let mut websocket = accept_hdr(stream.unwrap(), callback).unwrap();

            loop {
                let msg = websocket.read_message().unwrap();
                if msg.is_binary() || msg.is_text() {
                    websocket.write_message(msg).unwrap();
                }
            }
        });
    }
    
}