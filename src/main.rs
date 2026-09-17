use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_stream::wrappers::ReceiverStream;

const INDEX: &str = include_str!("../static/index.html");
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const HEARTBEAT_MS: u64 = 10_000;

#[derive(Clone, Serialize, Deserialize)]
struct House {
    name: String,
    key: String,
    consequence: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Missile {
    from: usize,
    launched: u64,
    impact: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct FalseAlarm {
    to: usize,
    detected: u64,
    impact: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Game {
    id: String,
    created: u64,
    start: u64,
    end: u64,
    flight: u64,
    false_alarm_rate: f64,
    houses: [House; 2],
    missiles: Vec<Missile>,
    false_alarms: Vec<FalseAlarm>,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Pending,
    Active,
    Over,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Outcome {
    Impact,
    Dud,
}

#[derive(Serialize, PartialEq)]
struct Warning {
    detected: u64,
    impact: u64,
    outcome: Option<Outcome>,
}

#[derive(Serialize, PartialEq)]
struct DebriefEntry {
    t: u64,
    text: String,
}

#[derive(Serialize, PartialEq)]
struct View {
    phase: Phase,
    start: u64,
    end: u64,
    flight: u64,
    me: String,
    enemy: String,
    consequence: String,
    destroyed: Option<u64>,
    enemy_destroyed: Option<u64>,
    launched: Option<u64>,
    my_impact: Option<u64>,
    can_launch: bool,
    warnings: Vec<Warning>,
    debrief: Option<Vec<DebriefEntry>>,
}

#[derive(Serialize)]
struct Envelope<'a> {
    now: u64,
    view: &'a View,
}

impl Game {
    fn missile_from(&self, house: usize) -> Option<&Missile> {
        self.missiles.iter().find(|m| m.from == house)
    }

    fn destroyed_at(&self, house: usize, now: u64) -> Option<u64> {
        self.missiles
            .iter()
            .filter(|m| m.from != house && m.impact <= now)
            .map(|m| m.impact)
            .min()
    }

    fn visible_false_alarms(&self, house: usize, now: u64) -> impl Iterator<Item = &FalseAlarm> {
        let destroyed = self.destroyed_at(house, now);
        self.false_alarms.iter().filter(move |f| {
            f.to == house && f.detected <= now && destroyed.is_none_or(|d| f.detected < d)
        })
    }

    fn phase(&self, now: u64) -> Phase {
        let landed = self.missiles.iter().all(|m| m.impact <= now);
        let anyone_destroyed = (0..2).any(|h| self.destroyed_at(h, now).is_some());
        if landed && (now >= self.end || anyone_destroyed) {
            Phase::Over
        } else if now < self.start {
            Phase::Pending
        } else {
            Phase::Active
        }
    }

    fn can_launch(&self, house: usize, now: u64) -> Result<(), &'static str> {
        if now < self.start {
            return Err("The ceremony has not started yet.");
        }
        if now >= self.end || self.phase(now) == Phase::Over {
            return Err("The ceremony is over.");
        }
        if self.destroyed_at(house, now).is_some() {
            return Err("Your house has been destroyed.");
        }
        if self.missile_from(house).is_some() {
            return Err("You have already launched.");
        }
        Ok(())
    }

    fn view(&self, house: usize, now: u64) -> View {
        let enemy = 1 - house;
        let phase = self.phase(now);

        let mut warnings: Vec<Warning> = self
            .missiles
            .iter()
            .filter(|m| m.from == enemy && m.launched <= now)
            .map(|m| Warning {
                detected: m.launched,
                impact: m.impact,
                outcome: (m.impact <= now).then_some(Outcome::Impact),
            })
            .chain(self.visible_false_alarms(house, now).map(|f| Warning {
                detected: f.detected,
                impact: f.impact,
                outcome: (f.impact <= now).then_some(Outcome::Dud),
            }))
            .collect();
        warnings.sort_by_key(|w| w.detected);

        let debrief = (phase == Phase::Over).then(|| self.debrief(now));

        View {
            phase,
            start: self.start,
            end: self.end,
            flight: self.flight,
            me: self.houses[house].name.clone(),
            enemy: self.houses[enemy].name.clone(),
            consequence: self.houses[house].consequence.clone(),
            destroyed: self.destroyed_at(house, now),
            enemy_destroyed: self.destroyed_at(enemy, now),
            launched: self.missile_from(house).map(|m| m.launched),
            my_impact: self.missile_from(house).map(|m| m.impact),
            can_launch: self.can_launch(house, now).is_ok(),
            warnings,
            debrief,
        }
    }

    fn debrief(&self, now: u64) -> Vec<DebriefEntry> {
        let mut entries = Vec::new();
        for h in 0..2 {
            let name = &self.houses[h].name;
            for f in self.visible_false_alarms(h, now) {
                entries.push(DebriefEntry {
                    t: f.detected,
                    text: format!("False alarm at {name}: sensors reported an incoming missile."),
                });
            }
        }
        for m in &self.missiles {
            let from = &self.houses[m.from].name;
            let to = &self.houses[1 - m.from].name;
            entries.push(DebriefEntry {
                t: m.launched,
                text: format!("{from} launched at {to}."),
            });
            entries.push(DebriefEntry {
                t: m.impact,
                text: format!("A missile from {from} destroyed {to}."),
            });
        }
        if self.missiles.is_empty() {
            entries.push(DebriefEntry {
                t: self.end,
                text: "Nobody launched. Both houses survived.".into(),
            });
        }
        entries.sort_by_key(|e| e.t);
        entries
    }
}

#[derive(Deserialize)]
struct NewHouse {
    name: String,
    consequence: String,
}

#[derive(Deserialize)]
struct CreateGame {
    start: u64,
    end: u64,
    flight_secs: u64,
    false_alarm_chance: f64,
    houses: [NewHouse; 2],
}

#[derive(Serialize)]
struct CreatedHouse {
    name: String,
    key: String,
}

#[derive(Serialize)]
struct Created {
    id: String,
    houses: Vec<CreatedHouse>,
}

struct App {
    games: Mutex<HashMap<String, Game>>,
    notify: Notify,
    state_path: PathBuf,
}

type Shared = Arc<App>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_millis() as u64
}

fn random_key(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn poisson(rng: &mut impl Rng, lambda: f64) -> usize {
    let limit = (-lambda).exp();
    let mut k = 0;
    let mut p = 1.0;
    loop {
        p *= rng.random::<f64>();
        if p <= limit || k >= 20 {
            return k;
        }
        k += 1;
    }
}

fn schedule_false_alarms(
    rng: &mut impl Rng,
    house: usize,
    start: u64,
    end: u64,
    flight: u64,
    rate: f64,
) -> Vec<FalseAlarm> {
    let latest = end - flight;
    let count = poisson(rng, rate);
    let mut times: Vec<u64> = Vec::new();
    for _ in 0..count * 50 {
        if times.len() == count {
            break;
        }
        let t = rng.random_range(start..=latest);
        if times.iter().all(|&o| o.abs_diff(t) >= 2 * flight) {
            times.push(t);
        }
    }
    times
        .into_iter()
        .map(|detected| FalseAlarm {
            to: house,
            detected,
            impact: detected + flight,
        })
        .collect()
}

fn find_house<'a>(games: &'a HashMap<String, Game>, key: &str) -> Option<(&'a Game, usize)> {
    games
        .values()
        .find_map(|g| g.houses.iter().position(|h| h.key == key).map(|i| (g, i)))
}

async fn save(app: &App, games: &HashMap<String, Game>) {
    let json = match serde_json::to_vec_pretty(games) {
        Ok(json) => json,
        Err(e) => return eprintln!("failed to serialize state: {e}"),
    };
    let tmp = app.state_path.with_extension("tmp");
    if let Err(e) = tokio::fs::write(&tmp, json).await {
        return eprintln!("failed to write {}: {e}", tmp.display());
    }
    if let Err(e) = tokio::fs::rename(&tmp, &app.state_path).await {
        eprintln!("failed to replace {}: {e}", app.state_path.display());
    }
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, msg.to_string()).into_response()
}

