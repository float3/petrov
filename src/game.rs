use rand::{Rng, RngExt, distr::Alphanumeric};
use rooms::lobby::{HasLobby, Lobby, OnWall, People, Player, Present};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_NAME: usize = 1000;
pub const MAX_CONSEQUENCE: usize = 20_000;
const MAX_PERCENT_PER_MINUTE: f64 = 10_000.0;
const MAX_FLIGHT_SECS: u64 = 365 * 24 * 60 * 60;
pub const KEY_LEN: usize = 24;
/// Far more false alarms than any ritual can use, but it keeps `advance` from
/// scanning a list that grows without bound at high rates over a long game.
pub const MAX_FALSE_ALARMS: usize = 2000;

pub fn random_key(rng: &mut impl Rng, len: usize) -> String {
    (0..len).map(|_| rng.sample(Alphanumeric) as char).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub names: [String; 2],
    pub consequences: [String; 2],
    pub false_alarm_percent_per_minute: [f64; 2],
    pub flight_secs: u64,
    pub expected_minutes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            names: ["Country A".into(), "Country B".into()],
            consequences: [String::new(), String::new()],
            false_alarm_percent_per_minute: [0.02, 0.02],
            flight_secs: 300,
            expected_minutes: 60,
        }
    }
}

impl Settings {
    /// Each country's chance of at least one false alarm over the
    /// expected duration, at its rate.
    pub fn odds(&self) -> Odds {
        let minutes = self.expected_minutes;
        Odds {
            minutes,
            chances: self
                .false_alarm_percent_per_minute
                .map(|rate| 1.0 - (-(rate / 100.0) * minutes as f64).exp()),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.names.iter().any(|n| n.chars().count() > MAX_NAME) {
            return Err("Names can have at most 1000 characters.");
        }
        if self
            .consequences
            .iter()
            .any(|c| c.chars().count() > MAX_CONSEQUENCE)
        {
            return Err("Consequences can have at most 20000 characters.");
        }
        if self
            .false_alarm_percent_per_minute
            .iter()
            .any(|r| !(0.0..=MAX_PERCENT_PER_MINUTE).contains(r))
        {
            return Err("False-alarm rates must be between 0 and 10000 percent per minute.");
        }
        if !(1..=MAX_FLIGHT_SECS).contains(&self.flight_secs) {
            return Err("Flight time must be at least a second and at most a year.");
        }
        Ok(())
    }

    pub fn name(&self, country: usize) -> String {
        match self.names[country].trim() {
            "" => ["Country A", "Country B"][country].to_string(),
            name => name.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct CountrySettings {
    pub consequences: [String; 2],
    pub flight_secs: u64,
    /// See [`Odds`]. Only before the game.
    pub odds: Option<Odds>,
}

/// Each country's chance of at least one false alarm over the evening the
/// hosts expect. Everybody sees it before the game: it is what the hosts
/// chose, and it says nothing about when an alarm will come or whether a
/// warning is real. Never the rate per minute, and nothing once the game
/// is live.
#[derive(Debug, PartialEq, Serialize)]
pub struct Odds {
    pub minutes: u64,
    /// From 0 to 1, per country.
    pub chances: [f64; 2],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Country {
    pub key: Option<String>,
    #[serde(default)]
    pub watch_key: Option<String>,
    pub started: bool,
}

/// What a country link lets its holder do: the command link can start, launch
/// and request the end; the watch link only sees what the country sees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    Command,
    Watch,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Missile {
    pub id: String,
    pub from: usize,
    pub launched: u64,
    pub impact: u64,
    pub warning_on_screen: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FalseAlarm {
    pub id: String,
    pub to: usize,
    pub detected: u64,
    pub ends: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Game {
    pub key: String,
    pub created: u64,
    pub last_seen: u64,
    pub settings: Settings,
    pub countries: [Country; 2],
    pub live_at: Option<u64>,
    pub clock: u64,
    pub next_false_alarm: [Option<u64>; 2],
    pub false_alarms: Vec<FalseAlarm>,
    pub missiles: Vec<Missile>,
    pub end_requests: [Option<u64>; 2],
    pub over_at: Option<u64>,
    /// People seated here by name, by the shared room or through the join
    /// link on the wall. Each has a key of their own, which commands their
    /// country, and their country is their seat in the lobby. Games saved
    /// before the lobby get a fresh one, with fresh keys, when they load.
    #[serde(default)]
    pub lobby: Lobby,
    /// How games saved before the lobby kept who was seated where. Moved
    /// into the lobby once, when the games load; see [`Game::backfill`].
    #[serde(default, skip_serializing)]
    seated: Vec<(String, usize)>,
}

/// How long a game stays in the store. A finished game is kept for a
/// month, so that a group can look up what happened. A game that was
/// never played is deleted the next day.
const KEEP_OVER: u64 = 30 * rooms::DAY_MS;
const KEEP_IDLE: u64 = rooms::DAY_MS;

impl rooms::Room for Game {
    fn seen(&mut self, now: u64) {
        self.last_seen = now;
    }

    /// A game changes without anyone acting: missiles land, false alarms
    /// are raised, and the ritual ends. A watcher's tick drives that.
    fn tick(&mut self, now: u64) -> bool {
        self.advance(now, &mut rand::rng())
    }

    fn expired(&self, now: u64) -> bool {
        match self.over_at {
            Some(over) => over + KEEP_OVER <= now,
            None => self.last_seen + KEEP_IDLE <= now,
        }
    }

    /// The lobby's keys, and both countries' command and watch keys, so
    /// that a country link finds its game without a search.
    fn keys(&self) -> Vec<&str> {
        self.lobby
            .keys()
            .chain(
                self.countries
                    .iter()
                    .flat_map(|c| [c.key.as_deref(), c.watch_key.as_deref()])
                    .flatten(),
            )
            .collect()
    }
}

impl HasLobby for Game {
    fn lobby(&self) -> &Lobby {
        &self.lobby
    }

    fn lobby_mut(&mut self) -> &mut Lobby {
        &mut self.lobby
    }

    /// Seats somebody new in the country with fewer people, so that a
    /// room splits in two, and claims that country if nobody has yet.
    /// Everybody seated in a country commands it, under their own key.
    fn arrived(&mut self, _key: &str, _now: u64) -> Result<Option<u32>, &'static str> {
        let country = usize::from(self.seated_in(1) < self.seated_in(0));
        if self.countries[country].key.is_none() {
            self.claim(country, &mut rand::rng())?;
        }
        Ok(Some(country as u32))
    }

    /// The host may not take anybody out while the game is live: the game
    /// link then shows nothing about either country, and taking a
    /// country's people out from under it would be an act of war. The
    /// shared room still can, through `Tool::leave`.
    fn removable(&self, _player: &Player) -> Result<(), &'static str> {
        if self.status() == Status::Live {
            return Err("People can be taken out before the game is live or once it is over.");
        }
        Ok(())
    }

    fn wall_game(&self, _now: u64) -> Value {
        serde_json::to_value(self.wall_view()).unwrap_or(Value::Null)
    }

    /// The QR code stays up for latecomers, but the room's list of people
    /// does not: before the game, [`Game::wall_view`] names them by
    /// country instead. Nobody's connection is shown while the game is
    /// live, because a phone dropping out tells the room which country is
    /// still up; the shared room's wall and host follow this too.
    fn on_wall(&self, _now: u64) -> OnWall {
        let status = self.status();
        OnWall {
            join: true,
            people: People::Hidden,
            presence: status != Status::Live,
            busy: status != Status::Setup,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Setup,
    Live,
    Over,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WarningResult {
    Impact,
    Malfunction,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct WarningView {
    pub id: String,
    pub detected: u64,
    pub ends: u64,
    pub result: Option<WarningResult>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct OutgoingView {
    pub launched: u64,
    pub impact: u64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct CountryStatus {
    pub name: String,
    pub claimed: bool,
    pub started: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct TimelineEntry {
    pub t: u64,
    pub text: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Reveal {
    pub headline: String,
    pub timeline: Vec<TimelineEntry>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct CountryView {
    pub status: Status,
    pub me: usize,
    pub names: [String; 2],
    pub settings: CountrySettings,
    pub countries: Option<[CountryStatus; 2]>,
    pub live_at: Option<u64>,
    pub destroyed: Option<u64>,
    pub outgoing: Option<OutgoingView>,
    pub warnings: Vec<WarningView>,
    pub end_requested: Option<u64>,
    pub can_launch: bool,
    pub reveal: Option<Reveal>,
    pub watch_only: bool,
    pub watch_key: Option<String>,
    /// The country's command link, to share, for a device that may
    /// command. A person seated by name opens their own key, which is not
    /// for sharing.
    pub command_key: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ClaimSlot {
    pub name: String,
    pub claimed: bool,
    pub started: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GameSetup {
    pub names: [String; 2],
    pub settings: Settings,
    pub countries: [ClaimSlot; 2],
}

#[derive(Debug, PartialEq, Serialize)]
pub struct GameView<'a> {
    pub status: Status,
    pub setup: Option<GameSetup>,
    pub reveal: Option<Reveal>,
    /// Who is seated, with whose phone is connected. Not while the game is
    /// live: then the game link shows only the status.
    pub people: Option<Vec<Present<'a>>>,
    /// The countries' names, for the seats in `people`.
    pub names: [String; 2],
    /// The join link's key, for seating a room by name.
    pub join: &'a str,
    /// The wall's key, `/w/<key>`.
    pub wall: &'a str,
}

/// A country as the wall shows it before the game: who is in it, so the
/// room can split into two corners, and what it does when hit.
#[derive(Debug, PartialEq, Serialize)]
pub struct WallCountry {
    pub name: String,
    pub people: Vec<String>,
    pub claimed: bool,
    pub started: bool,
    pub consequence: String,
}

/// What the projector shows. Before the game, both countries and their
/// consequences. While it is live, only that it is live and since when:
/// never an alarm, a launch, an end request or who is connected, because
/// the whole room sees the wall and a country must learn nothing about
/// the other. At the end, the reveal.
#[derive(Debug, PartialEq, Serialize)]
pub struct WallView {
    pub status: Status,
    pub names: [String; 2],
    pub countries: Option<[WallCountry; 2]>,
    pub flight_secs: Option<u64>,
    /// See [`Odds`]. Only before the game.
    pub odds: Option<Odds>,
    pub live_at: Option<u64>,
    pub reveal: Option<Reveal>,
    pub consequences: Option<[String; 2]>,
}

impl Game {
    pub fn new(now: u64, rng: &mut impl Rng) -> Game {
        let country = || Country {
            key: None,
            watch_key: None,
            started: false,
        };
        Game {
            key: random_key(rng, KEY_LEN),
            created: now,
            last_seen: now,
            settings: Settings::default(),
            countries: [country(), country()],
            live_at: None,
            clock: now,
            next_false_alarm: [None, None],
            false_alarms: Vec::new(),
            missiles: Vec::new(),
            end_requests: [None, None],
            over_at: None,
            lobby: Lobby::new(),
            seated: Vec::new(),
        }
    }

    pub fn status(&self) -> Status {
        match (self.live_at, self.over_at) {
            (_, Some(_)) => Status::Over,
            (Some(_), None) => Status::Live,
            (None, None) => Status::Setup,
        }
    }

    fn names(&self) -> [String; 2] {
        [self.settings.name(0), self.settings.name(1)]
    }

    fn flight_ms(&self) -> u64 {
        self.settings.flight_secs * 1000
    }

    fn destroyed_at(&self, country: usize, t: u64) -> Option<u64> {
        self.missiles
            .iter()
            .filter(|m| m.from != country && m.impact <= t)
            .map(|m| m.impact)
            .min()
    }

    fn in_flight(&self, t: u64) -> bool {
        self.missiles
            .iter()
            .any(|m| m.launched <= t && t < m.impact)
            || self
                .false_alarms
                .iter()
                .any(|f| f.detected <= t && t < f.ends)
    }

    fn end_requested_by_anyone(&self, t: u64) -> bool {
        self.end_requests.iter().flatten().any(|&r| r <= t)
    }

    fn warning_active(&self, country: usize, t: u64) -> bool {
        self.missiles
            .iter()
            .any(|m| m.from != country && m.launched <= t && t < m.impact)
            || self
                .false_alarms
                .iter()
                .any(|f| f.to == country && f.detected <= t && t < f.ends)
    }

    fn false_alarm_gap(&self, country: usize, rng: &mut impl Rng) -> Option<u64> {
        let per_minute = self.settings.false_alarm_percent_per_minute[country] / 100.0;
        if per_minute <= 0.0 {
            return None;
        }
        let minutes = -(1.0 - rng.random::<f64>()).ln() / per_minute;
        Some((minutes * 60_000.0).round().clamp(1.0, 1e15) as u64)
    }

    fn check_over(&mut self, t: u64) -> bool {
        if self.over_at.is_some() || self.live_at.is_none() || self.in_flight(t) {
            return false;
        }
        let impacted = self.missiles.iter().any(|m| m.impact <= t);
        if self.end_requested_by_anyone(t) || impacted {
            self.over_at = Some(t);
            return true;
        }
        false
    }

    pub fn advance(&mut self, now: u64, rng: &mut impl Rng) -> bool {
        if self.status() != Status::Live || now <= self.clock {
            return false;
        }
        let mut changed = false;
        loop {
            let clock = self.clock;
            let next = self
                .next_false_alarm
                .iter()
                .flatten()
                .copied()
                .chain(self.missiles.iter().map(|m| m.impact))
                .chain(self.false_alarms.iter().map(|f| f.ends))
                .filter(|&t| t > clock)
                .min();
            let Some(t) = next.filter(|&t| t <= now) else {
                break;
            };
            self.clock = t;
            for country in 0..2 {
                if self.next_false_alarm[country] != Some(t) {
                    continue;
                }
                changed = true;
                if self.end_requested_by_anyone(t)
                    || self.destroyed_at(country, t).is_some()
                    || self.false_alarms.len() >= MAX_FALSE_ALARMS
                {
                    self.next_false_alarm[country] = None;
                    continue;
                }
                let alarm = FalseAlarm {
                    id: random_key(rng, 16),
                    to: country,
                    detected: t,
                    ends: t + self.flight_ms(),
                };
                self.false_alarms.push(alarm);
                self.next_false_alarm[country] = self.false_alarm_gap(country, rng).map(|g| t + g);
            }
            if self.check_over(t) {
                return true;
            }
        }
        self.clock = now;
        changed
    }

    pub fn update_settings(&mut self, settings: Settings) -> Result<bool, &'static str> {
        if self.status() != Status::Setup {
            return Err("Settings are locked once the game is live.");
        }
        settings.validate()?;
        if settings == self.settings {
            return Ok(false);
        }
        self.settings = settings;
        for country in &mut self.countries {
            country.started = false;
        }
        Ok(true)
    }

    pub fn claim(&mut self, country: usize, rng: &mut impl Rng) -> Result<String, &'static str> {
        if self.countries[country].key.is_some() {
            return Err(
                "This country has already been claimed. If that was not your host, set up a new game.",
            );
        }
        let key = random_key(rng, KEY_LEN);
        self.countries[country].key = Some(key.clone());
        self.countries[country].watch_key = Some(random_key(rng, KEY_LEN));
        Ok(key)
    }

    /// How many people are seated in `country`.
    fn seated_in(&self, country: usize) -> usize {
        self.lobby
            .players
            .iter()
            .filter(|p| p.seat == Some(country as u32))
            .count()
    }

    /// The host moves the person with `id` to the other country, before
    /// the game only: while it is live, moving somebody would change who
    /// commands a country under it, and the room is told nothing then.
    /// Their key then commands the other country. Both countries have to
    /// press Start again, since who is in them changed, as when the
    /// settings change. Answers the country they are in now.
    pub fn move_player(&mut self, id: u32) -> Result<usize, &'static str> {
        if self.status() != Status::Setup {
            return Err("People can be moved to the other country only before the game is live.");
        }
        let player = self
            .lobby
            .players
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("That person is not in the room any more.")?;
        let to = match player.seat {
            Some(0) => 1,
            Some(1) => 0,
            _ => return Err("That person is not in a country."),
        };
        player.seat = Some(to as u32);
        if self.countries[to].key.is_none() {
            self.claim(to, &mut rand::rng())?;
        }
        for country in &mut self.countries {
            country.started = false;
        }
        Ok(to)
    }

    /// Brings a game saved by an older version up to date: countries
    /// claimed before watch links existed get one, and people seated
    /// before the lobby are moved into it, keeping their country. They
    /// held the country's key then, which still works; their name gets
    /// them a key of their own.
    pub fn backfill(&mut self, rng: &mut impl Rng) {
        for country in &mut self.countries {
            if country.key.is_some() && country.watch_key.is_none() {
                country.watch_key = Some(random_key(rng, KEY_LEN));
            }
        }
        for (name, country) in std::mem::take(&mut self.seated) {
            if country < 2 {
                self.lobby.add(&name, Some(country as u32));
            }
        }
    }

    /// A person seated by name commands their country; so does the
    /// country's own link, and its watch link watches.
    pub fn country_for(&self, key: &str) -> Option<(usize, Access)> {
        if let Some(player) = self.lobby.player(key) {
            return player
                .seat
                .filter(|&seat| seat < 2)
                .map(|seat| (seat as usize, Access::Command));
        }
        self.countries.iter().enumerate().find_map(|(i, c)| {
            if c.key.as_deref() == Some(key) {
                Some((i, Access::Command))
            } else if c.watch_key.as_deref() == Some(key) {
                Some((i, Access::Watch))
            } else {
                None
            }
        })
    }

    pub fn start(
        &mut self,
        country: usize,
        now: u64,
        rng: &mut impl Rng,
    ) -> Result<(), &'static str> {
        match self.status() {
            Status::Setup => {}
            Status::Live => return Ok(()),
            Status::Over => return Err("The game is over."),
        }
        self.countries[country].started = true;
        if self.countries.iter().all(|c| c.started && c.key.is_some()) {
            self.live_at = Some(now);
            self.clock = now;
            for c in 0..2 {
                self.next_false_alarm[c] = self.false_alarm_gap(c, rng).map(|g| now + g);
            }
        }
        Ok(())
    }

    pub fn can_launch(&self, country: usize, now: u64) -> Result<(), &'static str> {
        match self.status() {
            Status::Setup => return Err("The game is not live yet."),
            Status::Over => return Err("The game is over."),
            Status::Live => {}
        }
        if self.destroyed_at(country, now).is_some() {
            return Err("Your country has been destroyed.");
        }
        if self.missiles.iter().any(|m| m.from == country) {
            return Err("Your missile has already been launched.");
        }
        if self.end_requests[country].is_some() && !self.warning_active(country, now) {
            return Err(
                "After requesting the end you can only launch while a warning is on screen.",
            );
        }
        Ok(())
    }

    pub fn launch(
        &mut self,
        country: usize,
        now: u64,
        rng: &mut impl Rng,
    ) -> Result<(), &'static str> {
        self.advance(now, rng);
        self.can_launch(country, now)?;
        let missile = Missile {
            id: random_key(rng, 16),
            from: country,
            launched: now,
            impact: now + self.flight_ms(),
            warning_on_screen: self.warning_active(country, now),
        };
        self.missiles.push(missile);
        Ok(())
    }

    pub fn request_end(
        &mut self,
        country: usize,
        now: u64,
        rng: &mut impl Rng,
    ) -> Result<(), &'static str> {
        self.advance(now, rng);
        match self.status() {
            Status::Setup => return Err("The game has not started."),
            Status::Over => return Ok(()),
            Status::Live => {}
        }
        if self.end_requests[country].is_none() {
            self.end_requests[country] = Some(now);
        }
        self.check_over(now);
        Ok(())
    }

    pub fn country_view(&self, country: usize, access: Access, now: u64) -> CountryView {
        let other = 1 - country;
        let status = self.status();
        let names = self.names();

        let mut warnings: Vec<WarningView> = self
            .missiles
            .iter()
            .filter(|m| m.from == other && m.launched <= now)
            .map(|m| WarningView {
                id: m.id.clone(),
                detected: m.launched,
                ends: m.impact,
                result: (m.impact <= now).then_some(WarningResult::Impact),
            })
            .chain(
                self.false_alarms
                    .iter()
                    .filter(|f| f.to == country && f.detected <= now)
                    .map(|f| WarningView {
                        id: f.id.clone(),
                        detected: f.detected,
                        ends: f.ends,
                        result: (f.ends <= now).then_some(WarningResult::Malfunction),
                    }),
            )
            .collect();
        warnings.sort_by(|a, b| (a.detected, &a.id).cmp(&(b.detected, &b.id)));

        let countries = (status == Status::Setup).then(|| {
            [0, 1].map(|c| CountryStatus {
                name: names[c].clone(),
                claimed: self.countries[c].key.is_some(),
                started: self.countries[c].started,
            })
        });

        CountryView {
            status,
            me: country,
            names,
            settings: CountrySettings {
                consequences: self.settings.consequences.clone(),
                flight_secs: self.settings.flight_secs,
                odds: (status == Status::Setup).then(|| self.settings.odds()),
            },
            countries,
            live_at: self.live_at,
            destroyed: self.destroyed_at(country, now),
            outgoing: self
                .missiles
                .iter()
                .find(|m| m.from == country)
                .map(|m| OutgoingView {
                    launched: m.launched,
                    impact: m.impact,
                }),
            warnings,
            end_requested: self.end_requests[country],
            can_launch: access == Access::Command && self.can_launch(country, now).is_ok(),
            reveal: (status == Status::Over).then(|| self.reveal()),
            watch_only: access == Access::Watch,
            watch_key: self.countries[country].watch_key.clone(),
            command_key: (access == Access::Command)
                .then(|| self.countries[country].key.clone())
                .flatten(),
        }
    }

    pub fn game_view(&self, now: u64) -> GameView<'_> {
        let status = self.status();
        let names = self.names();
        let setup = (status == Status::Setup).then(|| GameSetup {
            names: names.clone(),
            settings: self.settings.clone(),
            countries: [0, 1].map(|c| ClaimSlot {
                name: names[c].clone(),
                claimed: self.countries[c].key.is_some(),
                started: self.countries[c].started,
            }),
        });
        GameView {
            status,
            setup,
            reveal: (status == Status::Over).then(|| self.reveal()),
            people: (status != Status::Live).then(|| self.lobby.roster(now)),
            names,
            join: &self.lobby.join,
            wall: &self.lobby.wall,
        }
    }

    pub fn wall_view(&self) -> WallView {
        let status = self.status();
        let names = self.names();
        let countries = (status == Status::Setup).then(|| {
            [0, 1].map(|c| WallCountry {
                name: names[c].clone(),
                people: self
                    .lobby
                    .players
                    .iter()
                    .filter(|p| p.seat == Some(c as u32))
                    .map(|p| p.name.clone())
                    .collect(),
                claimed: self.countries[c].key.is_some(),
                started: self.countries[c].started,
                consequence: self.settings.consequences[c].clone(),
            })
        });
        WallView {
            status,
            names,
            countries,
            flight_secs: (status == Status::Setup).then_some(self.settings.flight_secs),
            odds: (status == Status::Setup).then(|| self.settings.odds()),
            live_at: self.live_at.filter(|_| status == Status::Live),
            reveal: (status == Status::Over).then(|| self.reveal()),
            consequences: (status == Status::Over).then(|| self.settings.consequences.clone()),
        }
    }

    pub fn reveal(&self) -> Reveal {
        let names = self.names();
        let mut timeline = Vec::new();
        if let Some(t) = self.live_at {
            timeline.push(TimelineEntry {
                t,
                text: "Both countries pressed Start. The game went live.".into(),
            });
        }
        for f in &self.false_alarms {
            timeline.push(TimelineEntry {
                t: f.detected,
                text: format!("False alarm at {}.", names[f.to]),
            });
            if self.held_fire(f) {
                timeline.push(TimelineEntry {
                    t: f.ends,
                    text: format!(
                        "{} held its fire through a false alarm ({} on screen).",
                        names[f.to],
                        clock(f.ends - f.detected)
                    ),
                });
            }
        }
        for m in &self.missiles {
            let (from, to) = (&names[m.from], &names[1 - m.from]);
            let screen = if m.warning_on_screen {
                "with a warning on screen"
            } else {
                "with no warning on screen"
            };
            timeline.push(TimelineEntry {
                t: m.launched,
                text: format!("{from} launched at {to}, {screen}."),
            });
        }
        for m in &self.missiles {
            let (from, to) = (&names[m.from], &names[1 - m.from]);
            timeline.push(TimelineEntry {
                t: m.impact,
                text: format!("The missile from {from} hit {to}. {to} was destroyed."),
            });
        }
        for (c, request) in self.end_requests.iter().enumerate() {
            if let Some(t) = *request {
                timeline.push(TimelineEntry {
                    t,
                    text: format!("{} requested the end.", names[c]),
                });
            }
        }
        if let Some(t) = self.over_at {
            timeline.push(TimelineEntry {
                t,
                text: "Game over.".into(),
            });
        }
        timeline.sort_by_key(|e| e.t);
        Reveal {
            headline: self.headline(),
            timeline,
        }
    }

    /// Whether the country a false alarm warned could have launched while
    /// it was on screen and did not: it still had its missile, was not
    /// destroyed before the warning ended, and launched nothing during it.
    /// A launch during the warning has its own line in the timeline.
    fn held_fire(&self, alarm: &FalseAlarm) -> bool {
        let country = alarm.to;
        let launched_by_end = self
            .missiles
            .iter()
            .any(|m| m.from == country && m.launched <= alarm.ends);
        !launched_by_end && self.destroyed_at(country, alarm.ends).is_none()
    }

    fn headline(&self) -> String {
        let names = self.names();
        let mut launches: Vec<&Missile> = self.missiles.iter().collect();
        launches.sort_by_key(|m| m.launched);
        match launches.as_slice() {
            [] => match self.false_alarms.len() {
                0 => "Peace held.".into(),
                1 => "Peace held through a false alarm.".into(),
                n => format!("Peace held through {n} false alarms."),
            },
            [m] => {
                let (from, to) = (&names[m.from], &names[1 - m.from]);
                if m.warning_on_screen {
                    format!("{from} retaliated against a false alarm and destroyed {to}.")
                } else {
                    format!("{to} was destroyed.")
                }
            }
            [first, ..] => {
                if first.warning_on_screen {
                    format!(
                        "{} retaliated against a false alarm. Mutual destruction.",
                        names[first.from]
                    )
                } else {
                    "Mutual destruction.".into()
                }
            }
        }
    }
}

/// A length of time as the screens show it: `5:00`, or `1:05:00`.
fn clock(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use rooms::lobby;

    const FLIGHT: u64 = 300_000;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    fn live_game(rates: [f64; 2]) -> (Game, StdRng) {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        g.update_settings(Settings {
            names: ["A".into(), "B".into()],
            consequences: ["cake".into(), "leave".into()],
            false_alarm_percent_per_minute: rates,
            flight_secs: FLIGHT / 1000,
            expected_minutes: 60,
        })
        .unwrap();
        g.claim(0, &mut rng).unwrap();
        g.claim(1, &mut rng).unwrap();
        g.start(0, 1_000, &mut rng).unwrap();
        g.start(1, 2_000, &mut rng).unwrap();
        (g, rng)
    }

    fn inject_false_alarm(g: &mut Game, to: usize, detected: u64) {
        g.false_alarms.push(FalseAlarm {
            id: format!("fa{detected}"),
            to,
            detected,
            ends: detected + FLIGHT,
        });
    }

    fn without_ids(view: &CountryView) -> serde_json::Value {
        let mut v = serde_json::to_value(view).unwrap();
        for w in v["warnings"].as_array_mut().unwrap() {
            w.as_object_mut().unwrap().remove("id");
        }
        v.as_object_mut().unwrap().remove("watch_key");
        v
    }

    #[test]
    fn goes_live_only_when_both_claimed_countries_start() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        g.claim(0, &mut rng).unwrap();
        g.start(0, 10, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Setup);
        g.start(1, 20, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Setup);
        g.claim(1, &mut rng).unwrap();
        g.start(1, 30, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Live);
        assert_eq!(g.live_at, Some(30));
    }

    #[test]
    fn countries_can_be_claimed_once() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        assert!(g.claim(1, &mut rng).is_ok());
        assert!(g.claim(1, &mut rng).is_err());
        assert!(g.game_view(0).setup.unwrap().countries[1].claimed);
    }

    #[test]
    fn changing_settings_resets_start_and_locks_when_live() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        g.claim(0, &mut rng).unwrap();
        g.start(0, 10, &mut rng).unwrap();
        let mut s = g.settings.clone();
        s.names[0] = "Ours".into();
        assert!(g.update_settings(s.clone()).unwrap());
        assert!(!g.countries[0].started);
        assert!(!g.update_settings(s.clone()).unwrap());

        let (mut live, _) = live_game([0.0, 0.0]);
        assert!(live.update_settings(s).is_err());
    }

    #[test]
    fn false_alarm_is_indistinguishable_from_a_missile_until_the_window_ends() {
        let (mut real, mut rng) = live_game([0.0, 0.0]);
        let (mut fake, _) = live_game([0.0, 0.0]);
        real.launch(0, 50_000, &mut rng).unwrap();
        inject_false_alarm(&mut fake, 1, 50_000);

        for t in [50_000, 120_000, 50_000 + FLIGHT - 1] {
            real.advance(t, &mut rng);
            fake.advance(t, &mut rng);
            assert_eq!(
                without_ids(&real.country_view(1, Access::Command, t)),
                without_ids(&fake.country_view(1, Access::Command, t))
            );
        }

        let t = 50_000 + FLIGHT;
        real.advance(t, &mut rng);
        fake.advance(t, &mut rng);
        let hit = real.country_view(1, Access::Command, t);
        let spared = fake.country_view(1, Access::Command, t);
        assert_eq!(hit.warnings[0].result, Some(WarningResult::Impact));
        assert_eq!(hit.destroyed, Some(t));
        assert_eq!(spared.warnings[0].result, Some(WarningResult::Malfunction));
        assert_eq!(spared.destroyed, None);
        assert!(fake.country_view(0, Access::Command, t).warnings.is_empty());
    }

    #[test]
    fn false_alarms_are_random_in_time_and_stop_after_an_end_request() {
        let hours = 100 * 3_600_000;
        let (mut g, mut rng) = live_game([10.0, 10.0]);
        g.advance(hours, &mut rng);
        assert!(g.false_alarms.len() > 500);
        assert!(
            g.false_alarms
                .iter()
                .filter(|f| (f.detected - 2_000) % 1000 == 0)
                .count()
                < 10
        );
        assert!(g.false_alarms.iter().all(|f| f.ends == f.detected + FLIGHT));

        g.request_end(0, hours, &mut rng).unwrap();
        let count = g.false_alarms.len();
        g.advance(hours + 10 * FLIGHT, &mut rng);
        assert_eq!(g.false_alarms.len(), count);
        assert_eq!(g.status(), Status::Over);
    }

    #[test]
    fn false_alarms_stop_at_the_cap() {
        let (mut g, mut rng) = live_game([10.0, 10.0]);
        g.advance(1000 * 3_600_000, &mut rng);
        assert_eq!(g.false_alarms.len(), MAX_FALSE_ALARMS);
        assert_eq!(g.next_false_alarm, [None, None]);
    }

    #[test]
    fn destroyed_country_gets_no_more_false_alarms() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.launch(0, 3_000, &mut rng).unwrap();
        inject_false_alarm(&mut g, 0, 3_000 + FLIGHT - 10);
        g.advance(3_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Live);
        g.next_false_alarm[1] = Some(3_000 + FLIGHT + 10);
        let before = g.false_alarms.len();
        g.advance(3_000 + FLIGHT + 20, &mut rng);
        assert_eq!(g.false_alarms.len(), before);
        assert_eq!(g.next_false_alarm[1], None);
    }

    #[test]
    fn one_missile_per_country_and_destroyed_countries_cannot_launch() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.launch(0, 10_000, &mut rng).unwrap();
        assert!(g.launch(0, 20_000, &mut rng).is_err());
        g.advance(10_000 + FLIGHT, &mut rng);
        assert!(g.launch(1, 10_000 + FLIGHT, &mut rng).is_err());
    }

    #[test]
    fn missile_launched_before_destruction_still_hits() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.launch(0, 10_000, &mut rng).unwrap();
        g.launch(1, 10_000 + FLIGHT - 1, &mut rng).unwrap();
        g.advance(10_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Live);
        g.advance(10_000 + 2 * FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        assert!(
            g.country_view(0, Access::Command, 10_000 + 2 * FLIGHT)
                .destroyed
                .is_some()
        );
        assert!(
            g.country_view(1, Access::Command, 10_000 + 2 * FLIGHT)
                .destroyed
                .is_some()
        );
        let reveal = g.reveal();
        assert_eq!(reveal.headline, "Mutual destruction.");
        assert!(
            reveal
                .timeline
                .iter()
                .any(|e| e.text == "B launched at A, with a warning on screen.")
        );
    }

    #[test]
    fn end_request_with_nothing_in_flight_ends_immediately() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.request_end(1, 60_000, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Over);
        assert_eq!(g.reveal().headline, "Peace held.");
    }

    #[test]
    fn the_reveal_says_when_a_country_held_its_fire_through_a_false_alarm() {
        let held = "B held its fire through a false alarm (5:00 on screen).";
        // Two false alarms, nobody launched.
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        inject_false_alarm(&mut g, 1, 10_000);
        inject_false_alarm(&mut g, 0, 20_000);
        g.advance(20_000 + FLIGHT, &mut rng);
        g.request_end(0, 20_000 + FLIGHT, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Over);
        let reveal = g.reveal();
        assert_eq!(reveal.headline, "Peace held through 2 false alarms.");
        let at = |text: &str| reveal.timeline.iter().find(|e| e.text == text).map(|e| e.t);
        assert_eq!(at(held), Some(10_000 + FLIGHT));
        assert_eq!(
            at("A held its fire through a false alarm (5:00 on screen)."),
            Some(20_000 + FLIGHT)
        );

        // A launch during the warning is not holding fire, and neither
        // is a warning to a country with no missile left.
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        inject_false_alarm(&mut g, 1, 10_000);
        g.launch(1, 15_000, &mut rng).unwrap();
        inject_false_alarm(&mut g, 1, 20_000);
        g.advance(15_000 + FLIGHT, &mut rng);
        let reveal = g.reveal();
        assert!(
            reveal
                .timeline
                .iter()
                .all(|e| !e.text.contains("held its fire")),
            "{:?}",
            reveal.timeline
        );

        // Nor is a warning that was still on screen when the country was hit.
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.launch(0, 3_000, &mut rng).unwrap();
        inject_false_alarm(&mut g, 1, 3_000 + FLIGHT - 10_000);
        g.advance(3_000 + 2 * FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        let reveal = g.reveal();
        assert_eq!(reveal.headline, "B was destroyed.");
        assert!(reveal.timeline.iter().all(|e| e.text != held));
        assert_eq!(clock(65 * 60_000), "1:05:00");
        assert_eq!(clock(1), "0:01");
    }

    #[test]
    fn end_request_waits_for_flight_and_stays_hidden_from_the_other_country() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        inject_false_alarm(&mut g, 1, 10_000);
        let before = serde_json::to_value(g.country_view(1, Access::Command, 20_000)).unwrap();
        g.request_end(0, 20_000, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Live);
        assert_eq!(
            serde_json::to_value(g.country_view(1, Access::Command, 20_000)).unwrap(),
            before
        );
        assert_eq!(
            g.country_view(0, Access::Command, 20_000).end_requested,
            Some(20_000)
        );

        assert!(g.launch(0, 30_000, &mut rng).is_err());
        g.launch(1, 30_000, &mut rng).unwrap();
        assert!(g.country_view(0, Access::Command, 30_000).can_launch);
        g.launch(0, 40_000, &mut rng).unwrap();
        g.advance(10_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Live);
        g.advance(40_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        assert_eq!(g.over_at, Some(40_000 + FLIGHT));
        assert_eq!(
            g.reveal().headline,
            "B retaliated against a false alarm. Mutual destruction."
        );
    }

    #[test]
    fn game_ends_after_an_impact_once_nothing_is_in_flight() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        g.launch(0, 10_000, &mut rng).unwrap();
        inject_false_alarm(&mut g, 0, 20_000);
        g.advance(10_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Live);
        g.advance(20_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        assert_eq!(g.reveal().headline, "B was destroyed.");
    }

    #[test]
    fn retaliating_against_a_false_alarm_is_called_out() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        inject_false_alarm(&mut g, 0, 10_000);
        g.launch(0, 15_000, &mut rng).unwrap();
        g.advance(15_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        assert_eq!(
            g.reveal().headline,
            "A retaliated against a false alarm and destroyed B."
        );
    }

    #[test]
    fn countries_see_the_chance_of_a_false_alarm_only_before_the_game() {
        let (g, _) = live_game([0.5, 0.5]);
        let mut rng = rng();
        let mut setup = Game::new(0, &mut rng);
        setup.settings.false_alarm_percent_per_minute = [0.02, 0.5];
        for json in [
            serde_json::to_string(&g.country_view(0, Access::Command, 5_000)).unwrap(),
            serde_json::to_string(&setup.country_view(1, Access::Command, 5_000)).unwrap(),
        ] {
            assert!(!json.contains("false_alarm"), "never the rate: {json}");
        }
        assert_eq!(
            g.country_view(0, Access::Command, 5_000).settings.odds,
            None
        );
        assert!(g.wall_view().odds.is_none());
        let odds = setup
            .country_view(1, Access::Command, 0)
            .settings
            .odds
            .unwrap();
        assert_eq!(odds.minutes, 60);
        assert!((odds.chances[0] - 0.0119).abs() < 0.0001, "{odds:?}");
        assert!((odds.chances[1] - 0.2592).abs() < 0.0001, "{odds:?}");
        assert_eq!(setup.wall_view().odds, Some(odds));
    }

    #[test]
    fn live_views_hide_the_other_country() {
        let (g, _) = live_game([0.0, 0.0]);
        assert!(
            g.country_view(0, Access::Command, 5_000)
                .countries
                .is_none()
        );
        let view = g.game_view(0);
        assert!(view.setup.is_none() && view.reveal.is_none());
    }

    #[test]
    fn watch_links_see_the_country_but_cannot_act() {
        let (g, _) = live_game([0.0, 0.0]);
        let command = g.countries[0].key.clone().unwrap();
        let watch = g.countries[0].watch_key.clone().unwrap();
        assert_ne!(command, watch);
        assert_eq!(g.country_for(&command), Some((0, Access::Command)));
        assert_eq!(g.country_for(&watch), Some((0, Access::Watch)));
        let commanding = g.country_view(0, Access::Command, 5_000);
        let watching = g.country_view(0, Access::Watch, 5_000);
        assert!(commanding.can_launch && !commanding.watch_only);
        assert!(!watching.can_launch && watching.watch_only);
        assert_eq!(watching.watch_key.as_deref(), Some(watch.as_str()));
    }

    #[test]
    fn a_room_seated_by_name_splits_between_the_two_countries() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        let ada = lobby::admit(&mut g, "Ada", None, 0).unwrap();
        let bo = lobby::admit(&mut g, "Bo", None, 0).unwrap();
        let cy = lobby::admit(&mut g, "Cy", None, 0).unwrap();
        assert_eq!(g.country_for(&ada), Some((0, Access::Command)));
        assert_eq!(g.country_for(&bo), Some((1, Access::Command)));
        assert_eq!(g.country_for(&cy), Some((0, Access::Command)));
        assert_ne!(cy, ada, "everybody has a key of their own");
        assert_ne!(g.countries[0].key.as_deref(), Some(ada.as_str()));
        assert!(g.countries.iter().all(|c| c.key.is_some()), "both claimed");
        assert_eq!(
            lobby::admit(&mut g, "bo", None, 1_000),
            Err("That name is taken. Add an initial, or pick another."),
            "the same name while Bo's phone is connected"
        );
        assert_eq!(lobby::admit(&mut g, "Bo", Some(&bo), 1_000), Ok(bo.clone()));
        assert_eq!(g.lobby.len(), 3);
        // A person's view hands them the country's own link to share.
        let view = g.country_view(1, Access::Command, 0);
        assert_eq!(view.command_key, g.countries[1].key);
        assert_eq!(g.country_view(1, Access::Watch, 0).command_key, None);
        // Every key a link can carry finds the game without a search.
        let keys = rooms::Room::keys(&g);
        for key in [&ada, &bo, &g.lobby.join, &g.lobby.wall] {
            assert!(keys.contains(&key.as_str()));
        }
        for c in &g.countries {
            assert!(keys.contains(&c.key.as_deref().unwrap()));
            assert!(keys.contains(&c.watch_key.as_deref().unwrap()));
        }
        // A country the host already claimed keeps its link.
        let mut h = Game::new(0, &mut rng);
        let claimed = h.claim(0, &mut rng).unwrap();
        lobby::admit(&mut h, "Ada", None, 0).unwrap();
        assert_eq!(h.countries[0].key.as_deref(), Some(claimed.as_str()));
        // Somebody taken out no longer commands anything.
        let id = g.lobby.player(&cy).unwrap().id;
        lobby::remove(&mut g, id).unwrap();
        assert_eq!(g.country_for(&cy), None);
    }

    #[test]
    fn the_host_can_move_somebody_to_the_other_country_before_the_game() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        let ada = lobby::admit(&mut g, "Ada", None, 0).unwrap();
        assert_eq!(g.country_for(&ada), Some((0, Access::Command)));
        assert!(g.countries[1].key.is_none(), "nobody in B yet");
        g.start(0, 0, &mut rng).unwrap();
        let id = g.lobby.player(&ada).unwrap().id;

        assert_eq!(g.move_player(id), Ok(1));
        assert_eq!(g.country_for(&ada), Some((1, Access::Command)));
        assert!(g.countries[1].key.is_some(), "B is claimed for her");
        assert!(!g.countries[0].started, "who is in A changed");
        assert_eq!(g.wall_view().countries.unwrap()[1].people, ["Ada"]);
        // Her key now commands B: pressing Start starts B, not A.
        g.start(1, 1_000, &mut rng).unwrap();
        assert!(g.countries[1].started && !g.countries[0].started);
        assert_eq!(g.move_player(id), Ok(0), "and back");
        assert_eq!(g.country_for(&ada), Some((0, Access::Command)));
        assert_eq!(
            g.move_player(id + 100),
            Err("That person is not in the room any more.")
        );
    }

