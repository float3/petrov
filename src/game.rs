use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};

pub const MAX_NAME: usize = 1000;
pub const MAX_CONSEQUENCE: usize = 20_000;
const MAX_PERCENT_PER_MINUTE: f64 = 10_000.0;
const MAX_FLIGHT_SECS: u64 = 365 * 24 * 60 * 60;
pub const KEY_LEN: usize = 24;

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
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Country {
    pub key: Option<String>,
    pub started: bool,
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
pub struct GameView {
    pub status: Status,
    pub setup: Option<GameSetup>,
    pub reveal: Option<Reveal>,
}

impl Game {
    pub fn new(now: u64, rng: &mut impl Rng) -> Game {
        let country = || Country {
            key: None,
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
                if self.end_requested_by_anyone(t) || self.destroyed_at(country, t).is_some() {
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
        Ok(key)
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

    pub fn country_view(&self, country: usize, now: u64) -> CountryView {
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
            can_launch: self.can_launch(country, now).is_ok(),
            reveal: (status == Status::Over).then(|| self.reveal()),
        }
    }

    pub fn game_view(&self) -> GameView {
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

    fn headline(&self) -> String {
        let names = self.names();
        let mut launches: Vec<&Missile> = self.missiles.iter().collect();
        launches.sort_by_key(|m| m.launched);
        match launches.as_slice() {
            [] => "Peace held.".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

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
        assert!(g.game_view().setup.unwrap().countries[1].claimed);
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
                without_ids(&real.country_view(1, t)),
                without_ids(&fake.country_view(1, t))
            );
        }

        let t = 50_000 + FLIGHT;
        real.advance(t, &mut rng);
        fake.advance(t, &mut rng);
        let hit = real.country_view(1, t);
        let spared = fake.country_view(1, t);
        assert_eq!(hit.warnings[0].result, Some(WarningResult::Impact));
        assert_eq!(hit.destroyed, Some(t));
        assert_eq!(spared.warnings[0].result, Some(WarningResult::Malfunction));
        assert_eq!(spared.destroyed, None);
        assert!(fake.country_view(0, t).warnings.is_empty());
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
        assert!(g.country_view(0, 10_000 + 2 * FLIGHT).destroyed.is_some());
        assert!(g.country_view(1, 10_000 + 2 * FLIGHT).destroyed.is_some());
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
    fn end_request_waits_for_flight_and_stays_hidden_from_the_other_country() {
        let (mut g, mut rng) = live_game([0.0, 0.0]);
        inject_false_alarm(&mut g, 1, 10_000);
        let before = serde_json::to_value(g.country_view(1, 20_000)).unwrap();
        g.request_end(0, 20_000, &mut rng).unwrap();
        assert_eq!(g.status(), Status::Live);
        assert_eq!(
            serde_json::to_value(g.country_view(1, 20_000)).unwrap(),
            before
        );
        assert_eq!(g.country_view(0, 20_000).end_requested, Some(20_000));

        assert!(g.launch(0, 30_000, &mut rng).is_err());
        g.launch(1, 30_000, &mut rng).unwrap();
        assert!(g.country_view(0, 30_000).can_launch);
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
    fn countries_are_never_told_about_false_alarm_rates() {
        let (g, _) = live_game([0.5, 0.5]);
        let mut rng = rng();
        let setup = Game::new(0, &mut rng);
        for json in [
            serde_json::to_string(&g.country_view(0, 5_000)).unwrap(),
            serde_json::to_string(&setup.country_view(1, 5_000)).unwrap(),
        ] {
            assert!(!json.contains("false_alarm"), "{json}");
        }
    }

    #[test]
    fn live_views_hide_the_other_country() {
        let (g, _) = live_game([0.0, 0.0]);
        assert!(g.country_view(0, 5_000).countries.is_none());
        let view = g.game_view();
        assert!(view.setup.is_none() && view.reveal.is_none());
    }
}