async fn create_game(State(app): State<Shared>, Json(req): Json<CreateGame>) -> Response {
    let now = now_ms();
    let flight = req.flight_secs * 1000;
    if !(10..=600).contains(&req.flight_secs) {
        return bad_request("Retaliation window must be between 10 and 600 seconds.");
    }
    if req.end <= req.start || req.end - req.start > DAY_MS {
        return bad_request("The ceremony must end after it starts and last at most a day.");
    }
    if req.end - req.start <= flight {
        return bad_request("The ceremony must be longer than the retaliation window.");
    }
    if req.end <= now || req.start > now + 30 * DAY_MS {
        return bad_request("The ceremony must end in the future and start within 30 days.");
    }
    if !(0.0..=99.0).contains(&req.false_alarm_chance) {
        return bad_request("The false alarm chance must be between 0 and 99 percent.");
    }
    let false_alarm_rate = -(1.0 - req.false_alarm_chance / 100.0).ln();
    for h in &req.houses {
        let name = h.name.trim();
        if name.is_empty() || name.chars().count() > 60 || h.consequence.chars().count() > 500 {
            return bad_request("House names need 1 to 60 characters, consequences at most 500.");
        }
    }
    if req.houses[0].name.trim() == req.houses[1].name.trim() {
        return bad_request("The two houses need different names.");
    }

    let mut false_alarms: Vec<FalseAlarm> = {
        let mut rng = rand::rng();
        (0..2)
            .flat_map(|h| {
                schedule_false_alarms(&mut rng, h, req.start, req.end, flight, false_alarm_rate)
            })
            .collect()
    };
    false_alarms.sort_by_key(|f| f.detected);

    let [a, b] = req.houses;
    let make = |h: NewHouse| House {
        name: h.name.trim().to_string(),
        key: random_key(24),
        consequence: h.consequence.trim().to_string(),
    };
    let game = Game {
        id: random_key(12),
        created: now,
        start: req.start,
        end: req.end,
        flight,
        false_alarm_rate,
        houses: [make(a), make(b)],
        missiles: Vec::new(),
        false_alarms,
    };
    let created = Created {
        id: game.id.clone(),
        houses: game
            .houses
            .iter()
            .map(|h| CreatedHouse {
                name: h.name.clone(),
                key: h.key.clone(),
            })
            .collect(),
    };

    let mut games = app.games.lock().await;
    games.retain(|_, g| g.end + 7 * DAY_MS > now);
    if games.len() >= 500 {
        return (StatusCode::SERVICE_UNAVAILABLE, "Too many games.").into_response();
    }
    games.insert(game.id.clone(), game);
    save(&app, &games).await;
    Json(created).into_response()
}

