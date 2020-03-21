use warp::Rejection;
use warp::Reply;
use warp::{Filter};
use warp::http::{header, StatusCode, Response};

use serde::Deserialize;

use rspotify::client::Spotify;
use rspotify::oauth2::{SpotifyClientCredentials, SpotifyOAuth};

fn spot_auth_url() -> String {
    let oauth = SpotifyOAuth::default()
        .scope("user-modify-playback-state")
        .build();

    oauth.get_authorize_url(None, None)
}

async fn spotify_callback(code: String) -> Result<impl Reply, Rejection> {
    let oauth = SpotifyOAuth::default();
    match oauth.get_access_token(&code).await {
        Some(token) => {
            let client_credential = SpotifyClientCredentials::default()
                .token_info(token)
                .build();
            let artists = Spotify::default()
                .client_credentials_manager(client_credential)
                .search_artist("fleet", 3, None, None)
                .await;

            println!("artists: {:?}", artists);

            Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, "/")
                .header(
                    header::SET_COOKIE,
                    format!("SPOTIFYTOKEN={}; SameSite=Strict; HttpOpnly", "test"),
                )
                .body(b"".to_vec())
                .map_err(|_| warp::reject::not_found())
        }
        None => Err(warp::reject::not_found()),
    }
}

async fn spotify_start(
    songs: Vec<String>,
    position_ms: Option<u32>,
) -> Result<String, warp::Rejection> {
    Spotify::default()
        .start_playback(None, None, Some(songs), None, position_ms)
        .await;

    Ok("Ok".into())
}

#[derive(Deserialize)]
struct SpotifyCallbackParams {
    code: String,
    state: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    // Hello world
    let hello = warp::path!("rooms").map(|| format!("Hello world!"));

    // Web client
    let auth = warp::path("auth")
        .map(|| warp::redirect(warp::http::Uri::from_maybe_shared(spot_auth_url()).unwrap()));
    let callback = warp::path("callback")
        .and(warp::query::<SpotifyCallbackParams>())
        .and_then(|params: SpotifyCallbackParams| spotify_callback(params.code));
    let play =
        warp::path!("play" / String / u32).and_then(|uri, pos| spotify_start(vec![uri], Some(pos)));

    // Start server
    warp::serve(hello.or(auth).or(callback).or(play))
        .run(([127, 0, 0, 1], 8888))
        .await;
}
