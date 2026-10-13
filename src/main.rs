//! Multiplayer Petrov Day.
//!
//! Two groups, one missile each, a warning window, and false alarms that
//! look exactly like real launches. The room machinery — the keys, the
//! event streams, the store, the QR codes — is [`rooms`]; what is here is
//! the ritual itself, the brands it is served under, and the list of
//! groups playing it this year.

mod brand;
mod game;
mod groups;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use brand::Brand;
use game::{Access, Game, Settings};
use groups::{Group, GroupInput};
use rooms::{Rooms, http, now_ms, qr};
use serde::{Deserialize, de::DeserializeOwned};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const INDEX: &str = include_str!("../static/index.html");
const MANIFEST: &str = include_str!("../static/manifest.webmanifest");
const ICON: &str = include_str!("../static/icon.svg");
/// The page's own half, compiled from the TypeScript in `client/` by the
/// build script. Nothing here is written in JavaScript. The look stays in
/// the page itself: this one is a dark console, not a room's panels.
const APP_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/app.js"));

const MAX_GAMES: usize = 2000;
/// How often to look for games nobody has opened in a day.
const SWEEP: Duration = Duration::from_secs(5 * 60);

struct App {
    brand: &'static Brand,
    index: &'static str,
    manifest: &'static str,
    games: Arc<Rooms<Game>>,
    /// The groups playing this year. Not rooms: one list, with a password
    /// per entry, thrown away on the day.
    groups: Mutex<Vec<Group>>,
    groups_path: PathBuf,
}

type Shared = Arc<App>;

fn no_such_game() -> Response {
    http::error(
        StatusCode::NOT_FOUND,
        "This link does not belong to any game.",
    )
}

fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) {
    let json = match serde_json::to_vec(value) {
        Ok(json) => json,
        Err(e) => return eprintln!("failed to serialize {}: {e}", path.display()),
    };
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path)) {
        eprintln!("failed to write {}: {e}", path.display());
    }
}

fn read_json<T: DeserializeOwned + Default>(path: &std::path::Path) -> T {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e) => {
                let backup = path.with_extension("unreadable.json");
                eprintln!(
                    "cannot parse {}: {e}; moving it to {}",
                    path.display(),
                    backup.display()
                );
                std::fs::rename(path, &backup).expect("cannot move unreadable state aside");
                T::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => panic!("cannot read {}: {e}", path.display()),
    }
}

/// Which game a country key belongs to, and what it is allowed to do.
async fn find_country(app: &App, key: &str) -> Option<(String, usize, Access)> {
    let key = key.to_string();
    app.games
        .find(move |game_key, game| {
            game.country_for(&key)
                .map(|(country, access)| (game_key.to_string(), country, access))
        })
        .await
}

/// Runs `f` against a game, after telling it the time and letting it move
/// on. Saves afterwards.
async fn with_game<R>(app: &App, key: &str, f: impl FnOnce(&mut Game, u64) -> R) -> Option<R> {
    app.games.with(key, f).await
}

async fn with_country<R>(
    app: &App,
    key: &str,
    f: impl FnOnce(&mut Game, usize, Access, u64) -> R,
) -> Option<R> {
    let (game_key, country, access) = find_country(app, key).await?;
    with_game(app, &game_key, |game, now| f(game, country, access, now)).await
}

async fn create_game(State(app): State<Shared>) -> Response {
    let now = now_ms();
    let made = app
        .games
        .create(|key| {
            let mut game = Game::new(now, &mut rand::rng());
            // The store files a game under a key of its own; the game
            // carries the same one so that everything that looks at a
            // game can say where it lives.
            game.key = key.to_string();
            game
        })
        .await;
    match made {
        Some(key) => Json(serde_json::json!({ "game": key })).into_response(),
        None => http::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many games are running.",
        ),
    }
}

async fn game_state(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    match with_game(&app, &key, |game, now| {
        http::envelope(now, &game.game_view())
    })
    .await
    {
        Some(body) => http::json(body),
        None => no_such_game(),
    }
}

async fn update_settings(
    State(app): State<Shared>,
    Path(key): Path<String>,
    Json(settings): Json<Settings>,
) -> Response {
    let done = with_game(&app, &key, |game, now| {
        game.update_settings(settings)
            .map(|_changed| http::envelope(now, &game.game_view()))
    })
    .await;
    match done {
        None => no_such_game(),
        Some(Err(message)) => http::error(StatusCode::CONFLICT, message),
        Some(Ok(body)) => http::json(body),
    }
}

