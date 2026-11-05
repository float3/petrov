//! Multiplayer Petrov Day.
//!
//! Two groups, one missile each, a warning window, and false alarms that
//! look exactly like real launches. The keys, the event streams, the
//! store and the QR codes are [`rooms`]. What is here is the ritual, the
//! two brands it is served under, and the list of groups playing it this
//! year.

pub mod brand;
pub mod game;
mod groups;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use brand::Brand;
use game::{Access, Game, Settings};
use groups::{Group, GroupInput};
use rooms::{
    Rooms, http,
    lobby::{self, HasLobby},
    now_ms, site,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

const INDEX: &str = include_str!("../static/index.html");
const MANIFEST: &str = include_str!("../static/manifest.webmanifest");
const ICON: &str = include_str!("../static/icon.svg");
/// The page's own half, compiled from the TypeScript in `client/` by the
/// build script. Nothing here is written in JavaScript. The look stays in
/// the page itself: this one is a dark console, not a room's panels.
const APP_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/app.js"));

const MAX_GAMES: usize = 2000;

struct App {
    brand: &'static Brand,
    index: &'static str,
    manifest: &'static str,
    games: Arc<Rooms<Game>>,
    /// The groups playing this year. These are not rooms but one list,
    /// with a password per entry, emptied every year on the day.
    groups: Arc<Mutex<Vec<Group>>>,
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

/// Which game a country key, or a person's key, belongs to, and what it
/// may do.
async fn find_country(app: &App, key: &str) -> Option<(String, usize, Access)> {
    if let Some(at) = app.games.locate(key) {
        return app
            .games
            .read(&at, |game, _| game.country_for(key))
            .await
            .flatten()
            .map(|(country, access)| (at, country, access));
    }
    // A key handed out since the store last saved its index.
    let key = key.to_string();
    app.games
        .find(move |game_key, game| {
            game.country_for(&key)
                .map(|(country, access)| (game_key.to_string(), country, access))
        })
        .await
}

/// Runs `f` against a game, after telling it the time and letting it
/// advance. Saves afterwards.
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
            // The store files a game under a key of its own. The game
            // keeps the same key, so that anything holding a game knows
            // where it is stored.
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
        http::envelope(now, &game.game_view(now))
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
            .map(|_changed| http::envelope(now, &game.game_view(now)))
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

/// An action by one country, which only a command link may take.
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
    rooms::one_at(app.games.clone(), key, |game, now| {
        serde_json::to_string(&game.game_view(now)).ok()
    })
}

async fn country_events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    let Some((game_key, country, access)) = find_country(&app, &key).await else {
        return no_such_game();
    };
    let person = app
        .games
        .read(&game_key, |game, _| game.lobby.player(&key).is_some())
        .await
        .unwrap_or(false);
    if person {
        // A person seated by name: their stream keeps them connected, and
        // closes when the host takes them out.
        return lobby::player_events(app.games.clone(), key, |game, key, now| {
            let (country, access) = game.country_for(key)?;
            serde_json::to_string(&game.country_view(country, access, now)).ok()
        })
        .await;
    }
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

/// Empties the groups list every year on the day itself. The games are
/// swept by the store; see `impl Room for Game`.
async fn forget_old_groups(app: Shared) {
    loop {
        tokio::time::sleep(rooms::SWEEP).await;
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

/// The site, served at the root.
pub fn router(petrov: Petrov) -> Router {
    router_under(petrov, "")
}

/// The site, for mounting under `base` with `Router::nest`, which is how
/// the shared room in float3/rooms serves it next to the other games.
/// The shared room links to `/h/<key>` for the game and `/p/<key>` for a
/// player, which here are the setup screen and a country's console.
pub fn router_under(petrov: Petrov, base: &str) -> Router {
    let app = petrov.0;
    let base = base.trim_end_matches('/');
    let app = if base.is_empty() {
        app
    } else {
        // Once per mount when the server starts, so leaking is fine.
        Arc::new(App {
            index: Box::leak(site::rebase(app.index, base).into_boxed_str()),
            manifest: Box::leak(
                app.manifest
                    .replace("\"/", &format!("\"{base}/"))
                    .into_boxed_str(),
            ),
            brand: app.brand,
            games: app.games.clone(),
            groups: app.groups.clone(),
            groups_path: app.groups_path.clone(),
        })
    };
    Router::new()
        .route("/", get(index))
        .route("/h/{key}", get(index))
        .route("/p/{key}", get(index))
        .route("/check", get(index))
        .route("/groups", get(index))
        .route("/info", get(index))
        .route("/g/{key}", get(index))
        .route("/k/{key}", get(index))
        .route("/j/{key}", get(index))
        .route("/w/{key}", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route(
            "/icon.svg",
            get(|| async { ([(header::CONTENT_TYPE, "image/svg+xml")], ICON) }),
        )
        .route("/app.js", get(|| async { site::script(APP_JS) }))
        .route("/api/time", get(site::time))
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/{id}", put(update_group).delete(delete_group))
        .route("/api/qr", post(site::qr_code))
        .route("/api/games", post(create_game))
        .route("/api/g/{key}", get(game_state))
        .route("/api/g/{key}/events", get(game_events))
        .route("/api/g/{key}/settings", put(update_settings))
        .route("/api/g/{key}/claim/{country}", post(claim))
        // Joining, the host taking somebody out, and the wall.
        .merge(lobby::routes(app.games.clone()))
        .route("/api/k/{key}", get(country_state))
        .route("/api/k/{key}/events", get(country_events))
        .route("/api/k/{key}/start", post(start))
        .route("/api/k/{key}/launch", post(launch))
        .route("/api/k/{key}/end", post(request_end))
        .with_state(app)
}

/// Petrov Day as one server holds it: the games, this year's groups and
/// the brand it is served under.
#[derive(Clone)]
pub struct Petrov(Shared);

/// Loads the games kept at `state` and the groups kept at `groups`, and
/// starts sweeping both. `None` for a brand there is no such page for.
pub fn load(state: impl Into<PathBuf>, groups: impl Into<PathBuf>, brand: &str) -> Option<Petrov> {
    let brand = Brand::find(brand)?;
    let games: Arc<Rooms<Game>> = Rooms::load(state.into(), MAX_GAMES);
    let groups_path = groups.into();
    let list: Vec<Group> = read_json(&groups_path);
    let app = Arc::new(App {
        brand,
        index: Box::leak(brand.render(INDEX).into_boxed_str()),
        manifest: Box::leak(brand.render(MANIFEST).into_boxed_str()),
        games: games.clone(),
        groups: Arc::new(Mutex::new(list)),
        groups_path,
    });
    tokio::spawn({
        let games = games.clone();
        async move {
            games.each(|game| game.backfill(&mut rand::rng())).await;
            games.sweep(rooms::SWEEP).await;
        }
    });
    tokio::spawn(forget_old_groups(app.clone()));
    Some(Petrov(app))
}

/// Petrov Day as one of the shared room's games. A game is opened as the
/// setup screen opens one, and each person is seated by name in one of
/// the two countries, under a key of their own; see `Game`'s
/// [`HasLobby::arrived`].
impl rooms::tool::Tool for Petrov {
    fn id(&self) -> &'static str {
        "petrov"
    }

    fn title(&self) -> &'static str {
        self.0.brand.day
    }

    fn open(&self) -> rooms::tool::Boxed<'_, Option<String>> {
        Box::pin(async move {
            let now = now_ms();
            self.0
                .games
                .create(|key| {
                    let mut game = Game::new(now, &mut rand::rng());
                    game.key = key.to_string();
                    game
                })
                .await
        })
    }

    fn join<'a>(
        &'a self,
        host: &'a str,
        name: &'a str,
    ) -> rooms::tool::Boxed<'a, Option<Result<String, &'static str>>> {
        Box::pin(async move {
            with_game(&self.0, host, |game, now| {
                lobby::admit(game, name, None, now)
            })
            .await
        })
    }

    fn rejoin<'a>(
        &'a self,
        host: &'a str,
        known: Option<&'a str>,
        name: &'a str,
    ) -> rooms::tool::Boxed<'a, Option<Result<String, &'static str>>> {
        Box::pin(async move {
            with_game(&self.0, host, |game, now| {
                lobby::admit(game, name, known, now)
            })
            .await
        })
    }

    fn leave<'a>(&'a self, host: &'a str, key: &'a str) -> rooms::tool::Boxed<'a, Option<bool>> {
        Box::pin(async move {
            with_game(&self.0, host, |game, _| {
                lobby::remove_key(game, key).is_some()
            })
            .await
        })
    }

    fn wall<'a>(&'a self, host: &'a str) -> rooms::tool::Boxed<'a, Option<String>> {
        Box::pin(async move {
            self.0
                .games
                .read(host, |game, _| game.lobby.wall.clone())
                .await
        })
    }

    fn on_wall<'a>(&'a self, host: &'a str) -> rooms::tool::Boxed<'a, Option<lobby::OnWall>> {
        Box::pin(async move { self.0.games.look(host, |game, now| game.on_wall(now)).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use rooms::tool::Tool;
    use tower::ServiceExt;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("petrov-{name}-{}.json", std::process::id()))
    }

    async fn page(app: &Router, uri: &str) -> (StatusCode, String) {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn the_shared_room_opens_a_game_and_seats_a_room_in_it() {
        let (state, groups) = (temp("mount"), temp("mount-groups"));
        let petrov = load(&state, &groups, "petrov").expect("a brand");
        assert_eq!(petrov.title(), "Petrov Day");
        let host = petrov.open().await.expect("there is room");
        let ada = petrov.join(&host, "Ada").await.unwrap().unwrap();
        let bo = petrov.join(&host, "Bo").await.unwrap().unwrap();
        assert_ne!(ada, bo, "two people go to two countries");
        assert!(
            petrov.join(&host, "Ada").await.unwrap().is_err(),
            "Ada has only just joined, so her name is taken"
        );
        assert_eq!(
            petrov.rejoin(&host, Some(&ada), "Ada").await,
            Some(Ok(ada.clone())),
            "but the shared room, which kept her key, gets her back"
        );
        assert_eq!(petrov.join("no-such-game", "Ada").await, None);
        let wall = petrov.wall(&host).await.expect("a wall");
        assert_ne!(wall, host);
        let shows = petrov.on_wall(&host).await.expect("the game is there");
        assert!(shows.presence && !shows.busy, "before the game: {shows:?}");
        assert_eq!(
            shows.people,
            lobby::People::Hidden,
            "named by country instead"
        );
        for key in [&ada, &bo] {
            petrov
                .0
                .games
                .with(&host, |game, now| {
                    let (country, _) = game.country_for(key).unwrap();
                    game.start(country, now, &mut rand::rng())
                })
                .await
                .unwrap()
                .unwrap();
        }
        let shows = petrov.on_wall(&host).await.unwrap();
        assert!(!shows.presence && shows.busy, "while live: {shows:?}");
        assert_eq!(petrov.on_wall("no-such-game").await, None);

        let app = Router::new().nest("/petrov", router_under(petrov.clone(), "/petrov"));
        for link in [format!("/petrov/h/{host}"), format!("/petrov/p/{ada}")] {
            let (status, body) = page(&app, &link).await;
            assert_eq!(status, StatusCode::OK, "{link}");
            assert!(body.contains(r#"<meta name="rooms-base" content="/petrov">"#));
            assert!(body.contains(r#"src="/petrov/app.js""#));
        }
        let (status, body) = page(&app, &format!("/petrov/api/k/{ada}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""watch_only":false"#), "{body}");
        let (_, manifest) = page(&app, "/petrov/manifest.webmanifest").await;
        assert!(manifest.contains(r#""/petrov/icon.svg""#), "{manifest}");
        let (status, _) = page(&app, &format!("/petrov/api/w/{wall}")).await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(petrov.leave(&host, &bo).await, Some(true));
        assert_eq!(petrov.leave(&host, &bo).await, Some(false));
        let (status, _) = page(&app, &format!("/petrov/api/k/{bo}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "Bo was taken out");
        let _ = std::fs::remove_file(state);
        let _ = std::fs::remove_file(groups);
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, String) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    fn json(body: &str) -> serde_json::Value {
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {body}"))
    }

    /// A game opened on its own server, and its join and wall keys.
    async fn opened(name: &str) -> (Router, String, String, String, [PathBuf; 2]) {
        let paths = [temp(name), temp(&format!("{name}-groups"))];
        let app = router(load(&paths[0], &paths[1], "petrov").expect("a brand"));
        let (status, body) = send(&app, "POST", "/api/games", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let game = json(&body)["game"].as_str().unwrap().to_string();
        let (_, body) = page(&app, &format!("/api/g/{game}")).await;
        let view = &json(&body)["view"];
        let join = view["join"].as_str().unwrap().to_string();
        let wall = view["wall"].as_str().unwrap().to_string();
        (app, game, join, wall, paths)
    }

    #[tokio::test]
    async fn a_room_joins_by_name_and_the_host_can_take_somebody_out() {
        let (app, game, join, _, paths) = opened("join").await;
        let at = format!("/api/j/{join}");
        let (status, body) = send(&app, "POST", &at, serde_json::json!({ "name": "Ada" })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let ada = json(&body)["player"].as_str().unwrap().to_string();

        let (status, body) = send(&app, "POST", &at, serde_json::json!({ "name": " ada " })).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "Ada's phone is connected: {body}"
        );
        let (status, body) = send(
            &app,
            "POST",
            &at,
            serde_json::json!({ "name": "Ada", "key": ada }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            json(&body)["player"],
            ada.as_str(),
            "her kept key gets her back"
        );

        let (status, body) = page(&app, &format!("/api/k/{ada}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let view = &json(&body)["view"];
        assert_eq!(view["me"], 0);
        assert_eq!(view["watch_only"], false);
        assert_ne!(
            view["command_key"],
            ada.as_str(),
            "her key is not for sharing"
        );
        let (_, body) = page(&app, &at).await;
        assert_eq!(json(&body)["players"], 1);
        let (_, body) = page(&app, &format!("/api/g/{game}")).await;
        let person = json(&body)["view"]["people"][0].clone();
        assert_eq!(person["name"], "Ada");

        let (status, body) = send(
            &app,
            "POST",
            &format!("/api/h/{game}/remove"),
            serde_json::json!({ "id": person["id"] }),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        assert!(body.is_empty(), "{body}");
        let (_, body) = page(&app, &format!("/api/g/{game}")).await;
        assert_eq!(json(&body)["view"]["people"], serde_json::json!([]));
        for gone in [format!("/api/k/{ada}"), format!("/api/k/{ada}/events")] {
            let (status, _) = page(&app, &gone).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
        }
        let (status, _) = send(
            &app,
            "POST",
            &format!("/api/k/{ada}/start"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a removed person cannot act");
        let (status, body) = send(&app, "POST", &at, serde_json::json!({ "name": "Ada" })).await;
        assert_eq!(status, StatusCode::OK, "her name is free again: {body}");
        assert_ne!(json(&body)["player"], ada.as_str());
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn the_wall_has_its_own_key_and_tells_the_room_nothing_while_live() {
        let (app, game, join, wall, paths) = opened("wall").await;
        let at = format!("/api/j/{join}");
        let mut keys = Vec::new();
        for name in ["Ada", "Bo"] {
            let (_, body) = send(&app, "POST", &at, serde_json::json!({ "name": name })).await;
            keys.push(json(&body)["player"].as_str().unwrap().to_string());
        }
        let (ada, bo) = (&keys[0], &keys[1]);
        let (_, body) = page(&app, &format!("/api/g/{game}")).await;
        let bo_id = json(&body)["view"]["people"][1]["id"].clone();
        assert_eq!(json(&body)["view"]["people"][1]["name"], "Bo");

        let (status, body) = page(&app, &format!("/api/w/{wall}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let view = json(&body)["view"].clone();
        assert_eq!(view["join"], join.as_str());
        assert!(view["people"].is_null(), "names only by country: {view}");
        assert!(view["connected"].is_null(), "nobody's connection: {view}");
        assert_eq!(view["busy"], false);
        assert_eq!(
            view["game"]["countries"][0]["people"],
            serde_json::json!(["Ada"])
        );
        assert_eq!(
            view["game"]["countries"][1]["people"],
            serde_json::json!(["Bo"])
        );

        // Only the wall key opens the wall, and it opens nothing else.
        for key in [&game, &join, ada] {
            let (status, _) = page(&app, &format!("/api/w/{key}")).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{key}");
        }
        for path in [format!("/api/g/{wall}"), format!("/api/k/{wall}")] {
            let (status, _) = page(&app, &path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
        for path in [
            format!("/api/h/{wall}/remove"),
            format!("/api/k/{wall}/start"),
        ] {
            let (status, _) = send(&app, "POST", &path, serde_json::json!({ "id": 1 })).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }

        for key in [ada, bo] {
            let (status, body) = send(
                &app,
                "POST",
                &format!("/api/k/{key}/start"),
                serde_json::json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let live = || async {
            let (_, body) = page(&app, &format!("/api/w/{wall}")).await;
            json(&body)["view"].clone()
        };
        let quiet = live().await;
        assert_eq!(quiet["game"]["status"], "live");
        assert_eq!(quiet["busy"], true);
        assert!(
            quiet["connected"].is_null() && quiet["count"].is_null(),
            "{quiet}"
        );
        assert!(quiet["game"]["countries"].is_null(), "{quiet}");
        let (status, _) = send(
            &app,
            "POST",
            &format!("/api/k/{ada}/launch"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &app,
            "POST",
            &format!("/api/k/{bo}/end"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            live().await,
            quiet,
            "a launch and an end request leave the wall as it was"
        );
        let text = quiet.to_string();
        for secret in [ada, bo, &game] {
            assert!(!text.contains(secret.as_str()), "{text}");
        }

        let (status, body) = send(
            &app,
            "POST",
            &format!("/api/h/{game}/remove"),
            serde_json::json!({ "id": bo_id }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "not while live: {body}");
        assert!(body.contains("before the game is live"), "{body}");
        let (_, body) = page(&app, &format!("/api/g/{game}")).await;
        assert!(json(&body)["view"]["people"].is_null(), "{body}");
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }
}
