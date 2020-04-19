use js_sys::Function;
use seed::{prelude::*, *};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use shared::lib::{Input, Output, Room, Track, User};

mod pages;
mod spotify;
use pages::Page;

const WS_URL: &str = dotenv_codegen::dotenv!("SERVER_WS_URL");

const SPOTIFY_PROFILE_URL: &str = "https://api.spotify.com/v1/me";
const SPOTIFY_PLAY_URL: &str = "https://api.spotify.com/v1/me/player/play";
const SPOTIFY_QUEUE_URL: &str = "https://api.spotify.com/v1/me/player/queue";
const SPOTIFY_SEARCH_URL: &str = "https://api.spotify.com/v1/search";

#[derive(Debug)]
struct Model {
    data: Data,
    page: Page,
    services: Services,
}

#[derive(Debug)]
enum Data {
    Authed(AuthedModel),
    UnAuthed(),
}

#[derive(Debug)]
struct AuthedModel {
    auth: Auth,
    search: String,
    room: Option<Room>,
    profile: Option<spotify::SpotifyProfile>,
    search_result: Option<spotify::SpotifySearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Auth {
    access_token: String,
    expires_epoch: u64,
}

impl AuthedModel {
    fn new(auth: Auth) -> Self {
        AuthedModel {
            auth: auth,
            search: "".into(),
            room: None,
            profile: None,
            search_result: None,
        }
    }
}

#[derive(Debug)]
struct Services {
    ws: WebSocket,
    ls: seed::browser::service::storage::Storage,
}

fn before_mount(_: Url) -> BeforeMount {
    // Since we have the "loading..." text in the app section of index.html,
    // we use MountType::Takover which will overwrite it with the seed generated html
    BeforeMount::new().mount_type(MountType::Takeover)
}

fn after_mount(url: Url, orders: &mut impl Orders<Msg>) -> AfterMount<Model> {
    let ls = storage::get_storage().expect("get `LocalStorage`");
    let ws = WebSocket::new(WS_URL).unwrap();

    register_ws_handler(WebSocket::set_onopen, Msg::Connected, &ws, orders);
    register_ws_handler(WebSocket::set_onclose, Msg::Closed, &ws, orders);
    register_ws_handler(WebSocket::set_onmessage, Msg::ServerMessage, &ws, orders);
    register_ws_handler(WebSocket::set_onerror, Msg::Error, &ws, orders);

    let parsed_page = match url.path[0].as_ref() {
        "callback" => Page::Callback,
        _ => Page::Home,
    };

    if parsed_page == Page::Callback {
        orders.send_msg(Msg::LoggedIn);

        AfterMount::new(Model {
            data: Data::UnAuthed(),
            page: Page::Home,
            services: Services { ws, ls },
        })
    } else {
        let auth = ls
            .get_item("spotify_auth")
            .expect("try to get item from `LocalStorage`");

        let auth: Option<Auth> = auth.map(|x| serde_json::from_str(&x).unwrap());

        let data = match auth {
            Some(auth) => {
                if auth.access_token.is_empty() {
                    Data::UnAuthed()
                } else {
                    orders.perform_cmd(get_spotify_profile(auth.clone()));
                    Data::Authed(AuthedModel::new(auth))
                }
            }
            _ => Data::UnAuthed(),
        };

        AfterMount::new(Model {
            data: data,
            page: Page::Home,
            services: Services { ws, ls },
        })
    }
}

fn register_ws_handler<T, F>(
    ws_cb_setter: fn(&WebSocket, Option<&Function>),
    msg: F,
    ws: &WebSocket,
    orders: &mut impl Orders<Msg>,
) where
    T: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: Fn(T) -> Msg + 'static,
{
    let (app, msg_mapper) = (orders.clone_app(), orders.msg_mapper());

    let closure = Closure::new(move |data| {
        app.update(msg_mapper(msg(data)));
    });

    ws_cb_setter(ws, Some(closure.as_ref().unchecked_ref()));
    closure.forget();
}

#[derive(Debug, Clone)]
enum Msg {
    Connected(JsValue),
    ServerMessage(MessageEvent),
    Closed(JsValue),
    Error(JsValue),
    ChangePage(Page),
    AddTrack(String),
    RemoveTrack(String),
    SearchFetched(seed::fetch::ResponseDataResult<spotify::SpotifySearchResult>),
    ProfileFetched(seed::fetch::ResponseDataResult<spotify::SpotifyProfile>),
    LoggingIn,
    LoggedIn,
    Logout,
    Queued,
    Played,
    SearchChange(String),
    BecomeDj,
    UnbecomeDj,
}

async fn get_spotify_profile(auth: Auth) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::Logout).await
    } else {
        Request::new(SPOTIFY_PROFILE_URL)
            .header("Authorization", &format!("Bearer {}", auth.access_token))
            .fetch_json_data(Msg::ProfileFetched)
            .await
    }
}

