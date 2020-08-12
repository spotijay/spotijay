use gloo::timers::future::TimeoutFuture;
use seed::{prelude::*, *};
use serde::{Deserialize, Serialize};

use shared::lib::{prune_djs_without_queue, Input, Output, Room, Track, TrackArt, User};

mod pages;
mod spotify;
use pages::Page;
use std::sync::Arc;
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
    ws: Arc<WebSocket>,
}

fn before_mount(_: Url) -> BeforeMount {
    // Since we have the "loading..." text in the app section of index.html,
    // we use MountType::Takover which will overwrite it with the seed generated html
    BeforeMount::new().mount_type(MountType::Takeover)
}

fn after_mount(url: Url, orders: &mut impl Orders<Msg>) -> AfterMount<Model> {
    let ws = WebSocket::builder(WS_URL, orders)
        .on_open(|| Msg::WebSocketConnected)
        .on_close(|x| Msg::Closed(x))
        .on_message(Msg::ServerMessage)
        .on_error(|| Msg::Error)
        .build_and_open()
        .expect("Should be able to create a new webscocket to WS_URL");

    let session: Option<Session> = LocalStorage::get("spotijay_session").ok();

    let mut data = match session {
        Some(session) => Data::Authed(AuthedModel::new(session)),
        None => Data::UnAuthed(UnauthedModel::default()),
    };

    if is_logging_in(&url) {
        AfterMount::new(Model {
            data: data,
            page: Page::Home,
            connected: false,
            services: Services { ws: Arc::new(ws) },
        })
    } else {
        let auth: Option<SpotifyAuth> = LocalStorage::get("spotify_auth").ok();

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
            services: Services { ws: Arc::new(ws) },
        })
    }
}

fn heartbeat_sender(ws: Arc<WebSocket>) {
    if ws.state() != seed::browser::web_socket::State::Open {
        // TODO: Attempt reconnect
        return;
    }
    spawn_local(async move {
        loop {
            ws.send_text("ping").unwrap();
            TimeoutFuture::new(2_000).await;
        }
    });
}

fn is_logging_in(url: &Url) -> bool {
    if let Some(hash) = &url.hash() {
        hash.contains("access_token")
    } else {
        false
    }
}

fn has_login_expired(expires_epoch: &u64) -> bool {
    js_sys::Date::now() as u64 > *expires_epoch
}

#[derive(Debug)]
enum AuthenticatedMsg {
    Downvote,
    LoggedInSpotify(SpotifyAuth),
    SpotifyLogout,
    WebSocketConnected,
    BecomeDj,
    AddTrack(String),
    RemoveTrack(String),
    ServerMessage(Output),
    SearchChange(String),
    SearchFetched(spotify::SpotifySearchResult),
}

#[derive(Debug)]
enum Msg {
    Authenticate,
    LoggingInToSpotify,
    LoggedInSpotify,
    Authenticated(AuthenticatedMsg),
    UsernameChange(String),
    WebSocketConnected,
    ServerMessage(WebSocketMessage),
    Closed(CloseEvent),
    Error,
    ChangePage(Page),
    Queued,
    Played,
    LogError(String),
}

async fn get_spotify_search(auth: SpotifyAuth, search: String) -> Result<Msg, Msg> {
    if has_login_expired(&auth.expires_epoch) {
        futures::future::ok(Msg::Authenticated(AuthenticatedMsg::SpotifyLogout)).await
    } else {
        let url = url::Url::parse_with_params(
            SPOTIFY_SEARCH_URL,
            &[("q", search), ("type", "track".into())],
        )
        .unwrap()
        .into_string();

        Request::new(url)
            .header(Header::bearer(&auth.access_token))
            .fetch()
            .map_err(|e| Msg::LogError(format!("{:?}", e)))
            .await?
            .json::<spotify::SpotifySearchResult>()
            .await
            .map(|x| Msg::Authenticated(AuthenticatedMsg::SearchFetched(x)))
            .map_err(|e| Msg::LogError(format!("{:?}", e)))
    }
}