async fn claim(State(app): State<Shared>, Path((key, country)): Path<(String, usize)>) -> Response {
    if country > 1 {
        return no_such_game();
    }
    let done = with_game(&app, &key, |game, _| game.claim(country, &mut rand::rng())).await;
    match done {
        None => no_such_game(),
        Some(Err(message)) => http::error(StatusCode::GONE, message),
        Some(Ok(key)) => Json(serde_json::json!({ "country": key })).into_response(),
    }
}

async fn country_state(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    match with_country(&app, &key, |game, country, access, now| {
        http::envelope(now, &game.country_view(country, access, now))
    })
    .await
    {
        Some(body) => http::json(body),
        None => no_such_game(),
    }
}

/// Something one country does, which only a command link may do.
async fn country_action(
    app: Shared,
    key: String,
    action: impl FnOnce(&mut Game, usize, u64) -> Result<(), &'static str>,
) -> Response {
    let done = with_country(&app, &key, |game, country, access, now| {
        if access == Access::Watch {
            return Err((StatusCode::FORBIDDEN, "This link can only watch."));
        }
        action(game, country, now)
            .map(|()| http::envelope(now, &game.country_view(country, access, now)))
            .map_err(|message| (StatusCode::CONFLICT, message))
    })
    .await;
    match done {
        None => no_such_game(),
        Some(Err((status, message))) => http::error(status, message),
        Some(Ok(body)) => http::json(body),
    }
}

async fn start(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |game, country, now| {
        game.start(country, now, &mut rand::rng())
    })
    .await
}

async fn launch(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |game, country, now| {
        game.launch(country, now, &mut rand::rng())
    })
    .await
}

async fn request_end(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |game, country, now| {
        game.request_end(country, now, &mut rand::rng())
    })
    .await
}

async fn game_events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    rooms::one(app.games.clone(), key, |game| {
        serde_json::to_string(&game.game_view()).ok()
    })
}

async fn country_events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    let Some((game_key, country, access)) = find_country(&app, &key).await else {
        return no_such_game();
    };
    let games = app.games.clone();
    rooms::stream(games.clone(), move |held, now| {
        let game = held.get_mut(&game_key)?;
        game.last_seen = now;
        if game.advance(now, &mut rand::rng()) {
            let view = serde_json::to_string(&game.country_view(country, access, now)).ok();
            games.save(held);
            return view;
        }
        serde_json::to_string(&game.country_view(country, access, now)).ok()
    })
}

async fn qr_code(body: Bytes) -> Response {
    match std::str::from_utf8(&body) {
        Ok(link) => qr::response(link),
        Err(_) => http::error(StatusCode::BAD_REQUEST, "That is not a link."),
    }
}

/// The server's clock, which the preflight check compares its own against.
async fn time() -> Response {
    http::json(format!("{{\"now\":{}}}", now_ms()))
}

async fn list_groups(State(app): State<Shared>) -> Response {
    let groups = app.groups.lock().await;
    let reset = groups::last_reset(now_ms(), app.brand.month, app.brand.day_of_month);
    let mut public: Vec<_> = groups
        .iter()
        .filter(|group| group.created >= reset)
        .map(Group::public)
        .collect();
    public.sort_by_key(|group| std::cmp::Reverse(group.updated));
    Json(public).into_response()
}

async fn create_group(State(app): State<Shared>, Json(input): Json<GroupInput>) -> Response {
    if let Err(message) = input.validate() {
        return http::error(StatusCode::BAD_REQUEST, &message);
    }
    if app.groups.lock().await.len() >= groups::MAX_GROUPS {
        return http::error(StatusCode::SERVICE_UNAVAILABLE, "The list is full.");
    }
    let password = input.password.clone();
    let hash = tokio::task::spawn_blocking(move || groups::hash_password(&password))
        .await
        .expect("hashing task");
    let mut groups = app.groups.lock().await;
    if groups.len() >= groups::MAX_GROUPS {
        return http::error(StatusCode::SERVICE_UNAVAILABLE, "The list is full.");
    }
    let group = Group::new(&input, hash, now_ms(), &mut rand::rng());
    let public = group.public();
    groups.push(group);
    write_json(&app.groups_path, &*groups);
    Json(public).into_response()
}

async fn authorize(
    app: &App,
    id: &str,
    password: String,
) -> Result<(), (StatusCode, &'static str)> {
    let hash = app
        .groups
        .lock()
        .await
        .iter()
        .find(|group| group.id == id)
        .map(|group| group.password_hash.clone())
        .ok_or((StatusCode::NOT_FOUND, "This group no longer exists."))?;
    let ok = tokio::task::spawn_blocking(move || groups::verify_password(&password, &hash))
        .await
        .expect("verification task");
    if ok {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Wrong password."))
    }
}