async fn get_spotify_search(auth: Auth, search: String) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::Logout).await
    } else {
        let url = url::Url::parse_with_params(
            SPOTIFY_SEARCH_URL,
            &[("q", search), ("type", "track".into())],
        )
        .unwrap()
        .into_string();

        Request::new(url)
            .header("Authorization", &format!("Bearer {}", auth.access_token))
            .fetch_json_data(Msg::SearchFetched)
            .await
    }
}

async fn put_spotify_play(auth: Auth, uri: String, position_ms: u32) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::Logout).await
    } else {
        Request::new(SPOTIFY_PLAY_URL)
            .header("Authorization", &format!("Bearer {}", auth.access_token))
            .method(seed::browser::service::fetch::Method::Put)
            .send_json(&spotify::PlayRequest {
                uris: vec![uri],
                position_ms,
            })
            .fetch(|_| Msg::Played)
            .await
    }
}

async fn post_spotify_queue(auth: Auth, uri: String) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::Logout).await
    } else {
        let url = url::Url::parse_with_params(SPOTIFY_QUEUE_URL, &[("uri", uri)])
            .unwrap()
            .into_string();
        Request::new(url)
            .header("Authorization", &format!("Bearer {}", auth.access_token))
            .method(seed::browser::service::fetch::Method::Post)
            .fetch(|_| Msg::Queued)
            .await
    }
}

fn update(msg: Msg, model: &mut Model, orders: &mut impl Orders<Msg>) {
    log!("msg: ", msg);
    log!("model: ", model);

    match msg {
        Msg::Logout => model.data = Data::UnAuthed(),
        _ => match &mut model.data {
            Data::Authed(authed_model) => authed_update(msg, &model.services, authed_model, orders),
            Data::UnAuthed() => unauthed_update(msg, model, orders),
        },
    };
}

fn unauthed_update(msg: Msg, mut model: &mut Model, orders: &mut impl Orders<Msg>) {
    match msg {
        Msg::LoggingIn => {
            log!("Loggin in");
            window().location().set_href(&spotify::login_url()).unwrap();
        }
        Msg::LoggedIn => {
            let url =
                url::Url::parse(&window().location().href().unwrap().replace("#", "?")).unwrap();
            let query_pairs = url.query_pairs();
            let access_token = query_pairs
                .clone()
                .find(|x| x.0 == "access_token")
                .unwrap()
                .1
                .into_owned();

            // TODO: check this before every request to log user out automatically when it expires.
            // We can also regularily poll, as a long setTimeout doesn't work, it gets cancelled.
            let expires_in = query_pairs
                .clone()
                .find(|x| x.0 == "expires_in")
                .unwrap()
                .1
                .into_owned();

            let expires_epoch =
                js_sys::Date::now() as u64 + (expires_in.parse::<u64>().unwrap() * 1000);

            let auth = Auth {
                access_token,
                expires_epoch: expires_epoch,
            };
            storage::store_data(&model.services.ls, "spotify_auth", &auth);
            model.data = Data::Authed(AuthedModel::new(auth.clone()));

            seed::push_route(Page::Home.path());
            model.page = Page::Home;

            orders.perform_cmd(get_spotify_profile(auth.clone()));
        }
        Msg::ChangePage(page) => {
            seed::push_route(page.path());
            model.page = page;
        }
        Msg::Logout => {}
        Msg::Connected(_) => {
            log!("WebSocket connection is open now");
        }
        Msg::Closed(_) => {
            log!("WebSocket connection was closed");
        }
        Msg::Error(_) => {
            log!("Error");
        }
        _ => {
            error!("Invalid Msg for unauthed user: {:?}", msg);
        }
    }
}