async fn put_spotify_play(auth: SpotifyAuth, uri: String, position_ms: u32) -> Result<Msg, Msg> {
    if has_login_expired(&auth.expires_epoch) {
        futures::future::ok(Msg::Authenticated(AuthenticatedMsg::SpotifyLogout)).await
    } else {
        Request::new(SPOTIFY_PLAY_URL)
            .header(Header::bearer(auth.access_token))
            .method(Method::Put)
            .json(&spotify::PlayRequest {
                uris: vec![uri],
                position_ms,
            })
            .map_err(|e| Msg::LogError(format!("{:?}", e)))?
            .fetch()
            .await
            .map(|_| Msg::Played)
            .map_err(|e| Msg::LogError(format!("{:?}", e)))
    }
}

async fn post_spotify_next(auth: SpotifyAuth) -> Result<Msg, Msg> {
    if has_login_expired(&auth.expires_epoch) {
        futures::future::ok(Msg::Authenticated(AuthenticatedMsg::SpotifyLogout)).await
    } else {
        let response_result = Request::new(SPOTIFY_NEXT_URL)
            .header(Header::bearer(auth.access_token))
            .method(Method::Post)
            .fetch();

        match response_result.await {
            Ok(_response) => Ok(Msg::Played),
            Err(_error) => Err(Msg::LogError(
                "failed to make spotify play next track".to_string(),
            )),
        }
    }
}

async fn post_spotify_queue(auth: SpotifyAuth, uri: String) -> Result<Msg, Msg> {
    if has_login_expired(&auth.expires_epoch) {
        futures::future::ok(Msg::Authenticated(AuthenticatedMsg::SpotifyLogout)).await
    } else {
        let url = url::Url::parse_with_params(SPOTIFY_QUEUE_URL, &[("uri", uri)])
            .unwrap()
            .into_string();

        Request::new(url)
            .header(Header::bearer(auth.access_token))
            .method(Method::Post)
            .fetch()
            .await
            .map(|_| Msg::Queued)
            .map_err(|e| Msg::LogError(format!("{:?}", e)))
    }
}

fn unwrap_msg(result: Result<Msg,Msg>) -> Msg {
    match result {
        Ok(msg) => msg,
        Err(msg) => msg
    }
}

fn update(msg: Msg, model: &mut Model, orders: &mut impl Orders<Msg>) {
    match msg {
        Msg::WebSocketConnected => {
            model.connected = true;
            heartbeat_sender(model.services.ws.clone());

            match &mut model.data {
                Data::Authed(authed_model) => authed_update(
                    AuthenticatedMsg::WebSocketConnected,
                    &model.services,
                    authed_model,
                    &mut model.page,
                    orders,
                ),
                Data::UnAuthed(_) => (),
            }
        }
        Msg::Closed(error) => {
            model.connected = false;
            error!("WebSocket connection was closed", error);
        }
        Msg::ChangePage(page) => {
            seed::push_route(page.clone());
            model.page = page;
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

            LocalStorage::insert("spotify_auth", &auth).ok();

            match &mut model.data {
                Data::Authed(auth_model) => authed_update(
                    AuthenticatedMsg::LoggedInSpotify(auth),
                    &model.services,
                    auth_model,
                    &mut model.page,
                    orders,
                ),
                Data::UnAuthed(_) => (),
            }
        }
        Msg::ServerMessage(msg_event) => {
            let txt = msg_event.text().unwrap();

            if txt == "pong" {
                return;
            }

            let output: Output = serde_json::from_str(&txt).unwrap();

            match output {
                Output::Authenticated(user_id) => {
                    LocalStorage::insert(
                        "spotijay_session",
                        &Session {
                            user_id: user_id.clone(),
                        },
                    )
                    .ok();

                    let auth = LocalStorage::get("spotify_auth").ok().and_then(
                        |spotify_auth: SpotifyAuth| {
                            if !spotify_auth.access_token.is_empty() {
                                Some(spotify_auth)
                            } else {
                                None
                            }
                        },
                    );

                    let mut authed_model = AuthedModel::new(Session {
                        user_id: user_id.clone(),
                    });
                    authed_model.auth = auth;

                    model.data = Data::Authed(authed_model);

                    &model
                        .services
                        .ws
                        .send_json(&Input::JoinRoom(User {
                            id: user_id,
                            queue: vec![],
                            last_disconnect: None,
                        }))
                        .unwrap();
                }
                _ => match &mut model.data {
                    Data::UnAuthed(_) => (),
                    Data::Authed(authed_model) => {
                        let sub_msg = AuthenticatedMsg::ServerMessage(output);
                        authed_update(
                            sub_msg,
                            &model.services,
                            authed_model,
                            &mut model.page,
                            orders,
                        )
                    }
                },
            }
        }
        Msg::Authenticated(sub_msg) => match &mut model.data {
            Data::Authed(authed_model) => authed_update(
                sub_msg,
                &model.services,
                authed_model,
                &mut model.page,
                orders,
            ),
            Data::UnAuthed(_) => (),
        },
        _ => match &mut model.data {
            Data::Authed(_) => (),
            Data::UnAuthed(unauthed_model) => unauthed_update(msg, &model.services, unauthed_model),
        },
    };
}

