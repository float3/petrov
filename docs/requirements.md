# Petrov Day Ritual App – Requirements

A web app for two groups ("countries") to hold a Petrov Day ritual. Each country can
launch one missile at the other, and detection systems occasionally raise false alarms.
It is a solemn ritual, not a game: no score, no winner. "Nothing happened" is the good
outcome.

## Core user flow

### Setup
- Anyone creates a game and gets a game link containing a random, unguessable part.
  Many games can run at once, fully isolated from each other.
- Settings, editable by anyone with the game link until the game is live. Countries see
  changes live:
  - The names of the countries
  - False-alarm rate per country (default 0.02% per minute, about 1.2%/hour).
    The setup screen asks for the expected duration of the evening (used only for this
    estimate) and shows the resulting chance of at least one false alarm per country.
  - Missile flight time, which is also the warning window (default 5 min).
  - Each country's consequence: free text describing what that country does when it is hit
    (e.g. "We destroy our cake").
- The game page offers one claim button per country, and each country can be claimed only
  once. Claiming takes the claimer (the country's host) to a country link to share with their
  citizens (a QR code is nice to have). Anyone holding the country link is that country, on any number of devices,
  and all of them show the same live state. The host has no special powers: any device can
  press Start, launch or request the end.
- Each country also gets a watch-only link. It shows the same live state, alarms included,
  but cannot press Start, launch or request the end. The country decides who gets which link;
  how it decides to launch stays up to its own rules, not the app.
- Both consequences are visible to both countries.
- No accounts and no invitations. The host shares the links themselves.

### Pre-flight check
- Each device can run a check on its own before the game: siren audio, screen stays awake,
  fullscreen, vibration, connection. The implementer may build it as a PWA if that helps.
- The app tells users to keep at least one "command center" device per country
  open and on screen for the whole ritual.

### Live game
- Before start, each country sees whether the other has claimed and pressed Start.
- The game is **live** once both countries have pressed Start. Settings are then locked.
- Every device needs a user tap to unlock audio (browser autoplay rules). A device that has
  not been unlocked shows a prominent prompt.
- While live, a country sees nothing about the other country (no presence,
  device counts or activity). The game link shows only the status (not started / live / over).
- **False alarms:** random per country at the configured rate. Start times must not fall on
  a detectable grid (e.g. whole minutes after start), since real launches can happen at any
  second. Alarms occur only while the game is live, no end has been requested and the country
  is not destroyed. False alarms may overlap with each other and with real missiles.
- **Launch:** each country has exactly **one** missile. The country has to
  coordinate themselves who is allowed to launch, what the chain of command is
  etc. Technically, the missile can only be launched after a deliberate
  confirmation (e.g. hold 3 s or type "LAUNCH"). There is no recall and no
  defence. The launching country sees its outgoing missile with the countdown, alongside any
  incoming warnings.
- **Warning:** the target gets a prominent alarm with a looping siren, vibration where
  supported and a countdown equal to the flight time. Any device can acknowledge it, which
  silences siren and vibration on that device; each new warning sounds again. The warning and
  its countdown stay visible until the window ends. It never blocks the rest of the UI, so the
  country can still launch or request the end. Real and false
  warnings are **indistinguishable**, including in client code, timing and network traffic:
  the server never sends a "false" flag before the window ends. Multiple simultaneous warnings
  are each shown separately.
- **Window ends:**
  - Real missile: impact. The hit country sees its consequence full-screen and carries it out
    on the honour system; the launching country is told of the impact. The hit country is
    **destroyed**: it can no longer launch and gets no more false alarms. A missile it launched
    earlier still hits.
  - False alarm: "No impact – detection system malfunction". Only that country is told.
- The loss of connection to the server shows a loud "CONNECTION LOST" warning.
- All timing is server-authoritative, so devices that reconnect resume the correct state.

### Ending
- The end is only ever **requested**, by either country, and the request is always accepted without
  refusal (a refusal would leak information). It cannot be withdrawn.
- A request ends nothing by itself. Until both countries get the final "game over"
  confirmation, the game stays fully in effect for both: missiles in flight still hit, the
  other country may still retaliate, and every impact destroys its target and its consequence
  MUST be carried out. Worst case scenario a country will have to wait two
  times the missile flight time until their evening is really over.
- After requesting the end, a country may launch only while it has an active incoming warning
  (real or false). Its own missile limit still applies.
- The end takes effect as soon as nothing (real or false) is in flight, no matter when it was
  launched. Retaliation can therefore extend the time until the end.
- The other country is **not** told about the end request while anything is in flight. The
  requesting country sees "waiting for the world to settle…".
- Exception: the game ends automatically once an impact has happened and nothing else is in
  flight.

### Reveal
- When the game is over, both countries see the same full timeline: start, every false alarm
  (country and time), every launch (who, when, and whether a warning was on screen at the time),
  impacts and end requests (who, when).
- A headline outcome, e.g. "Peace held", "B was destroyed", "Mutual destruction", "A retaliated
  against a false alarm and destroyed B".
- The reveal stays read-only via the game and country links. Games are deleted ~30 days after
  they end.

## Around the ritual

- **Group board** (`/groups`): a public list where a group can offer itself as the second
  country — name, description, rough time, location or time zone, contact. Anyone can list a
  group, and a password chosen when listing is the only thing needed to edit or delete it
  later. Still no accounts. The board is cleared every year once the day is over everywhere
  on Earth (midnight at UTC-12).
- **Info page** (`/info`): links to what other people do on the day, led by the LessWrong
  ceremony.
- **Brands:** one deployment serves Petrov Day (26 September), another Arkhipov Day
  (27 October), from the same app. A branded page never mentions the other day.

## Edge cases
- **Intentionally low false-alarm rate:** at the default rate, a warning is usually real, so
  retaliating looks rational. That is the Petrov dilemma and is deliberate; don't "fix" it.
- **Several warnings at once:** all are shown, even though (with one missile per country)
  this reveals that at least one is false.
- **Request the end, then attack:** not possible. After an end request a country may only retaliate.
- **Requesting country learns something is in flight:** if A requests the end while B has a
  false alarm, A sees the wait. This is accepted.
- **Country has used its missile but isn't destroyed:** it still gets false alarms and cannot
  respond. It can still request the end.
- **If you lost your country link:** set up a new game.
- **Abandoned game** (live or not started, no end): deleted without a reveal after ~24 h with no
  device connected.
- **Mobile browsers** may not deliver alarms on locked or backgrounded phones. The mitigation is the
  command-center device plus the pre-flight check. How reliable this is on phones is **[NEEDS PROTOTYPE]**.

## Explicit non-goals
- More than two countries.
- Scoring, winners, multiple rounds, cross-game history and stats.
- Missile recall, interception and defence.
- A facilitator role or any human control over false alarms.
- Automatic enforcement of consequences (e.g. ending a video call).
- Accounts, email/invitations, native apps, guaranteed push notifications.
- Preventing a determined participant from getting access to both countries (honour system).