fn authed_update(
    msg: Msg,
    services: &Services,
    authed_model: &mut AuthedModel,
    orders: &mut impl Orders<Msg>,
) {
    match msg {
        Msg::LoggingIn => {
            error!("Loggin in again after already logged in, oh no");
        }
        Msg::LoggedIn => {
            error!("Logged in again after already logged in, oh no");
        }
        Msg::Queued => {}
        Msg::Played => {}
        Msg::ChangePage(_) => {}
        Msg::Logout => {}
        Msg::Connected(_) => {}
        Msg::BecomeDj => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::BecomeDj(
                        authed_model.profile.clone().unwrap().id,
                    ))
                    .unwrap(),
                )
                .unwrap();
        }
        Msg::UnbecomeDj => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::UnbecomeDj(
                        authed_model.profile.clone().unwrap().id,
                    ))
                    .unwrap(),
                )
                .unwrap();
        }
        Msg::AddTrack(track_id) => {
            let track = authed_model
                .search_result
                .clone()
                .unwrap()
                .tracks
                .items
                .iter()
                .find(|x| x.id == track_id)
                .unwrap()
                .clone();

            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::AddTrack(
                        authed_model.profile.clone().unwrap().id,
                        Track {
                            id: track.id,
                            name: track.name,
                            uri: track.uri,
                            duration_ms: track.duration_ms,
                        },
                    ))
                    .unwrap(),
                )
                .unwrap();

            authed_model.search_result = None;
        }
        Msg::RemoveTrack(track_id) => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::RemoveTrack(
                        authed_model.profile.clone().unwrap().id,
                        track_id,
                    ))
                    .unwrap(),
                )
                .unwrap();

            authed_model.search_result = None;
        }
        Msg::ProfileFetched(res) => match res {
            Ok(profile) => {
                services
                    .ws
                    .send_with_str(
                        &serde_json::to_string(&Input::JoinRoom(User {
                            id: profile.id.clone(),
                            queue: vec![],
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                authed_model.profile = Some(profile)
            }
            Err(e) => error!("Profile error: {}", e),
        },
        Msg::ServerMessage(msg_event) => {
            log!("Client received a message");
            let txt = msg_event.data().as_string().unwrap();
            let json: Output = serde_json::from_str(&txt).unwrap();

            match json {
                Output::RoomState(room) => {
                    if let None = authed_model.room {
                        if let Some(playing) = &room.playing {
                            let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                            orders.perform_cmd(put_spotify_play(
                                authed_model.auth.clone(),
                                playing.uri.clone(),
                                offset_ms as u32,
                            ));
                        }
                    }

                    authed_model.room = Some(room);
                }
                Output::TrackPlayed(playing) => {
                    let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                    orders.perform_cmd(put_spotify_play(
                        authed_model.auth.clone(),
                        playing.uri.clone(),
                        offset_ms as u32,
                    ));
                }
                Output::NextTrackQueued(track) => {
                    orders.perform_cmd(post_spotify_queue(
                        authed_model.auth.clone(),
                        track.uri.clone(),
                    ));
                }
            }
        }
        Msg::Closed(_) => {
            log!("WebSocket connection was closed");
        }
        Msg::Error(_) => {
            log!("Error");
        }
        Msg::SearchChange(val) => {
            authed_model.search = val.clone();

            if !val.is_empty() {
                orders.perform_cmd(get_spotify_search(authed_model.auth.clone(), val.clone()));
            }
        }
        Msg::SearchFetched(res) => match res {
            Ok(search_results) => authed_model.search_result = Some(search_results),
            Err(e) => error!("Profile error: {}", e),
        },
    }
}

fn current_user(user_id: &str, room: &Room) -> Option<User> {
    if let Some(djs) = &room.djs {
        djs.iter()
            .chain(room.users.iter())
            .find(|x| x.id == user_id)
            .map(|x| x.clone())
    } else {
        room.users
            .iter()
            .find(|x| x.id == user_id)
            .map(|x| x.clone())
    }
}

fn users_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let room = authed_model.room.clone()?;
    let items = room.users.iter().map(|x| li![x.id]);

    Some(ul![items])
}

fn playlist_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let tracks = current_user(
        authed_model.profile.as_ref().map(|x| x.id.as_ref())?,
        authed_model.room.as_ref()?,
    )?
    .queue
    .into_iter();

    for track in tracks {
        let event_track = track.clone();

        items.push(li![button![
            ev(Ev::Click, move |_| Msg::RemoveTrack(event_track.id)),
            track.name
        ]]);
    }

    Some(ol![items])
}