fn unauthed_update(msg: Msg, services: &Services, unauthed_model: &mut UnauthedModel) {
    match msg {
        Msg::Authenticate => {
            services
                .ws
                .send_json(&Input::Authenticate(unauthed_model.user_id.clone()))
                .unwrap();
        }
        Msg::UsernameChange(user_id) => {
            unauthed_model.user_id = user_id.clone();
        }
        Msg::Error => {
            error!("WebSocket error for unauthed user");
        }
        _ => {
            error!("Invalid Msg for unauthed user.");
        }
    }
}

fn authed_update(
    msg: AuthenticatedMsg,
    services: &Services,
    authed_model: &mut AuthedModel,
    page: &mut Page,
    orders: &mut impl Orders<Msg>,
) {
    match msg {
        AuthenticatedMsg::Downvote => {
            services
                .ws
                .send_json(&Input::Downvote(authed_model.session.user_id.clone()))
                .unwrap();
        }
        AuthenticatedMsg::LoggedInSpotify(spotify_auth) => {
            authed_model.auth = Some(spotify_auth);

            seed::push_route(Page::Home);
            *page = Page::Home;
        }
        AuthenticatedMsg::SpotifyLogout => {
            LocalStorage::remove("spotify_auth").unwrap();
            authed_model.auth = None;
        }
        AuthenticatedMsg::WebSocketConnected => {
            services
                .ws
                .send_json(&Input::Authenticate(authed_model.session.user_id.clone()))
                .unwrap();
        }
        AuthenticatedMsg::BecomeDj => {
            services
                .ws
                .send_json(&Input::BecomeDj(authed_model.session.clone().user_id))
                .unwrap();
        }
        AuthenticatedMsg::AddTrack(track_id) => {
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
                .send_json(&Input::AddTrack(
                    authed_model.session.clone().user_id,
                    Track::from(track),
                ))
                .unwrap();

            authed_model.search = "".into();
            authed_model.search_result = None;
        }
        AuthenticatedMsg::RemoveTrack(track_id) => {
            match services.ws.send_json(&Input::RemoveTrack(
                authed_model.session.clone().user_id,
                track_id,
            )) {
                Ok(()) => authed_model.search_result = None,
                Err(ws_error) => error!("AuthenticatedMsg::RemoveTrack",ws_error),
            }
        }
        AuthenticatedMsg::ServerMessage(output) => match output {
            Output::RoomState(room) => {
                if let None = authed_model.room {
                    if let Some(playing) = &room.playing {
                        if let Some(authed_model) = &authed_model.auth {
                            let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                            let cloned_authed_model = authed_model.clone();
                            let playing_uri = playing.uri.clone();
                            orders.perform_cmd(async move { 
                                unwrap_msg(put_spotify_play(
                                cloned_authed_model,
                                playing_uri,
                                offset_ms as u32,
                                ).await)
                            });
                        }
                    }
                }

                authed_model.room = Some(room);
            }
            Output::TrackPlayed(playing) => {
                if let Some(authed_model) = &authed_model.auth {
                    if let Some(playing) = &playing {
                        let offset_ms = (js_sys::Date::now() as u64) - playing.started;
                        let cloned_authed_model = authed_model.clone();
                        let playing_uri = playing.uri.clone();
                        orders.perform_cmd(async move {
                            unwrap_msg(put_spotify_play(
                                cloned_authed_model,
                                playing_uri,
                                offset_ms as u32,
                            ).await)
                        });
                    } else {
                        let cloned_authed_model = authed_model.clone();
                        orders.perform_cmd(async {
                            unwrap_msg(post_spotify_next(cloned_authed_model).await)
                        });
                    }
                }

                if let Some(room) = &mut authed_model.room {
                    room.playing = playing;

                    prune_djs_without_queue(room);
                }
            }
            Output::NextTrackQueued(track) => {
                if let Some(authed_model) = &authed_model.auth {
                    let cloned_authed_model = authed_model.clone();
                    let track_uri = track.uri.clone();
                    orders.perform_cmd(async {
                        unwrap_msg(post_spotify_queue(cloned_authed_model, track_uri).await)
                    });
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
            Output::Authenticated(_) => (),
        },
        AuthenticatedMsg::SearchChange(val) => {
            authed_model.search = val.clone();

            if !val.is_empty() {
                if let Some(authed_model) = &authed_model.auth {
                    let cloned_authed_model = authed_model.clone();
                    let cloned_val = val.clone();
                    orders.perform_cmd(async {
                        unwrap_msg(get_spotify_search(cloned_authed_model, cloned_val).await)
                    });
                }
            } else {
                authed_model.search_result = None;
            }
        }
        AuthenticatedMsg::SearchFetched(search_results) => {
            authed_model.search_result = Some(search_results)
        }
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
    let items = room.users.iter().map(|x| li![&x.id]);

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
            ev(Ev::Click, move |_| Msg::Authenticated(
                AuthenticatedMsg::RemoveTrack(event_track.id)
            )),
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
            input_ev(Ev::Input, |x| Msg::Authenticated(
                AuthenticatedMsg::SearchChange(x)
            ))
        ],
        search_result_view(authed_model).unwrap_or(ol![class!("playlist-list"), items]),
    ])
}