async fn update_group(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(input): Json<GroupInput>,
) -> Response {
    if let Err(message) = input.validate() {
        return http::error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err((status, message)) = authorize(&app, &id, input.password.clone()).await {
        return http::error(status, message);
    }
    let mut groups = app.groups.lock().await;
    let Some(group) = groups.iter_mut().find(|group| group.id == id) else {
        return http::error(StatusCode::NOT_FOUND, "This group no longer exists.");
    };
    group.update(&input, now_ms());
    let public = group.public();
    write_json(&app.groups_path, &*groups);
    Json(public).into_response()
}

#[derive(Deserialize)]
struct PasswordOnly {
    password: String,
}

async fn delete_group(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<PasswordOnly>,
) -> Response {
    if let Err((status, message)) = authorize(&app, &id, body.password).await {
        return http::error(status, message);
    }
    let mut groups = app.groups.lock().await;
    groups.retain(|group| group.id != id);
    write_json(&app.groups_path, &*groups);
    StatusCode::NO_CONTENT.into_response()
}

async fn index(State(app): State<Shared>) -> Html<&'static str> {
    Html(app.index)
}

async fn manifest(State(app): State<Shared>) -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        app.manifest,
    )
        .into_response()
}

async fn script() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_JS,
    )
        .into_response()
}

/// The groups list is emptied every year on the day itself. The games
/// look after themselves; see `Room for Game`.
async fn forget_old_groups(app: Shared) {
    loop {
        tokio::time::sleep(SWEEP).await;
        let now = now_ms();
        let mut groups = app.groups.lock().await;
        let before = groups.len();
        let reset = groups::last_reset(now, app.brand.month, app.brand.day_of_month);
        groups.retain(|group| group.created >= reset);
        if groups.len() != before {
            write_json(&app.groups_path, &*groups);
        }
    }
}

fn router(app: Shared) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/check", get(index))
        .route("/groups", get(index))
        .route("/info", get(index))
        .route("/g/{key}", get(index))
        .route("/k/{key}", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route(
            "/icon.svg",
            get(|| async { ([(header::CONTENT_TYPE, "image/svg+xml")], ICON) }),
        )
        .route("/app.js", get(script))
        .route("/api/time", get(time))
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/{id}", put(update_group).delete(delete_group))
        .route("/api/qr", post(qr_code))
        .route("/api/games", post(create_game))
        .route("/api/g/{key}", get(game_state))
        .route("/api/g/{key}/events", get(game_events))
        .route("/api/g/{key}/settings", put(update_settings))
        .route("/api/g/{key}/claim/{country}", post(claim))
        .route("/api/k/{key}", get(country_state))
        .route("/api/k/{key}/events", get(country_events))
        .route("/api/k/{key}/start", post(start))
        .route("/api/k/{key}/launch", post(launch))
        .route("/api/k/{key}/end", post(request_end))
        .with_state(app)
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("PETROV_ADDR").unwrap_or_else(|_| "127.0.0.1:8095".into());
    let state_path =
        PathBuf::from(std::env::var("PETROV_STATE").unwrap_or_else(|_| "games.json".into()));
    let groups_path = state_path.with_file_name("groups.json");

    let games: Arc<Rooms<Game>> = Rooms::load(&state_path, MAX_GAMES);
    games
        .each(|game| game.backfill_watch_keys(&mut rand::rng()))
        .await;
    let groups: Vec<Group> = read_json(&groups_path);

    let brand_name = std::env::var("PETROV_BRAND").unwrap_or_else(|_| "petrov".into());
    let brand =
        Brand::find(&brand_name).unwrap_or_else(|| panic!("unknown PETROV_BRAND {brand_name}"));

    let app = Arc::new(App {
        brand,
        index: Box::leak(brand.render(INDEX).into_boxed_str()),
        manifest: Box::leak(brand.render(MANIFEST).into_boxed_str()),
        games: games.clone(),
        groups: Mutex::new(groups),
        groups_path,
    });
    tokio::spawn(games.sweep(SWEEP));
    tokio::spawn(forget_old_groups(app.clone()));

    // Under systemd the port is held by a socket unit and passed in, so
    // the service itself runs with no network of its own.
    let listener = rooms::listen(&addr).await;
    println!("{}", rooms::where_it_listens(&listener));
    axum::serve(listener, router(app))
        .await
        .expect("server error");
}