fn search_result_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let tracks = authed_model.search_result.clone()?.tracks.items.into_iter();

    for track in tracks {
        let event_track = track.clone();

        items.push(li![button![
            ev(Ev::Click, move |_| Msg::AddTrack(event_track.id)),
            track.name
        ]]);
    }

    Some(ul![items])
}

fn djs_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let djs = authed_model.room.clone().and_then(|x| x.djs);

    if let Some(djs) = djs {
        let playing = authed_model
            .room
            .clone()
            .and_then(|x| x.playing.map(|x| x.name))
            .unwrap_or("Nothing!".to_string());

        for dj in djs.iter() {
            items.push(li![button![
                ev(Ev::Click, move |_| Msg::UnbecomeDj),
                if dj.id == djs.current.id {
                    div![b![dj.id], format!(" - {}", playing)]
                } else {
                    span![dj.id]
                }
            ]]);
        }

        if djs
            .iter()
            .any(|x| x.id != authed_model.profile.clone().unwrap().id)
        {
            items.push(li![button![
                simple_ev(Ev::Click, Msg::BecomeDj),
                "Become a DJ"
            ]]);
        }
    } else {
        items.push(li![button![
            simple_ev(Ev::Click, Msg::BecomeDj),
            "Become a DJ"
        ]]);
    }

    Some(ol![items])
}

fn authed_view(authed_model: &AuthedModel) -> Node<Msg> {
    div![
        h2![format!(
            "logged in {}!",
            authed_model
                .profile
                .clone()
                .map(|x| x.display_name)
                .unwrap_or("".to_string())
        )],
        djs_view(authed_model).unwrap_or(div![]),
        if let Some(items) = playlist_view(authed_model) {
            div![
                h3!["Your playlist"],
                input![
                    attrs! {At::Placeholder => "Search for tracks"},
                    input_ev(Ev::Input, Msg::SearchChange)
                ],
                search_result_view(authed_model).unwrap_or(items),
            ]
        } else {
            div![]
        },
        div![
            h3!["Users"],
            users_view(authed_model).unwrap_or(div!["No users"])
        ]
    ]
}

fn unauthed_view() -> Node<Msg> {
    div![button![simple_ev(Ev::Click, Msg::LoggingIn), "Login"]]
}

fn data_view(data: &Data) -> Node<Msg> {
    match &data {
        Data::Authed(authed) => {
            if authed.profile.is_some() && authed.room.is_some() {
                authed_view(authed)
            } else {
                h1!["Loading"]
            }
        }
        Data::UnAuthed() => unauthed_view(),
    }
}

fn view(model: &Model) -> impl View<Msg> {
    let data = &model.data;

    vec![h1!["Spotijay"], data_view(data)]
}

fn routes(url: Url) -> Option<Msg> {
    let parsed_page = Page::from_url(url);

    if parsed_page == Some(Page::Callback) {
        Some(Msg::LoggedIn)
    } else {
        parsed_page.map(Msg::ChangePage)
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    App::builder(update, view)
        .before_mount(before_mount)
        .after_mount(after_mount)
        .routes(routes)
        .build_and_start();
}