    #[test]
    fn nobody_is_moved_once_the_game_is_live() {
        let (mut g, _) = live_game([0.0, 0.0]);
        let ada = lobby::admit(&mut g, "Ada", None, 5_000).unwrap();
        let (country, _) = g.country_for(&ada).unwrap();
        let wall = serde_json::to_string(&g.wall_view()).unwrap();
        let id = g.lobby.player(&ada).unwrap().id;
        assert_eq!(
            g.move_player(id),
            Err("People can be moved to the other country only before the game is live.")
        );
        assert_eq!(g.country_for(&ada), Some((country, Access::Command)));
        assert_eq!(serde_json::to_string(&g.wall_view()).unwrap(), wall);
    }

    #[test]
    fn games_saved_before_the_lobby_load_and_keep_their_seats() {
        let mut rng = rng();
        let mut g = Game::new(0, &mut rng);
        let country = g.claim(1, &mut rng).unwrap();
        let mut saved = serde_json::to_value(&g).unwrap();
        let old = saved.as_object_mut().unwrap();
        old.remove("lobby");
        old.insert(
            "seated".into(),
            serde_json::json!([["Ada", 0], ["Bo", 1], ["Cy", 0]]),
        );
        let mut loaded: Game = serde_json::from_value(saved).unwrap();
        loaded.backfill(&mut rng);
        assert_eq!(loaded.lobby.len(), 3);
        assert_eq!(loaded.lobby.named("Bo").unwrap().seat, Some(1));
        assert_eq!(loaded.country_for(&country), Some((1, Access::Command)));
        // Back by name, since nobody holds the new key yet.
        let bo = lobby::admit(&mut loaded, "Bo", None, 10_000).unwrap();
        assert_eq!(loaded.country_for(&bo), Some((1, Access::Command)));
        let again = serde_json::to_string(&loaded).unwrap();
        assert!(!again.contains("seated"), "{again}");
    }

