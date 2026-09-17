mod game;
mod groups;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use game::{Game, Settings};
use groups::{Group, GroupInput};
use qrcode::{QrCode, render::svg};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, MutexGuard, mpsc};
use tokio_stream::wrappers::ReceiverStream;

const INDEX: &str = include_str!("../static/index.html");
const MANIFEST: &str = include_str!("../static/manifest.webmanifest");
const ICON: &str = include_str!("../static/icon.svg");
const TICK_MS: u64 = 250;
const HEARTBEAT_MS: u64 = 5_000;
const HOUR_MS: u64 = 60 * 60 * 1000;
const DAY_MS: u64 = 24 * HOUR_MS;
const MAX_GAMES: usize = 2000;

struct App {
    games: Mutex<HashMap<String, Game>>,
    state_path: PathBuf,
    groups: Mutex<Vec<Group>>,
    groups_path: PathBuf,
}

type Shared = Arc<App>;
type Games<'a> = MutexGuard<'a, HashMap<String, Game>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_millis() as u64
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, msg.to_string()).into_response()
}

fn not_found() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "This link does not belong to any game.",
    )
}

fn envelope<T: Serialize>(now: u64, view: &T) -> String {
    let view = serde_json::to_string(view).expect("view serializes");
    format!("{{\"now\":{now},\"view\":{view}}}")
}

fn json_response(body: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) {
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

fn save(app: &App, games: &HashMap<String, Game>) {
    write_json(&app.state_path, games);
}

fn find_country(games: &Games, key: &str) -> Option<(String, usize)> {
    games.values().find_map(|g| {
        g.countries
            .iter()
            .position(|c| c.key.as_deref() == Some(key))
            .map(|i| (g.key.clone(), i))
    })
}

async fn with_game<R>(app: &App, game_key: &str, f: impl FnOnce(&mut Game, u64) -> R) -> Option<R> {
    let now = now_ms();
    let mut games = app.games.lock().await;
    let game = games.get_mut(game_key)?;
    game.last_seen = now;
    let changed = game.advance(now, &mut rand::rng());
    let result = f(game, now);
    if changed {
        save(app, &games);
    }
    Some(result)
}

async fn with_country<R>(
    app: &App,
    key: &str,
    f: impl FnOnce(&mut Game, usize, u64) -> R,
) -> Option<R> {
    let (game_key, country) = find_country(&app.games.lock().await, key)?;
    with_game(app, &game_key, |g, now| f(g, country, now)).await
}

async fn persist(app: &App) {
    let games = app.games.lock().await;
    save(app, &games);
}

async fn create_game(State(app): State<Shared>) -> Response {
    let now = now_ms();
    let mut games = app.games.lock().await;
    if games.len() >= MAX_GAMES {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many games are running.",
        );
    }
    let game = Game::new(now, &mut rand::rng());
    let key = game.key.clone();
    games.insert(key.clone(), game);
    save(&app, &games);
    Json(serde_json::json!({ "game": key })).into_response()
}

async fn game_state(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    match with_game(&app, &key, |g, now| envelope(now, &g.game_view())).await {
        Some(body) => json_response(body),
        None => not_found(),
    }
}

async fn update_settings(
    State(app): State<Shared>,
    Path(key): Path<String>,
    Json(settings): Json<Settings>,
) -> Response {
    let result = with_game(&app, &key, |g, now| {
        g.update_settings(settings)
            .map(|changed| (changed, envelope(now, &g.game_view())))
    })
    .await;
    match result {
        None => not_found(),
        Some(Err(msg)) => error(StatusCode::CONFLICT, msg),
        Some(Ok((changed, body))) => {
            if changed {
                persist(&app).await;
            }
            json_response(body)
        }
    }
}

async fn claim(State(app): State<Shared>, Path((key, country)): Path<(String, usize)>) -> Response {
    if country > 1 {
        return not_found();
    }
    let result = with_game(&app, &key, |g, _| g.claim(country, &mut rand::rng())).await;
    match result {
        None => not_found(),
        Some(Err(msg)) => error(StatusCode::GONE, msg),
        Some(Ok(key)) => {
            persist(&app).await;
            Json(serde_json::json!({ "country": key })).into_response()
        }
    }
}

async fn country_state(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    match with_country(&app, &key, |g, c, now| {
        envelope(now, &g.country_view(c, now))
    })
    .await
    {
        Some(body) => json_response(body),
        None => not_found(),
    }
}

async fn country_action(
    app: Shared,
    key: String,
    action: impl FnOnce(&mut Game, usize, u64) -> Result<(), &'static str>,
) -> Response {
    let result = with_country(&app, &key, |g, c, now| {
        action(g, c, now).map(|()| envelope(now, &g.country_view(c, now)))
    })
    .await;
    match result {
        None => not_found(),
        Some(Err(msg)) => error(StatusCode::CONFLICT, msg),
        Some(Ok(body)) => {
            persist(&app).await;
            json_response(body)
        }
    }
}

async fn start(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |g, c, now| g.start(c, now, &mut rand::rng())).await
}

async fn launch(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |g, c, now| g.launch(c, now, &mut rand::rng())).await
}

async fn request_end(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    country_action(app, key, |g, c, now| {
        g.request_end(c, now, &mut rand::rng())
    })
    .await
}

async fn next_tick() {
    tokio::time::sleep(Duration::from_millis(TICK_MS - now_ms() % TICK_MS)).await;
}

