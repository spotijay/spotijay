#[macro_use]
extern crate dotenv_codegen;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{HtmlInputElement, Request, RequestInit, RequestMode, Response};

use gloo::events::EventListener;
use log::Level;
use serde::Serialize;
use url::Url;

#[derive(Serialize)]
struct PlayRequest {
    uris: Vec<String>,
}

fn get_login_url() -> url::Url {
    Url::parse_with_params(
        "https://accounts.spotify.com/authorize",
        &[
            ("client_id", dotenv!("CLIENT_ID")),
            ("response_type", "token"),
            ("redirect_uri", "http://localhost:8000/callback"),
            ("scope", "user-modify-playback-state"),
        ],
    )
    .unwrap()
}

async fn play(access_token: String, uri: String) {
    let body = serde_json::to_string(&PlayRequest { uris: vec![uri] }).unwrap();

    let mut opts = RequestInit::new();
    opts.method("PUT");
    opts.mode(RequestMode::Cors);
    opts.body(Some(&JsValue::from(body)));

    let url = "https://api.spotify.com/v1/me/player/play";

    let request = Request::new_with_str_and_init(&url, &opts).unwrap();

    request
        .headers()
        .set("Authorization", &format!("Bearer {}", &access_token))
        .unwrap();

    let window = web_sys::window().expect("no global `window` exists");
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .unwrap();

    let resp: Response = resp_value.dyn_into().unwrap();
    JsFuture::from(resp.json().unwrap()).await.unwrap();
}

#[wasm_bindgen(start)]
pub async fn main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(Level::Debug).unwrap();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let body = document.body().unwrap();

    let mut access_token_wrapper = None;

    if window.location().pathname().unwrap() == "/callback" {
        let url = Url::parse(&window.location().href().unwrap().replace("#", "?")).unwrap();
        let query_pairs = url.query_pairs();
        access_token_wrapper = Some(
            query_pairs
                .clone()
                .find(|x| x.0 == "access_token")
                .unwrap()
                .1
                .into_owned(),
        );

        // TODO: check this before every request to log user out automatically when it expires.
        // We can also regularily poll, as a long setTimeout doesn't work, it gets cancelled.
        let _expires_in = query_pairs.clone().find(|x| x.0 == "expires_in").unwrap();

        window
            .history()
            .unwrap()
            .push_state_with_url(&JsValue::NULL, &"Spotijay", Some("/"))
            .unwrap();
    }

    if let Some(access_token) = access_token_wrapper {
        let play_input = document.create_element("input").unwrap();
        play_input.set_id("play_input");
        body.append_child(&play_input).unwrap();

        let play_button = document.create_element("button").unwrap();
        play_button.set_inner_html("Play");
        body.append_child(&play_button).unwrap();

        let play_button_click_listener = EventListener::new(&play_button, "click", move |_| {
            let token = access_token.clone();

            // TODO: Use a global to store this from an event listener instead. Or just use a framework like Seed.
            let play_input_el = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .get_element_by_id("play_input")
                .unwrap();
            let play_input_value = JsCast::dyn_ref::<HtmlInputElement>(&play_input_el)
                .unwrap()
                .value();

            spawn_local(play(token, play_input_value.into()));
        });

        play_button_click_listener.forget();
    }

    let login_button = document.create_element("button").unwrap();
    login_button.set_inner_html("Login");

    body.append_child(&login_button).unwrap();

    let login_button_click_listener = EventListener::new(&login_button, "click", move |_| {
        window
            .location()
            .set_href(get_login_url().as_str())
            .unwrap();
    });

    login_button_click_listener.forget();
}
