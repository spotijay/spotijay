use gloo::timers::future::TimeoutFuture;
use js_sys::Function;
use seed::{prelude::*, *};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use shared::lib::{prune_djs_without_queue, Input, Output, Room, Track, TrackArt, User};

mod pages;
mod spotify;
use pages::Page;
use wasm_bindgen_futures::spawn_local;

const WS_URL: &str = dotenv_codegen::dotenv!("SERVER_WS_URL");

const SPOTIFY_PLAY_URL: &str = "https://api.spotify.com/v1/me/player/play";
const SPOTIFY_NEXT_URL: &str = "https://api.spotify.com/v1/me/player/next";
const SPOTIFY_QUEUE_URL: &str = "https://api.spotify.com/v1/me/player/queue";
const SPOTIFY_SEARCH_URL: &str = "https://api.spotify.com/v1/search";

#[derive(Debug)]
struct Model {
    data: Data,
    page: Page,
    connected: bool,
    services: Services,
}

#[derive(Debug)]
enum Data {
    Authed(AuthedModel),
    UnAuthed(UnauthedModel),
}

#[derive(Debug)]
struct AuthedModel {
    session: Session,
    auth: Option<SpotifyAuth>,
    search: String,
    room: Option<Room>,
    search_result: Option<spotify::SpotifySearchResult>,
}

#[derive(Debug)]
struct UnauthedModel {
    user_id: String,
}

impl Default for UnauthedModel {
    fn default() -> Self {
        UnauthedModel { user_id: "".into() }
    }
}

impl From<spotify::SpotifyImage> for TrackArt {
    fn from(spotify_art: spotify::SpotifyImage) -> Self {
        TrackArt {
            uri: spotify_art.url,
            width: spotify_art.width,
            height: spotify_art.height,
        }
    }
}