fn stream(
    app: Shared,
    render: impl Fn(&mut Games, u64) -> Option<String> + Send + 'static,
) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(4);
    tokio::spawn(async move {
        let mut last = String::new();
        let mut last_sent = 0;
        loop {
            let now = now_ms();
            let view = {
                let mut games = app.games.lock().await;
                render(&mut games, now)
            };
            let Some(view) = view else { break };
            if view != last || now - last_sent >= HEARTBEAT_MS {
                let data = format!("{{\"now\":{now},\"view\":{view}}}");
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    break;
                }
                last = view;
                last_sent = now;
            }
            next_tick().await;
        }
    });
    (
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

fn tick_game(app: &App, games: &mut Games, game_key: &str, now: u64) -> bool {
    let Some(game) = games.get_mut(game_key) else {
        return false;
    };
    game.last_seen = now;
    if game.advance(now, &mut rand::rng()) {
        save(app, games);
    }
    true
}

async fn game_events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    if !app.games.lock().await.contains_key(&key) {
        return not_found();
    }
    let shared = app.clone();
    stream(app, move |games, now| {
        tick_game(&shared, games, &key, now)
            .then(|| serde_json::to_string(&games[&key].game_view()).expect("view serializes"))
    })
}

async fn country_events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    let Some((game_key, country)) = find_country(&app.games.lock().await, &key) else {
        return not_found();
    };
    let shared = app.clone();
    stream(app, move |games, now| {
        tick_game(&shared, games, &game_key, now).then(|| {
            serde_json::to_string(&games[&game_key].country_view(country, now))
                .expect("view serializes")
        })
    })
}

async fn qr(body: Bytes) -> Response {
    if body.len() > 512 {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "Too long for a QR code.");
    }
    match QrCode::new(&body) {
        Ok(code) => {
            let image = code
                .render::<svg::Color>()
                .min_dimensions(240, 240)
                .dark_color(svg::Color("#000000"))
                .light_color(svg::Color("#ffffff"))
                .build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], image).into_response()
        }
        Err(_) => error(StatusCode::BAD_REQUEST, "Cannot encode this as a QR code."),
    }
}

async fn time() -> Response {
    json_response(format!("{{\"now\":{}}}", now_ms()))
}

async fn list_groups(State(app): State<Shared>) -> Response {
    let groups = app.groups.lock().await;
    let mut public: Vec<_> = groups.iter().map(Group::public).collect();
    public.sort_by_key(|g| std::cmp::Reverse(g.updated));
    Json(public).into_response()
}

async fn create_group(State(app): State<Shared>, Json(input): Json<GroupInput>) -> Response {
    if let Err(msg) = input.validate() {
        return error(StatusCode::BAD_REQUEST, &msg);
    }
    let password = input.password.clone();
    let hash = tokio::task::spawn_blocking(move || groups::hash_password(&password))
        .await
        .expect("hashing task");
    let mut groups = app.groups.lock().await;
    if groups.len() >= groups::MAX_GROUPS {
        return error(StatusCode::SERVICE_UNAVAILABLE, "The list is full.");
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
        .find(|g| g.id == id)
        .map(|g| g.password_hash.clone())
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
    if let Err(msg) = input.validate() {
        return error(StatusCode::BAD_REQUEST, &msg);
    }
    if let Err((status, msg)) = authorize(&app, &id, input.password.clone()).await {
        return error(status, msg);
    }
    let mut groups = app.groups.lock().await;
    let Some(group) = groups.iter_mut().find(|g| g.id == id) else {
        return error(StatusCode::NOT_FOUND, "This group no longer exists.");
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
    if let Err((status, msg)) = authorize(&app, &id, body.password).await {
        return error(status, msg);
    }
    let mut groups = app.groups.lock().await;
    groups.retain(|g| g.id != id);
    write_json(&app.groups_path, &*groups);
    StatusCode::NO_CONTENT.into_response()
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn cleanup(app: Shared) {
    tokio::time::sleep(Duration::from_secs(10 * 60)).await;
    loop {
        let now = now_ms();
        let mut games = app.games.lock().await;
        games.retain(|_, g| match g.over_at {
            Some(over) => over + 30 * DAY_MS > now,
            None => g.last_seen + DAY_MS > now,
        });
        save(&app, &games);
        drop(games);
        let mut groups = app.groups.lock().await;
        let before = groups.len();
        groups.retain(|g| g.updated + 400 * DAY_MS > now);
        if groups.len() != before {
            write_json(&app.groups_path, &*groups);
        }
        drop(groups);
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
    }
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("PETROV_ADDR").unwrap_or_else(|_| "127.0.0.1:8095".into());
    let state_path =
        PathBuf::from(std::env::var("PETROV_STATE").unwrap_or_else(|_| "games.json".into()));

    let groups_path = state_path.with_file_name("groups.json");
    let games: HashMap<String, Game> = read_json(&state_path);
    let groups: Vec<Group> = read_json(&groups_path);

    let app = Arc::new(App {
        games: Mutex::new(games),
        state_path,
        groups: Mutex::new(groups),
        groups_path,
    });
    tokio::spawn(cleanup(app.clone()));

    let router = Router::new()
        .route("/", get(index))
        .route("/check", get(index))
        .route("/groups", get(index))
        .route("/info", get(index))
        .route("/g/{key}", get(index))
        .route("/k/{key}", get(index))
        .route(
            "/manifest.webmanifest",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/manifest+json")],
                    MANIFEST,
                )
            }),
        )
        .route(
            "/icon.svg",
            get(|| async { ([(header::CONTENT_TYPE, "image/svg+xml")], ICON) }),
        )
        .route("/api/time", get(time))
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/{id}", put(update_group).delete(delete_group))
        .route("/api/qr", post(qr))
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
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("listening on http://{addr}");
    axum::serve(listener, router).await.expect("server error");
}