fn search_result_view(authed_model: &AuthedModel) -> Option<Node<Msg>> {
    let mut items: Vec<Node<Msg>> = Vec::new();

    let tracks = authed_model.search_result.clone()?.tracks.items.into_iter();

    for track in tracks {
        let track_id = track.id.clone();
        let click_event = ev(Ev::Click, move |_| {
            Msg::Authenticated(AuthenticatedMsg::AddTrack(track_id))
        });
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
                        div![b![&dj.id], format!(" - {}", playing)],
                        button![
                            ev(Ev::Click, |_| Msg::Authenticated(AuthenticatedMsg::Downvote)),
                            class!["list-button-downvote", "list-button-downvote-downvoted" => downvoted],
                            "👎"
                        ],
                    ]
                } else {
                    vec![div![&dj.id]]
                }
            ]);
        }

        if djs.iter().any(|x| x.id != authed_model.session.user_id) {
            items.push(li![
                class!["list-button"],
                ev(Ev::Click, |_| Msg::Authenticated(
                    AuthenticatedMsg::BecomeDj
                )),
                div!["Become a DJ"]
            ]);
        }
    } else {
        items.push(li![button![
            class!["list-button"],
            ev(Ev::Click, |_| Msg::Authenticated(
                AuthenticatedMsg::BecomeDj
            )),
            div!["Become a DJ"]
        ]]);
    }

    Some(div![id!["djs"], h3!["DJs"], ol![class!("dj-list"), items]])
}

fn spotify_login_button() -> Node<Msg> {
    div![button![
        ev(Ev::Click, |_| Msg::LoggingInToSpotify),
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
            button![ev(Ev::Click, |_| Msg::Authenticate), "Log in"],
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

fn view(model: &Model) -> impl IntoNodes<Msg> {
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
    console_error_panic_hook::set_once();
    App::builder(update, view)
        .before_mount(before_mount)
        .after_mount(after_mount)
        .routes(routes)
        .build_and_start();
}