impl From<spotify::SpotifyTrack> for Track {
    fn from(spotify_track: spotify::SpotifyTrack) -> Self {
        Track {
            id: spotify_track.id,
            name: spotify_track.name,
            uri: spotify_track.uri,
            duration_ms: spotify_track.duration_ms,
            artwork: spotify_track
                .album
                .images
                .last()
                .map(|x| TrackArt::from(x.to_owned()))
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpotifyAuth {
    access_token: String,
    expires_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    user_id: String,
}

impl AuthedModel {
    fn new(session: Session) -> Self {
        AuthedModel {
            session: session,
            auth: None,
            search: "".into(),
            room: None,
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

    let session = ls
        .get_item("spotijay_session")
        .expect("try to get item from `LocalStorage`");
    let session: Option<Session> = session.map(|x| serde_json::from_str(&x).unwrap());

    let mut data = match session {
        Some(session) => Data::Authed(AuthedModel::new(session)),
        None => Data::UnAuthed(UnauthedModel::default()),
    };

    if is_logging_in(&url) {
        AfterMount::new(Model {
            data: data,
            page: Page::Home,
            connected: false,
            services: Services { ws, ls },
        })
    } else {
        let auth = ls
            .get_item("spotify_auth")
            .expect("try to get item from `LocalStorage`");

        let auth: Option<SpotifyAuth> = auth.map(|x| serde_json::from_str(&x).unwrap());

        if let Data::Authed(authed_model) = &mut data {
            if let Some(auth) = auth {
                if !auth.access_token.is_empty() {
                    authed_model.auth = Some(auth);
                };
            };
        };

        AfterMount::new(Model {
            data: data,
            page: Page::Home,
            connected: false,
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

fn heartbeat_sender(ws: &WebSocket) {
    if ws.ready_state() != WebSocket::OPEN {
        // TODO: Attempt reconnect

        return;
    }

    ws.send_with_str("ping").unwrap();

    let ws = ws.clone();
    spawn_local(async move {
        TimeoutFuture::new(2_000).await;

        heartbeat_sender(&ws);
    });
}

fn is_logging_in(url: &Url) -> bool {
    if let Some(url) = &url.hash {
        if url.contains("access_token") {
            true
        } else {
            false
        }
    } else {
        false
    }
}

#[derive(Debug, Clone)]
enum Msg {
    Authenticate,
    Downvote,
    UsernameChange(String),
    Connected(JsValue),
    ServerMessage(MessageEvent),
    Closed(JsValue),
    Error(JsValue),
    ChangePage(Page),
    AddTrack(String),
    RemoveTrack(String),
    SearchFetched(seed::fetch::ResponseDataResult<spotify::SpotifySearchResult>),
    LoggingInToSpotify,
    LoggedInSpotify,
    SpotifyLogout,
    Queued,
    Played,
    SearchChange(String),
    BecomeDj,
}

async fn get_spotify_search(auth: SpotifyAuth, search: String) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::SpotifyLogout).await
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

async fn put_spotify_play(auth: SpotifyAuth, uri: String, position_ms: u32) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::SpotifyLogout).await
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

async fn post_spotify_next(auth: SpotifyAuth) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::SpotifyLogout).await
    } else {
        Request::new(SPOTIFY_NEXT_URL)
            .header("Authorization", &format!("Bearer {}", auth.access_token))
            .method(seed::browser::service::fetch::Method::Post)
            .fetch(|_| Msg::Played)
            .await
    }
}

async fn post_spotify_queue(auth: SpotifyAuth, uri: String) -> Result<Msg, Msg> {
    if js_sys::Date::now() as u64 > auth.expires_epoch {
        futures::future::ok(Msg::SpotifyLogout).await
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
    match msg.clone() {
        Msg::Connected(_) => {
            model.connected = true;

            match &mut model.data {
                Data::Authed(authed_model) => {
                    authed_update(msg, &model.services, authed_model, &mut model.page, orders)
                }
                Data::UnAuthed(unauthed_model) => {
                    unauthed_update(msg, &model.services, unauthed_model)
                }
            }
        }
        Msg::Closed(_) => {
            model.connected = false;
            log!("WebSocket connection was closed");
        }
        Msg::ChangePage(page) => {
            seed::push_route(page.path());
            model.page = page;
        }
        Msg::ServerMessage(msg_event) => {
            let txt = msg_event.data().as_string().unwrap();
            

            if txt == "pong" {
                return;
            }

            let json: Output = serde_json::from_str(&txt).unwrap();

            match json {
                Output::Authenticated(user_id) => {
                    storage::store_data(
                        &model.services.ls,
                        "spotijay_session",
                        &Session {
                            user_id: user_id.clone(),
                        },
                    );

                    let auth = model
                        .services
                        .ls
                        .get_item("spotify_auth")
                        .expect("try to get item from `LocalStorage`");

                    let auth: Option<SpotifyAuth> = auth.map(|x| serde_json::from_str(&x).unwrap());

                    let auth = if let Some(auth) = auth {
                        if !auth.access_token.is_empty() {
                            Some(auth)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let mut authed_model = AuthedModel::new(Session {
                        user_id: user_id.clone(),
                    });
                    authed_model.auth = auth;

                    model.data = Data::Authed(authed_model);

                    &model
                        .services
                        .ws
                        .send_with_str(
                            &serde_json::to_string(&Input::JoinRoom(User {
                                id: user_id,
                                queue: vec![],
                                last_disconnect: None,
                            }))
                            .unwrap(),
                        )
                        .unwrap();
                }
                _ => match &mut model.data {
                    Data::Authed(authed_model) => {
                        authed_update(msg, &model.services, authed_model, &mut model.page, orders)
                    }
                    Data::UnAuthed(unauthed_model) => {
                        unauthed_update(msg, &model.services, unauthed_model)
                    }
                },
            }
        }
        _ => match &mut model.data {
            Data::Authed(authed_model) => {
                authed_update(msg, &model.services, authed_model, &mut model.page, orders)
            }
            Data::UnAuthed(unauthed_model) => unauthed_update(msg, &model.services, unauthed_model),
        },
    };
}

fn unauthed_update(msg: Msg, services: &Services, unauthed_model: &mut UnauthedModel) {
    match msg {
        Msg::Authenticate => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::Authenticate(unauthed_model.user_id.clone()))
                        .unwrap(),
                )
                .unwrap();
        }
        Msg::UsernameChange(user_id) => {
            unauthed_model.user_id = user_id;
        }
        Msg::Connected(_) => {
            heartbeat_sender(&services.ws);
        }
        Msg::Error(val) => {
            error!("Msg::Error for unauthed user: {:?}", val);
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
    page: &mut Page,
    orders: &mut impl Orders<Msg>,
) {
    match msg {
        Msg::Authenticate => {}
        Msg::UsernameChange(_) => {}
        Msg::Downvote => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::Downvote(authed_model.session.user_id.clone()))
                        .unwrap(),
                )
                .unwrap();
        }
        Msg::LoggingInToSpotify => {
            window().location().set_href(&spotify::login_url()).unwrap();
        }
        Msg::LoggedInSpotify => {
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

            let auth = SpotifyAuth {
                access_token,
                expires_epoch: expires_epoch,
            };
            storage::store_data(&services.ls, "spotify_auth", &auth);
            authed_model.auth = Some(auth.clone());

            seed::push_route(Page::Home.path());
            *page = Page::Home;
        }
        Msg::Queued => {}
        Msg::Played => {}
        Msg::ChangePage(_) => {}
        Msg::SpotifyLogout => {
            window()
                .local_storage()
                .unwrap()
                .unwrap()
                .remove_item("spotify_auth")
                .unwrap();

            authed_model.auth = None;
        }
        Msg::Connected(_) => {
            heartbeat_sender(&services.ws);

            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::Authenticate(
                        authed_model.session.user_id.clone(),
                    ))
                    .unwrap(),
                )
                .unwrap();
        }
        Msg::BecomeDj => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::BecomeDj(authed_model.session.clone().user_id))
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
                        authed_model.session.clone().user_id,
                        Track::from(track),
                    ))
                    .unwrap(),
                )
                .unwrap();

            authed_model.search = "".into();
            authed_model.search_result = None;
        }
        Msg::RemoveTrack(track_id) => {
            services
                .ws
                .send_with_str(
                    &serde_json::to_string(&Input::RemoveTrack(
                        authed_model.session.clone().user_id,
                        track_id,
                    ))
                    .unwrap(),
                )
                .unwrap();

            authed_model.search_result = None;
        }
        Msg::ServerMessage(msg_event) => {
            let txt = msg_event.data().as_string().unwrap();
            let json: Output = serde_json::from_str(&txt).unwrap();

            match json {
                Output::RoomState(room) => {
                    if let None = authed_model.room {
                        if let Some(playing) = &room.playing {
                            if let Some(authed_model) = &authed_model.auth {
                                let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                                orders.perform_cmd(put_spotify_play(
                                    authed_model.clone(),
                                    playing.uri.clone(),
                                    offset_ms as u32,
                                ));
                            }
                        }
                    }

                    authed_model.room = Some(room);
                }
                Output::TrackPlayed(playing) => {
                    if let Some(authed_model) = &authed_model.auth {
                        if let Some(playing) = &playing {
                            let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                            orders.perform_cmd(put_spotify_play(
                                authed_model.clone(),
                                playing.uri.clone(),
                                offset_ms as u32,
                            ));
                        } else {
                            orders.perform_cmd(post_spotify_next(authed_model.clone()));
                        }
                    }

                    if let Some(room) = &mut authed_model.room {
                        room.playing = playing;

                        prune_djs_without_queue(room);
                    }
                }
                Output::NextTrackQueued(track) => {
                    if let Some(authed_model) = &authed_model.auth {
                        orders.perform_cmd(post_spotify_queue(
                            authed_model.clone(),
                            track.uri.clone(),
                        ));
                    }
                }
                Output::Downvoted(user_id) => {
                    if let Some(Room {
                        playing: Some(playing),
                        ..
                    }) = &mut authed_model.room
                    {
                        playing.downvotes.insert(user_id);
                    }
                }
                Output::Authenticated(_) => {}
            }
        }
        Msg::Closed(_) => {}
        Msg::Error(_) => {
            log!("Error");
        }
        Msg::SearchChange(val) => {
            authed_model.search = val.clone();

            if !val.is_empty() {
                if let Some(authed_model) = &authed_model.auth {
                    orders.perform_cmd(get_spotify_search(authed_model.clone(), val.clone()));
                }
            } else {
                authed_model.search_result = None;
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

    Some(ul![class!["users-list"], items])
}

fn playlist_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    if authed_model.auth.is_none() {
        return Some(div![
            id!["playlist"],
            h3!["Your playlist"],
            spotify_login_button()
        ]);
    }

    let mut items: Vec<Node<Msg>> = Vec::new();

    let tracks = current_user(&authed_model.session.user_id, authed_model.room.as_ref()?)?
        .queue
        .into_iter();

    for track in tracks {
        let event_track = track.clone();

        items.push(li![button![
            class!["list-button"],
            ev(Ev::Click, move |_| Msg::RemoveTrack(event_track.id)),
            track.name
        ]]);
    }

    Some(div![
        id!["playlist"],
        h3!["Your playlist"],
        input![
            attrs! {At::Value => authed_model.search},
            attrs! {At::Placeholder => "Search for tracks"},
            attrs! {At::Type => "search"},
            input_ev(Ev::Input, Msg::SearchChange)
        ],
        search_result_view(authed_model).unwrap_or(ol![class!("playlist-list"), items]),
    ])
}

