# Petrov Day Ritual App – requirements

a web app for two groups ("countries") to hold a Petrov Day ritual. each country can
launch one missile at the other, and detection systems occasionally raise false alarms.
it is a solemn ritual, not a game: no score, no winner. "nothing happened" is the good
outcome.

## core user flow

### setup
- anyone creates a game and gets a game link containing a random, unguessable part.
  many games can run at once, fully isolated from each other.
- settings, editable by anyone with the game link until the game is live. countries see
  changes live:
  - the names of the countries
  - False-alarm rate per country (default 0.02% per minute, about 1.2%/hour).
    The setup screen asks for the expected duration of the evening (used only for this
    estimate) and shows the resulting chance of at least one false alarm per country.
  - missile flight time, which is also the warning window (default 5 min).
  - each country's consequence: free text describing what that country does when it is hit
    (e.g. "We destroy our cake").
- the game page offers one claim button per country, and each country can be claimed only
  once. claiming takes the claimer (the country's host) to a country link to share with their
  citizens (a QR code is nice to have). anyone holding the country link is that country, on any number of devices,
  and all of them show the same live state. the host has no special powers: any device can
  press Start, launch or request the end.
- each country also gets a watch-only link. it shows the same live state, alarms included,
  but cannot press Start, launch or request the end. the country decides who gets which link;
  how it decides to launch stays up to its own rules, not the app.
- both consequences are visible to both countries.
- no accounts and no invitations. the host shares the links themselves.

### Pre-flight check
- each device can run a check on its own before the game: siren audio, screen stays awake,
  fullscreen, vibration, connection. the implementer may build it as a PWA if that helps.
- the app tells users to keep at least one "command center" device per country
  open and on screen for the whole ritual.

### live game
- before start, each country sees whether the other has claimed and pressed Start.
- the game is **live** once both countries have pressed Start. settings are then locked.
- every device needs a user tap to unlock audio (browser autoplay rules). a device that has
  not been unlocked shows a prominent prompt.
- while live, a country sees nothing about the other country (no presence,
  device counts or activity). the game link shows only the status (not started / live / over).
- **false alarms:** random per country at the configured rate. start times must not fall on
  a detectable grid (e.g. whole minutes after start), since real launches can happen at any
  second. alarms occur only while the game is live, no end has been requested and the country
  is not destroyed. false alarms may overlap with each other and with real missiles.
- **launch:** each country has exactly **one** missile. the country has to
  coordinate themselves who is allowed to launch, what the chain of command is
  etc. technically, the missile can only be launched after a deliberate
  confirmation (e.g. hold 3 s or type "LAUNCH"). there is no recall and no
  defence. the launching country sees its outgoing missile with the countdown, alongside any
  incoming warnings.
- **warning:** the target gets a prominent alarm with a looping siren, vibration where
  supported and a countdown equal to the flight time. any device can acknowledge it, which
  silences siren and vibration on that device; each new warning sounds again. the warning and
  its countdown stay visible until the window ends. it never blocks the rest of the UI, so the
  country can still launch or request the end. real and false
  warnings are **indistinguishable**, including in client code, timing and network traffic:
  the server never sends a "false" flag before the window ends. multiple simultaneous warnings
  are each shown separately.
- **window ends:**
  - real missile: impact. the hit country sees its consequence full-screen and carries it out
    on the honour system; the launching country is told of the impact. The hit country is
    **destroyed**: it can no longer launch and gets no more false alarms. A missile it launched
    earlier still hits.
  - false alarm: "no impact – detection system malfunction". only that country is told.
- the loss of connection to the server shows a loud "CONNECTION LOST" warning.
- all timing is server-authoritative, so devices that reconnect resume the correct state.

### ending
- the end is only ever **requested**, by either country, and the request is always accepted without
  refusal (a refusal would leak information). it cannot be withdrawn.
- a request ends nothing by itself. until both countries get the final "game over"
  confirmation, the game stays fully in effect for both: missiles in flight still hit, the
  other country may still retaliate, and every impact destroys its target and its consequence
  MUST be carried out. worst case scenario a country will have to wait two
  times the missile flight time until their evening is really over.
- after requesting the end, a country may launch only while it has an active incoming warning
  (real or false). its own missile limit still applies.
- the end takes effect as soon as nothing (real or false) is in flight, no matter when it was
  launched. retaliation can therefore extend the time until the end.
- the other country is **not** told about the end request while anything is in flight. the
  requesting country sees "waiting for the world to settle…".
- exception: the game ends automatically once an impact has happened and nothing else is in
  flight.

### reveal
- when the game is over, both countries see the same full timeline: start, every false alarm
  (country and time), every launch (who, when, and whether a warning was on screen at the time),
  impacts and end requests (who, when).
- a headline outcome, e.g. "peace held", "B was destroyed", "Mutual destruction", "A retaliated
  against a false alarm and destroyed B".
- the reveal stays read-only via the game and country links. games are deleted ~30 days after
  they end.

## around the ritual

- **group board** (`/groups`): a public list where a group can offer itself as the second
  country — name, description, rough time, location or time zone, contact. anyone can list a
  group, and a password chosen when listing is the only thing needed to edit or delete it
  later. still no accounts. the board is cleared every year once the day is over everywhere
  on Earth (midnight at UTC-12).
- **info page** (`/info`): links to what other people do on the day, led by the LessWrong
  ceremony.
- **brands:** one deployment serves Petrov Day (26 September), another Arkhipov Day
  (27 October), from the same app. a branded page never mentions the other day.

## edge cases
- **intentionally low false-alarm rate:** at the default rate, a warning is usually real, so
  retaliating looks rational. that is the Petrov dilemma and is deliberate; don't "fix" it.
- **several warnings at once:** all are shown, even though (with one missile per country)
  this reveals that at least one is false.
- **request the end, then attack:** not possible. after an end request a country may only retaliate.
- **requesting country learns something is in flight:** if A requests the end while B has a
  false alarm, A sees the wait. this is accepted.
- **country has used its missile but isn't destroyed:** it still gets false alarms and cannot
  respond. it can still request the end.
- **if you lost your country link:** set up a new game.
- **abandoned game** (live or not started, no end): deleted without a reveal after ~24 h with no
  device connected.
- **mobile browsers** may not deliver alarms on locked or backgrounded phones. the mitigation is the
  command-center device plus the pre-flight check. how reliable this is on phones is **[NEEDS PROTOTYPE]**.

## explicit non-goals
- more than two countries.
- scoring, winners, multiple rounds, cross-game history and stats.
- missile recall, interception and defence.
- a facilitator role or any human control over false alarms.
- automatic enforcement of consequences (e.g. ending a video call).
- accounts, email/invitations, native apps, guaranteed push notifications.
- preventing a determined participant from getting access to both countries (honour system).