async fn house_state(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    let now = now_ms();
    let games = app.games.lock().await;
    match find_house(&games, &key) {
        Some((game, house)) => {
            let view = game.view(house, now);
            Json(Envelope { now, view: &view }).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Unknown house.").into_response(),
    }
}

async fn launch(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    let now = now_ms();
    let mut games = app.games.lock().await;
    let Some((id, house)) = find_house(&games, &key).map(|(g, h)| (g.id.clone(), h)) else {
        return (StatusCode::NOT_FOUND, "Unknown house.").into_response();
    };
    let game = games.get_mut(&id).expect("game exists");
    if let Err(msg) = game.can_launch(house, now) {
        return (StatusCode::CONFLICT, msg).into_response();
    }
    game.missiles.push(Missile {
        from: house,
        launched: now,
        impact: now + game.flight,
    });
    let view = game.view(house, now);
    let body = Json(Envelope { now, view: &view }).into_response();
    save(&app, &games).await;
    drop(games);
    app.notify.notify_waiters();
    body
}

async fn events(State(app): State<Shared>, Path(key): Path<String>) -> Response {
    if find_house(&*app.games.lock().await, &key).is_none() {
        return (StatusCode::NOT_FOUND, "Unknown house.").into_response();
    }
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(4);
    tokio::spawn(async move {
        let mut last: Option<View> = None;
        let mut last_sent = 0;
        loop {
            let notified = app.notify.notified();
            let now = now_ms();
            let view = match find_house(&*app.games.lock().await, &key) {
                Some((game, house)) => game.view(house, now),
                None => break,
            };
            if last.as_ref() != Some(&view) || now - last_sent >= HEARTBEAT_MS {
                let data =
                    serde_json::to_string(&Envelope { now, view: &view }).expect("view serializes");
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    break;
                }
                last = Some(view);
                last_sent = now;
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    });
    (
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("PETROV_ADDR").unwrap_or_else(|_| "127.0.0.1:8095".into());
    let state_path =
        PathBuf::from(std::env::var("PETROV_STATE").unwrap_or_else(|_| "games.json".into()));

    let games = match std::fs::read(&state_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("corrupt state file {}: {e}", state_path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => panic!("cannot read {}: {e}", state_path.display()),
    };

    let app = Arc::new(App {
        games: Mutex::new(games),
        notify: Notify::new(),
        state_path,
    });

    let router = Router::new()
        .route("/", get(index))
        .route("/h/{key}", get(index))
        .route("/api/games", post(create_game))
        .route("/api/h/{key}", get(house_state))
        .route("/api/h/{key}/events", get(events))
        .route("/api/h/{key}/launch", post(launch))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("listening on http://{addr}");
    axum::serve(listener, router).await.expect("server error");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(false_alarms: Vec<FalseAlarm>) -> Game {
        let house = |name: &str| House {
            name: name.into(),
            key: name.into(),
            consequence: String::new(),
        };
        Game {
            id: "g".into(),
            created: 0,
            start: 1_000,
            end: 100_000,
            flight: 10_000,
            false_alarm_rate: 0.0,
            houses: [house("a"), house("b")],
            missiles: Vec::new(),
            false_alarms,
        }
    }

    #[test]
    fn false_alarm_looks_like_a_missile_until_impact() {
        let fake = game(vec![FalseAlarm {
            to: 1,
            detected: 5_000,
            impact: 15_000,
        }]);
        let mut real = game(Vec::new());
        real.missiles.push(Missile {
            from: 0,
            launched: 5_000,
            impact: 15_000,
        });

        for t in [5_000, 9_000, 14_999] {
            assert_eq!(
                serde_json::to_string(&fake.view(1, t)).unwrap(),
                serde_json::to_string(&real.view(1, t)).unwrap()
            );
        }

        let after = fake.view(1, 15_000);
        assert!(after.destroyed.is_none());
        assert!(after.warnings[0].outcome == Some(Outcome::Dud));
        assert!(fake.can_launch(1, 15_000).is_ok());
        assert!(real.view(1, 15_000).destroyed == Some(15_000));
    }

    #[test]
    fn retaliation_destroys_both() {
        let mut g = game(Vec::new());
        g.missiles.push(Missile {
            from: 0,
            launched: 5_000,
            impact: 15_000,
        });
        assert!(g.can_launch(1, 14_999).is_ok());
        g.missiles.push(Missile {
            from: 1,
            launched: 14_999,
            impact: 24_999,
        });
        assert!(g.can_launch(0, 20_000).is_err());
        assert!(g.view(0, 20_000).phase == Phase::Active);
        assert!(g.can_launch(1, 15_000).is_err());
        let end = g.view(0, 25_000);
        assert!(
            end.phase == Phase::Over && end.destroyed.is_some() && end.enemy_destroyed.is_some()
        );
    }

    #[test]
    fn launch_window_and_single_missile() {
        let mut g = game(Vec::new());
        assert!(g.can_launch(0, 999).is_err());
        assert!(g.can_launch(0, 100_000).is_err());
        g.missiles.push(Missile {
            from: 0,
            launched: 2_000,
            impact: 12_000,
        });
        assert!(g.can_launch(0, 3_000).is_err());
        assert!(g.view(0, 50_000).phase == Phase::Over);
    }

    #[test]
    fn false_alarms_fit_inside_the_ceremony() {
        let mut rng = rand::rng();
        for _ in 0..200 {
            for f in schedule_false_alarms(&mut rng, 0, 1_000, 3_601_000, 60_000, 3.0) {
                assert!(f.detected >= 1_000 && f.impact <= 3_601_000);
            }
        }
    }

    #[test]
    fn false_alarms_are_private_until_the_debrief() {
        let g = game(vec![FalseAlarm {
            to: 0,
            detected: 5_000,
            impact: 15_000,
        }]);
        assert!(g.view(0, 10_000).warnings.len() == 1);
        assert!(g.view(1, 10_000).warnings.is_empty());
        assert!(g.view(1, 20_000).warnings.is_empty());
        let debrief = g.view(1, 100_000).debrief.expect("over");
        assert!(
            debrief
                .iter()
                .any(|e| e.text.starts_with("False alarm at a"))
        );
    }

    #[test]
    fn chance_maps_to_probability_of_any_false_alarm() {
        let mut rng = rand::rng();
        let rate = -(1.0_f64 - 0.3).ln();
        let hits = (0..20_000)
            .filter(|_| !schedule_false_alarms(&mut rng, 0, 0, 3_600_000, 60_000, rate).is_empty())
            .count();
        assert!((5_600..6_400).contains(&hits), "{hits}");
    }
}