fn search_result_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let tracks = authed_model.search_result.clone()?.tracks.items.into_iter();

    for track in tracks {
        let track_id = track.id.clone();
        let click_event = ev(Ev::Click, move |_| Msg::AddTrack(track_id));
        let maybe_track_image: Option<Node<Msg>> = track.album.images.last().map(|spotify_image| {
            img![class!("track-art"),attrs![At::Src => spotify_image.url, At::Width => spotify_image.width,At::Height => spotify_image.height]]
        });
        let track_title = h5![class!["track-title"], track.name];

        let track_artist = h6![
            class!["track-artist"],
            track
                .artists
                .iter()
                .map(|x| x.name.to_owned())
                .collect::<Vec<String>>()
                .join(", ")
        ];

        items.push(li![button![
            class!["track-card"],
            click_event,
            maybe_track_image.unwrap_or(Node::Empty),
            track_title,
            track_artist,
        ]]);
    }

    Some(ul![class!["search-results-list"], items])
}

fn djs_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let djs = authed_model.room.clone().and_then(|x| x.djs);

    if let Some(djs) = djs {
        let playing = authed_model
            .room
            .as_ref()
            .and_then(|x| x.playing.as_ref().map(|x| x.name.clone()))
            .unwrap_or("Nothing!".to_string());

        for dj in djs.iter() {
            items.push(li![
                class!["list-button"],
                if dj.id == djs.current.id {
                    let downvoted = authed_model.room.as_ref().and_then(|x| {
                        x.playing
                            .as_ref()
                            .map(|y| y.downvotes.contains(&authed_model.session.user_id))
                    }).unwrap_or(false);

                    vec![
                        div![b![dj.id], format!(" - {}", playing)],
                        button![
                            simple_ev(Ev::Click, Msg::Downvote),
                            class!["list-button-downvote", "list-button-downvote-downvoted" => downvoted],
                            "👎"
                        ],
                    ]
                } else {
                    vec![div![dj.id]]
                }
            ]);
        }

        if djs.iter().any(|x| x.id != authed_model.session.user_id) {
            items.push(li![
                class!["list-button"],
                simple_ev(Ev::Click, Msg::BecomeDj),
                div!["Become a DJ"]
            ]);
        }
    } else {
        items.push(li![button![
            class!["list-button"],
            simple_ev(Ev::Click, Msg::BecomeDj),
            div!["Become a DJ"]
        ]]);
    }

    Some(div![id!["djs"], h3!["DJs"], ol![class!("dj-list"), items]])
}

