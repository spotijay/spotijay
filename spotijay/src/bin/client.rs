use tungstenite::{connect, Message};
use url::Url;

pub struct ClientApp {

}

fn main() {
    env_logger::init();

    let _app = ClientApp {

    };

    let (mut socket, response) =
        connect(Url::parse("ws://localhost:3012/").unwrap()).expect("Can't connect");

    println!("Connected to the server");
    println!("Response HTTP code: {}", response.status());
    println!("Response contains the following headers:");
    for (ref header, _value) in response.headers() {
        println!("* {}", header);
    }

    socket
        .write_message(Message::Text("{\"Authenticate\": {\"id\": \"test_user_id\"}}".into()))
        .unwrap();
    loop {
        let msg = socket.read_message().expect("Error reading message");
        println!("Received: {}", msg);
    }
    // socket.close(None);
}