    #[test]
    fn the_game_link_does_not_show_who_is_connected_while_live() {
        let (mut g, _) = live_game([0.0, 0.0]);
        lobby::admit(&mut g, "Ada", None, 5_000).unwrap();
        assert!(g.game_view(5_000).people.is_none());
        let mut rng = rng();
        let mut setup = Game::new(0, &mut rng);
        lobby::admit(&mut setup, "Ada", None, 0).unwrap();
        let people = setup.game_view(0).people.unwrap();
        assert_eq!(people[0].name, "Ada");
        assert_eq!(people[0].seat, Some(0));
    }

    #[test]
    fn the_wall_shows_nothing_of_what_happens_until_the_reveal() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        let ada = lobby::admit(&mut g, "Ada", None, 3_000).unwrap();
        let quiet = serde_json::to_string(&g.wall_view()).unwrap();
        assert!(quiet.contains(r#""status":"live""#), "{quiet}");

        // A false alarm, a launch, an end request: the wall stays the same.
        inject_false_alarm(&mut g, 1, 10_000);
        g.advance(20_000, &mut rng);
        assert_eq!(serde_json::to_string(&g.wall_view()).unwrap(), quiet);
        g.launch(1, 30_000, &mut rng).unwrap();
        g.request_end(0, 40_000, &mut rng).unwrap();
        g.advance(40_000, &mut rng);
        assert_eq!(g.status(), Status::Live);
        assert_eq!(serde_json::to_string(&g.wall_view()).unwrap(), quiet);

        // Nothing a link or a setting could be read from, at any stage.
        let mut setup = Game::new(0, &mut rng);
        setup.settings.false_alarm_percent_per_minute = [0.37, 0.41];
        lobby::admit(&mut setup, "Ada", None, 0).unwrap();
        lobby::admit(&mut setup, "Bo", None, 0).unwrap();
        for game in [&g, &setup] {
            let wall = serde_json::to_string(&game.wall_view()).unwrap();
            assert!(
                !wall.contains("false_alarm") && !wall.contains("0.37"),
                "{wall}"
            );
            let mut secrets: Vec<&str> = vec![&game.key, &game.lobby.join, &game.lobby.wall];
            secrets.extend(game.lobby.players.iter().map(|p| p.key.as_str()));
            for c in &game.countries {
                secrets.extend(c.key.as_deref());
                secrets.extend(c.watch_key.as_deref());
            }
            for secret in secrets {
                assert!(!wall.contains(secret), "{wall} carries {secret}");
            }
        }
        let before = setup.wall_view();
        let countries = before.countries.as_ref().unwrap();
        assert_eq!(countries[0].people, ["Ada"]);
        assert_eq!(countries[1].people, ["Bo"]);
        assert!(before.reveal.is_none() && before.live_at.is_none());

        // The reveal names countries, never a person.
        g.advance(30_000 + FLIGHT, &mut rng);
        assert_eq!(g.status(), Status::Over);
        let after = serde_json::to_string(&g.wall_view()).unwrap();
        assert!(
            after.contains("B retaliated against a false alarm and destroyed A."),
            "{after}"
        );
        assert!(!after.contains("Ada") && !after.contains(&ada), "{after}");
    }
}