fn spotify_login_button() -> Node<Msg> {
    div![button![
        simple_ev(Ev::Click, Msg::LoggingInToSpotify),
        "Log in to Spotify to listen and build a playlist"
    ]]
}

fn authed_view(authed_model: &AuthedModel) -> Node<Msg> {
    div![
        id!["app-inner"],
        div![
            id!["content"],
            div![
                id!["listeners"],
                h3!["Listeners"],
                users_view(authed_model).unwrap_or(div!["No listeners"]),
            ],
            playlist_view(authed_model).unwrap_or(div![]),
            djs_view(authed_model).unwrap_or(div![]),
        ]
    ]
}

fn unauthed_view(_unauthed_model: &UnauthedModel) -> Node<Msg> {
    div![
        id!["app-inner"],
        div![
            id!["login"],
            input![
                attrs! {At::Placeholder => "Username"},
                input_ev(Ev::Input, Msg::UsernameChange)
            ],
            button![simple_ev(Ev::Click, Msg::Authenticate), "Log in"],
        ]
    ]
}

fn data_view(data: &Data) -> Node<Msg> {
    match &data {
        Data::Authed(authed) => authed_view(authed),
        Data::UnAuthed(unauthed_model) => unauthed_view(unauthed_model),
    }
}

fn global_loading_view(loading: bool) -> Node<Msg> {
    if loading {
        div![id!["global-loading"], h2!["Connecting"]]
    } else {
        empty![]
    }
}

fn view(model: &Model) -> impl View<Msg> {
    let data = &model.data;

    vec![
        h1!["Spotijay"],
        data_view(data),
        global_loading_view(!model.connected),
    ]
}

fn routes(url: Url) -> Option<Msg> {
    if is_logging_in(&url) {
        Some(Msg::LoggedInSpotify)
    } else {
        Page::from_url(url).map(Msg::ChangePage)
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
