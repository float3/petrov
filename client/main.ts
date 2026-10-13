// Petrov Day, in a browser.
//
// Five screens, one page: the home page, the preflight check, the list of
// groups playing, the game's setup screen, and the one that matters — a
// country's command centre, which has to wake a room up when a warning
// comes in and has to be trusted to still be connected when none does.

import {
  api,
  byId,
  copyTo,
  countdown,
  el,
  memo,
  persist,
  qrInto,
  stored,
  subscribe,
  timeOfDay,
  type Feed,
} from "./rooms";

/** How long the launch button has to be held. */
const HOLD_MS = 3000;

type Status = "setup" | "live" | "over";

interface Settings {
  names: [string, string];
  consequences: [string, string];
  false_alarm_percent_per_minute: [number, number];
  flight_secs: number;
  expected_minutes: number;
}

interface CountrySettings {
  consequences: [string, string];
  flight_secs: number;
}

interface WarningView {
  id: string;
  detected: number;
  ends: number;
  result: "impact" | "malfunction" | null;
}

interface OutgoingView {
  launched: number;
  impact: number;
}

interface CountryStatus {
  name: string;
  claimed: boolean;
  started: boolean;
}

interface TimelineEntry {
  t: number;
  text: string;
}

interface Reveal {
  headline: string;
  timeline: TimelineEntry[];
}

interface CountryView {
  status: Status;
  me: number;
  names: [string, string];
  settings: CountrySettings;
  countries: [CountryStatus, CountryStatus] | null;
  live_at: number | null;
  destroyed: number | null;
  outgoing: OutgoingView | null;
  warnings: WarningView[];
  end_requested: number | null;
  can_launch: boolean;
  reveal: Reveal | null;
  watch_only: boolean;
  watch_key: string | null;
}

interface ClaimSlot {
  name: string;
  claimed: boolean;
  started: boolean;
}

interface GameSetup {
  names: [string, string];
  settings: Settings;
  countries: [ClaimSlot, ClaimSlot];
}

interface GameView {
  status: Status;
  setup: GameSetup | null;
  reveal: Reveal | null;
}

interface PublicGroup {
  id: string;
  name: string;
  description: string;
  when: string;
  location: string;
  contact: string;
  created: number;
  updated: number;
}

const $ = byId;

/** The chance of at least one false alarm, at that rate, over so long. */
function falseAlarmChance(percentPerMinute: number, minutes: number): number {
  return 1 - Math.exp(-(percentPerMinute / 100) * minutes);
}

function percent(p: number): string {
  const value = p * 100;
  return value >= 10
    ? `${value.toFixed(0)}%`
    : value >= 1
      ? `${value.toFixed(1)}%`
      : `${value.toFixed(2)}%`;
}

// ------------------------------------------------------------------ sound
//
// A command centre has to be able to wake a room that is not looking at
// it. Browsers will not make a sound until the page has been touched, so
// the first touch anywhere unlocks the audio and asks to keep the screen
// on.

interface Siren {
  osc: OscillatorNode;
  lfo: OscillatorNode;
}

const sound = {
  ctx: null as AudioContext | null,
  siren: null as Siren | null,

  unlock(): void {
    try {
      this.ctx ??= new AudioContext();
      if (this.ctx.state !== "running") {
        void this.ctx.resume();
        const source = this.ctx.createBufferSource();
        source.buffer = this.ctx.createBuffer(1, 1, 22050);
        source.connect(this.ctx.destination);
        source.start();
      }
    } catch {
      /* a browser with no audio at all; the page says so in the check */
    }
    void keepAwake();
  },

  unlocked(): boolean {
    return !!this.ctx && this.ctx.state === "running";
  },

  alarm(on: boolean): void {
    const ctx = this.ctx;
    if (!ctx) return;
    if (on && !this.siren) {
      const osc = ctx.createOscillator();
      const lfo = ctx.createOscillator();
      const depth = ctx.createGain();
      const gain = ctx.createGain();
      osc.type = "sawtooth";
      osc.frequency.value = 820;
      lfo.frequency.value = 0.9;
      depth.gain.value = 340;
      gain.gain.value = 0.25;
      lfo.connect(depth).connect(osc.frequency);
      osc.connect(gain).connect(ctx.destination);
      osc.start();
      lfo.start();
      this.siren = { osc, lfo };
    } else if (!on && this.siren) {
      this.siren.osc.stop();
      this.siren.lfo.stop();
      this.siren = null;
    }
  },

  beep(): void {
    const ctx = this.ctx;
    if (!ctx) return;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "square";
    osc.frequency.value = 440;
    gain.gain.setValueAtTime(0.2, ctx.currentTime);
    gain.gain.setValueAtTime(0, ctx.currentTime + 0.25);
    osc.connect(gain).connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.3);
  },

  /** The sound of being hit. */
  boom(): void {
    const ctx = this.ctx;
    if (!ctx) return;
    const length = ctx.sampleRate * 3;
    const buffer = ctx.createBuffer(1, length, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < length; i++) {
      data[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / length, 3);
    }
    const source = ctx.createBufferSource();
    const filter = ctx.createBiquadFilter();
    source.buffer = buffer;
    filter.type = "lowpass";
    filter.frequency.value = 500;
    source.connect(filter).connect(ctx.destination);
    source.start();
  },
};

document.addEventListener("pointerdown", () => sound.unlock(), {
  capture: true,
});

let wakeLock: WakeLockSentinel | null = null;

async function keepAwake(): Promise<boolean> {
  try {
    if (
      "wakeLock" in navigator &&
      document.visibilityState === "visible" &&
      !wakeLock
    ) {
      wakeLock = await navigator.wakeLock.request("screen");
      wakeLock.addEventListener("release", () => {
        wakeLock = null;
      });
    }
  } catch {
    /* not supported, or refused; the check says so */
  }
  return !!wakeLock;
}

document.addEventListener("visibilitychange", () => void keepAwake());

// -------------------------------------------------------------- the parts

function renderSettingsSummary(
  container: HTMLElement,
  names: [string, string],
  settings: CountrySettings,
): void {
  container.replaceChildren(
    el(
      "div",
      { className: "panel consequences" },
      el("h3", { textContent: "Settings" }),
      el("p", {
        textContent: `Missile flight time and warning window: ${countdown(settings.flight_secs * 1000)}`,
      }),
      ...[0, 1].map((c) =>
        el(
          "p",
          {},
          el("strong", { textContent: `${names[c]} when hit: ` }),
          settings.consequences[c].trim() || "(no consequence set)",
        ),
      ),
    ),
  );
}

function renderReveal(
  container: HTMLElement,
  reveal: Reveal,
  names?: [string, string],
  settings?: CountrySettings,
): void {
  container.replaceChildren(
    el(
      "div",
      { className: "reveal" },
      el("small", { textContent: "Game over" }),
      el("h2", { textContent: reveal.headline }),
      el(
        "ol",
        { className: "plain timeline" },
        ...reveal.timeline.map((entry) =>
          el(
            "li",
            {},
            el("time", { textContent: timeOfDay(entry.t) }),
            el("span", { textContent: entry.text }),
          ),
        ),
      ),
      names && settings
        ? el(
            "div",
            { className: "panel consequences" },
            el("h3", { textContent: "Consequences" }),
            ...[0, 1].map((c) =>
              el(
                "p",
                {},
                el("strong", { textContent: `${names[c]}: ` }),
                settings.consequences[c].trim() || "(none)",
              ),
            ),
          )
        : null,
    ),
  );
}

interface CheckHandle {
  set: (kind: string, said: string) => void;
  ask: (question: string, yes: () => void, no: () => void) => void;
}

/**
 * The checks a command centre runs before the ritual: that this device
 * can be heard, will stay awake, and is talking to the server.
 */
function preflight(
  container: HTMLElement,
  connectionCheck?: () => string,
): void {
  const rows: HTMLElement[] = [];

  function row(name: string, run: (handle: CheckHandle) => void): void {
    const result = el("div", {
      className: "result muted",
      textContent: "Not checked yet.",
    });
    const buttons = el("div", { className: "buttons" });
    const node = el(
      "div",
      { className: "check" },
      el("strong", { textContent: name }),
      buttons,
      result,
    );
    const set = (kind: string, said: string) => {
      result.className = `result ${kind}`;
      result.textContent = said;
    };
    const reset = () =>
      buttons.replaceChildren(
        el("button", {
          textContent: "Test",
          onclick: () => run({ set, ask }),
        }),
      );
    const ask = (question: string, yes: () => void, no: () => void) => {
      buttons.replaceChildren(
        el("button", {
          textContent: "Yes",
          onclick: () => {
            reset();
            yes();
          },
        }),
        el("button", {
          textContent: "No",
          onclick: () => {
            reset();
            no();
          },
        }),
      );
      set("warn", question);
    };
    reset();
    rows.push(node);
  }

  row("Siren", ({ set, ask }) => {
    sound.unlock();
    if (!sound.ctx) {
      set("bad", "This browser cannot play audio.");
      return;
    }
    sound.alarm(true);
    setTimeout(() => {
      sound.alarm(false);
      ask(
        "Did you hear the siren?",
        () => set("ok", "Siren works."),
        () =>
          set(
            "bad",
            "Turn up the volume and check the silent switch. On iPhones the ring/silent switch mutes web audio.",
          ),
      );
    }, 2500);
  });

  row("Screen stays awake", ({ set }) => {
    void keepAwake().then((held) => {
      if (held) {
        set("ok", "The screen will stay awake while this page is visible.");
      } else {
        set(
          "warn",
          "Not supported here. Set the screen timeout to never and keep the device plugged in.",
        );
      }
    });
  });

  row("Fullscreen", ({ set }) => {
    const standalone = matchMedia("(display-mode: standalone)").matches;
    document.documentElement
      .requestFullscreen()
      .then(() => set("ok", "Fullscreen works. Press Escape or swipe to leave it."))
      .catch(() => {
        if (standalone) set("ok", "Running as an installed app.");
        else
          set(
            "warn",
            "Not supported here. On an iPhone, use Share → Add to Home Screen and open the page from there.",
          );
      });
  });

  row("Vibration", ({ set, ask }) => {
    if (!navigator.vibrate || !navigator.vibrate([400, 150, 400, 150, 400])) {
      set(
        "warn",
        "Not supported on this device (iPhones never vibrate for web pages). Rely on the siren.",
      );
      return;
    }
    setTimeout(
      () =>
        ask(
          "Did the device vibrate?",
          () => set("ok", "Vibration works."),
          () => set("warn", "No vibration. Rely on the siren."),
        ),
      1500,
    );
  });

  row("Connection", ({ set }) => {
    const started = performance.now();
    api<{ now: number }>("GET", "/api/time")
      .then(({ now }) => {
        const rtt = performance.now() - started;
        const skew = Math.abs(now - Date.now()) / 1000;
        const extra = connectionCheck ? ` ${connectionCheck()}` : "";
        set(
          "ok",
          `Server reachable in ${Math.round(rtt)} ms. This device's clock is ${skew.toFixed(1)} s off, which does not matter: all timing comes from the server.${extra}`,
        );
      })
      .catch(() => set("bad", "Cannot reach the server. Check the network."));
  });

  container.replaceChildren(
    el("p", {
      className: "hint",
      textContent:
        "Keep at least one command center device per country open, on screen, unlocked and plugged in for the whole ritual. Phones may not sound alarms while locked or in the background.",
    }),
    ...rows,
  );
}

// -------------------------------------------------------------- the pages

function groupsPage(): void {
  $("groups").hidden = false;
  let groups: PublicGroup[] = [];
  let editing: PublicGroup | null = null;
  const fields: Record<string, string> = {
    name: "groupName",
    description: "groupDescription",
    when: "groupWhen",
    location: "groupLocation",
    contact: "groupContact",
  };
  const field = (id: string) => byId<HTMLInputElement>(id);

  function contactNode(contact: string): Node {
    if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(contact)) {
      return el("a", { href: `mailto:${contact}`, textContent: contact });
    }
    if (/^https:\/\/\S+$/.test(contact)) {
      return el("a", {
        href: contact,
        textContent: contact,
        rel: "nofollow noopener",
        target: "_blank",
      });
    }
    return document.createTextNode(contact);
  }

  function openForm(group: PublicGroup | null): void {
    editing = group;
    $("groupForm").hidden = false;
    $("groupNew").hidden = true;
    $("groupFormTitle").textContent = group
      ? `Edit ${group.name}`
      : "List your group";
    $("groupSubmit").textContent = group ? "Save changes" : "Publish";
    $("groupPasswordHint").textContent = group
      ? "Enter the password you chose when you listed the group."
      : "You need this password to edit or delete your listing later. It cannot be recovered.";
    field("groupPassword").autocomplete = group
      ? "current-password"
      : "new-password";
    for (const [key, id] of Object.entries(fields)) {
      field(id).value = group ? String(group[key as keyof PublicGroup]) : "";
    }
    field("groupPassword").value = "";
    $("groupError").textContent = "";
    $("groupForm").scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function closeForm(): void {
    editing = null;
    $("groupForm").hidden = true;
    $("groupNew").hidden = false;
  }

  function render(): void {
    $("groupEmpty").hidden = groups.length > 0;
    $("groupList").replaceChildren(
      ...groups.map((group) => {
        const manage = el("div", { className: "manage" });
        const resetManage = () =>
          manage.replaceChildren(
            el("button", {
              type: "button",
              textContent: "Edit",
              onclick: () => openForm(group),
            }),
            el("button", {
              type: "button",
              textContent: "Delete",
              onclick: askDelete,
            }),
          );
        function askDelete(): void {
          const password = el("input", {
            type: "password",
            placeholder: "Password",
            autocomplete: "current-password",
          });
          const error = el("span", { className: "error" });
          const confirm = el("button", {
            type: "button",
            className: "danger",
            textContent: "Delete listing",
          });
          confirm.onclick = () => {
            confirm.disabled = true;
            api("DELETE", `/api/groups/${group.id}`, {
              password: password.value,
            })
              .then(() => {
                groups = groups.filter((other) => other.id !== group.id);
                render();
              })
              .catch((e: unknown) => {
                error.textContent = e instanceof Error ? e.message : String(e);
                confirm.disabled = false;
              });
          };
          manage.replaceChildren(
            password,
            confirm,
            el("button", {
              type: "button",
              textContent: "Cancel",
              onclick: resetManage,
            }),
            error,
          );
          password.focus();
        }
        resetManage();
        const listed = new Date(group.created).toLocaleDateString();
        const updated =
          group.updated !== group.created
            ? `, updated ${new Date(group.updated).toLocaleDateString()}`
            : "";
        return el(
          "article",
          { className: "group" },
          el("h3", { textContent: group.name || "Unnamed group" }),
          group.when || group.location
            ? el("div", {
                className: "meta",
                textContent: [group.when, group.location]
                  .filter(Boolean)
                  .join(" · "),
              })
            : null,
          group.description
            ? el("p", { className: "description", textContent: group.description })
            : null,
          group.contact
            ? el("p", { className: "contact" }, "Contact: ", contactNode(group.contact))
            : null,
          el("div", {
            className: "meta",
            textContent: `Listed ${listed}${updated}`,
          }),
          manage,
        );
      }),
    );
  }

  byId<HTMLButtonElement>("groupNew").onclick = () => openForm(null);
  byId<HTMLButtonElement>("groupCancel").onclick = closeForm;
  $("groupForm").addEventListener("submit", (event) => {
    event.preventDefault();
    const body: Record<string, string> = {
      password: field("groupPassword").value,
    };
    for (const [key, id] of Object.entries(fields)) body[key] = field(id).value;
    const submit = byId<HTMLButtonElement>("groupSubmit");
    submit.disabled = true;
    $("groupError").textContent = "";
    api<PublicGroup>(
      editing ? "PUT" : "POST",
      editing ? `/api/groups/${editing.id}` : "/api/groups",
      body,
    )
      .then((saved) => {
        groups = [saved, ...groups.filter((group) => group.id !== saved.id)];
        closeForm();
        render();
      })
      .catch((e: unknown) => {
        $("groupError").textContent = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        submit.disabled = false;
      });
  });

  api<PublicGroup[]>("GET", "/api/groups")
    .then((loaded) => {
      groups = loaded;
      render();
    })
    .catch((e: unknown) => {
      $("groupList").replaceChildren(
        el("p", {
          className: "error",
          textContent: `Could not load groups: ${e instanceof Error ? e.message : String(e)}`,
        }),
      );
    });
}

function homePage(): void {
  $("home").hidden = false;
  const button = byId<HTMLButtonElement>("createGame");
  button.addEventListener("click", () => {
    button.disabled = true;
    api<{ game: string }>("POST", "/api/games")
      .then(({ game }) => {
        location.href = `/g/${game}`;
      })
      .catch((e: unknown) => {
        $("createError").textContent = e instanceof Error ? e.message : String(e);
        button.disabled = false;
      });
  });
}

function checkPage(): void {
  $("checkPage").hidden = false;
  preflight($("checkPageBody"));
}

function gamePage(key: string): void {
  $("game").hidden = false;
  let view: GameView | null = null;
  let dirty = false;
  let saving = false;
  let debounce: number | undefined;
  const inputs = Array.from(
    $("settingsForm").querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
      "input, textarea",
    ),
  );
  const field = (id: string) => byId<HTMLInputElement>(id);

  function formSettings(): Settings {
    const settings = structuredClone(
      (view as GameView & { setup: GameSetup }).setup.settings,
    );
    for (const c of [0, 1]) {
      settings.names[c] = field(`name${c}`).value;
      settings.consequences[c] = field(`cons${c}`).value;
      const rate = Number(field(`rate${c}`).value);
      settings.false_alarm_percent_per_minute[c] = Number.isFinite(rate)
        ? Math.max(0, rate)
        : 0;
    }
    const flight = Math.round(Number(field("flight").value) * 60);
    if (field("flight").value !== "" && Number.isFinite(flight)) {
      settings.flight_secs = Math.max(1, flight);
    }
    const duration = Math.round(Number(field("duration").value));
    if (field("duration").value !== "" && Number.isFinite(duration)) {
      settings.expected_minutes = Math.max(0, duration);
    }
    return settings;
  }

  function fillForm(settings: Settings): void {
    const values: Record<string, string | number> = {
      name0: settings.names[0],
      name1: settings.names[1],
      cons0: settings.consequences[0],
      cons1: settings.consequences[1],
      rate0: settings.false_alarm_percent_per_minute[0],
      rate1: settings.false_alarm_percent_per_minute[1],
      flight: +(settings.flight_secs / 60).toFixed(3),
      duration: settings.expected_minutes,
    };
    for (const input of inputs) {
      if (input === document.activeElement) continue;
      if (String(input.value) !== String(values[input.id])) {
        input.value = String(values[input.id]);
      }
    }
  }

  function estimates(): void {
    const minutes = Number(field("duration").value) || 0;
    for (const c of [0, 1]) {
      const rate = Number(field(`rate${c}`).value) || 0;
      const name =
        field(`name${c}`).value.trim() || ["Country A", "Country B"][c];
      $(`estimate${c}`).textContent =
        `≈ ${percent(falseAlarmChance(rate, minutes))} chance of at least one false alarm at ${name} over ${minutes} minutes.`;
    }
  }

  function save(): void {
    if (saving) {
      dirty = true;
      return;
    }
    saving = true;
    dirty = false;
    $("saveStatus").textContent = "Saving…";
    api<{ now: number; view: GameView }>(
      "PUT",
      `/api/g/${key}/settings`,
      formSettings(),
    )
      .then((envelope) => {
        feed.accept(envelope);
        $("saveStatus").textContent =
          "Saved. Both countries see the change live and have to press Start again.";
        $("saveStatus").className = "hint";
      })
      .catch((e: unknown) => {
        $("saveStatus").textContent = e instanceof Error ? e.message : String(e);
        $("saveStatus").className = "error";
      })
      .finally(() => {
        saving = false;
        if (dirty) save();
      });
  }

  $("settingsForm").addEventListener("input", () => {
    estimates();
    dirty = true;
    clearTimeout(debounce);
    debounce = setTimeout(save, 600);
  });
  $("settingsForm").addEventListener("submit", (event) => event.preventDefault());

  const claimCards = [0, 1].map(() => {
    const card = el("div", { className: "panel" });
    $("claims").append(card);
    return { card, state: "" };
  });

  function claimCountry(c: number, button: HTMLButtonElement, error: HTMLElement): void {
    button.disabled = true;
    api<{ country: string }>("POST", `/api/g/${key}/claim/${c}`)
      .then(({ country }) => {
        location.href = `/k/${country}`;
      })
      .catch((e: unknown) => {
        error.textContent = e instanceof Error ? e.message : String(e);
        button.disabled = false;
      });
  }

  function renderClaims(setup: GameSetup): void {
    setup.countries.forEach((slot, c) => {
      const entry = claimCards[c];
      const state = `${slot.name}|${slot.claimed}|${slot.started}`;
      if (entry.state === state) return;
      entry.state = state;
      if (!slot.claimed) {
        const error = el("p", { className: "error" });
        const button = el("button", {
          className: "primary",
          textContent: `Claim ${slot.name}`,
        });
        button.onclick = () => claimCountry(c, button, error);
        entry.card.replaceChildren(
          el("h3", { textContent: slot.name }),
          el("p", {
            className: "hint",
            textContent: `Only the host of ${slot.name} should press this. It takes you to your country link, which you then share with your citizens.`,
          }),
          button,
          error,
        );
      } else {
        entry.card.replaceChildren(
          el("h3", { textContent: slot.name }),
          el("p", {
            className: "ok",
            textContent: slot.started
              ? "Claimed · pressed Start"
              : "Claimed · not started",
          }),
        );
      }
    });
  }

  const feed: Feed = subscribe<GameView>(
    `/api/g/${key}`,
    `/api/g/${key}/events`,
    (next) => {
      view = next;
      $("gameStatus").textContent = {
        setup: "not started",
        live: "live",
        over: "over",
      }[next.status];
      $("gameSetup").hidden = next.status !== "setup";
      $("gameLive").hidden = next.status !== "live";
      $("gameReveal").hidden = next.status !== "over";
      if (next.setup) {
        if (!dirty && !saving) fillForm(next.setup.settings);
        renderClaims(next.setup);
        estimates();
      }
      if (next.reveal) renderReveal($("gameReveal"), next.reveal);
    },
    () => {
      $("gameMissing").hidden = false;
    },
  );
}

/** The acknowledge button, in the middle of the card. */
function centred(child: HTMLElement): HTMLElement {
  const row = el("div", { className: "actions" }, child);
  row.style.justifyContent = "center";
  return row;
}

interface WarningCard {
  node: HTMLElement;
  count: HTMLElement;
  label: HTMLElement;
  title: HTMLElement;
  detail: HTMLElement;
  ack: HTMLButtonElement;
}

interface Notice {
  text: string;
  until: number;
  error?: boolean;
}

function countryPage(key: string): void {
  $("country").hidden = false;
  let view: CountryView | null = null;
  let version = 0;
  let armed = false;
  let disarmTimer: number | undefined;
  let holding: { frame: number } | null = null;
  let notice: Notice | null = null;
  let confirmingEnd = false;
  let hiddenSince: number | null = null;
  let hitClosed = stored(`hit-closed-${key}`, false);
  const acked = new Set(stored<string[]>(`acked-${key}`, []));
  const seen = { first: true, destroyed: null as number | null };
  const warningCards = new Map<string, WarningCard>();

  const url = `${location.origin}/k/${key}`;
  const link = byId<HTMLAnchorElement>("countryLink");
  link.href = url;
  link.textContent = url;
  byId<HTMLButtonElement>("copyCountryLink").onclick = () =>
    void copyTo($("copyCountryLink"), url);
  let watchUrl: string | null = null;
  byId<HTMLButtonElement>("copyWatchLink").onclick = () => {
    if (watchUrl) void copyTo($("copyWatchLink"), watchUrl);
  };

  function shareQrs(): void {
    const box = byId<HTMLDetailsElement>("shareBox");
    if (!box.open || !view) return;
    if (!view.watch_only && !$("countryQr").firstChild) qrInto("countryQr", url);
    if (watchUrl && !$("watchQr").firstChild) qrInto("watchQr", watchUrl);
  }
  $("shareBox").addEventListener("toggle", shareQrs);
  preflight($("checkBoxBody"), () =>
    feed.connected()
      ? "Live updates are connected."
      : "Live updates are NOT connected.",
  );

  byId<HTMLButtonElement>("unlockBanner").onclick = () => {
    sound.unlock();
    render();
  };
  byId<HTMLButtonElement>("hitClose").onclick = () => {
    hitClosed = true;
    persist(`hit-closed-${key}`, true);
    render();
  };

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      hiddenSince = Date.now();
    } else if (hiddenSince && view && view.status === "live") {
      $("backgroundBanner").textContent =
        `This device was in the background for ${countdown(Date.now() - hiddenSince)}. Alarms may not have sounded during that time. Tap to dismiss.`;
      $("backgroundBanner").hidden = false;
      hiddenSince = null;
    }
  });
  byId<HTMLButtonElement>("backgroundBanner").onclick = () => {
    $("backgroundBanner").hidden = true;
  };

  const feed: Feed = subscribe<CountryView>(
    `/api/k/${key}`,
    `/api/k/${key}/events`,
    (next) => {
      view = next;
      version++;
      if (!watchUrl && next.watch_key) {
        watchUrl = `${location.origin}/k/${next.watch_key}`;
        const watch = byId<HTMLAnchorElement>("watchLink");
        watch.href = watchUrl;
        watch.textContent = watchUrl;
        shareQrs();
      }
      $("watchNotice").hidden = !next.watch_only;
      $("commandShare").hidden = next.watch_only;
      react(next);
      render();
    },
    () => {
      $("countryMissing").hidden = false;
      $("shareBox").hidden = true;
    },
  );

  /** The sound of being hit, the first time it is true. */
  function react(next: CountryView): void {
    if (seen.first) {
      seen.first = false;
      seen.destroyed = next.destroyed;
      return;
    }
    if (next.destroyed && !seen.destroyed) sound.boom();
    seen.destroyed = next.destroyed;
  }

  function post(action: string, pending: string): void {
    notice = { text: pending, until: Date.now() + 10_000 };
    render();
    api<{ now: number; view: CountryView }>("POST", `/api/k/${key}/${action}`)
      .then((envelope) => {
        notice = null;
        feed.accept(envelope);
      })
      .catch((e: unknown) => {
        notice = {
          text: e instanceof Error ? e.message : String(e),
          until: Date.now() + 8_000,
          error: true,
        };
        render();
      });
  }

  byId<HTMLButtonElement>("startButton").onclick = () =>
    post("start", "Starting…");

  byId<HTMLButtonElement>("cover").onclick = () => {
    armed = !armed;
    clearTimeout(disarmTimer);
    if (armed) {
      disarmTimer = setTimeout(() => {
        armed = false;
        render();
      }, 20_000);
    }
    cancelHold();
    render();
  };

  const button = byId<HTMLButtonElement>("launch");
  const fill = () => button.querySelector<HTMLElement>(".fill");

  function cancelHold(): void {
    if (!holding) return;
    cancelAnimationFrame(holding.frame);
    holding = null;
    const bar = fill();
    if (bar) bar.style.height = "0";
  }

  button.addEventListener("pointerdown", (event) => {
    if (button.disabled) return;
    try {
      button.setPointerCapture(event.pointerId);
    } catch {
      /* some browsers refuse; the hold still works */
    }
    const began = performance.now();
    holding = { frame: 0 };
    const tick = () => {
      if (!holding) return;
      const held = Math.min(1, (performance.now() - began) / HOLD_MS);
      const bar = fill();
      if (bar) bar.style.height = `${held * 100}%`;
      if (held >= 1) {
        cancelHold();
        armed = false;
        clearTimeout(disarmTimer);
        post("launch", "Launching…");
      } else {
        holding.frame = requestAnimationFrame(tick);
      }
    };
    holding.frame = requestAnimationFrame(tick);
  });
  for (const event of ["pointerup", "pointercancel", "lostpointercapture"]) {
    button.addEventListener(event, cancelHold);
  }
  button.addEventListener("contextmenu", (event) => event.preventDefault());

  byId<HTMLButtonElement>("endButton").onclick = () => {
    confirmingEnd = true;
    render();
  };
  byId<HTMLButtonElement>("endNo").onclick = () => {
    confirmingEnd = false;
    render();
  };
  byId<HTMLButtonElement>("endYes").onclick = () => {
    confirmingEnd = false;
    post("end", "Requesting the end…");
  };

  function warningCard(warning: WarningView): HTMLElement {
    let card = warningCards.get(warning.id);
    if (!card) {
      const count = el("div", { className: "big" });
      const label = el("div");
      const title = el("h2");
      const detail = el("p", { className: "muted" });
      const ack = el("button", {
        textContent: "Acknowledge (silence this device)",
      });
      ack.onclick = () => {
        acked.add(warning.id);
        persist(`acked-${key}`, [...acked]);
        render();
      };
      card = {
        node: el(
          "div",
          { className: "alert" },
          title,
          count,
          label,
          detail,
          centred(ack),
        ),
        count,
        label,
        title,
        detail,
        ack,
      };
      warningCards.set(warning.id, card);
    }
    const now = feed.now();
    const done = warning.result !== null;
    card.node.classList.toggle("done", done);
    card.node.classList.toggle("acked", acked.has(warning.id));
    card.title.textContent = done
      ? warning.result === "impact"
        ? "IMPACT"
        : "No impact – detection system malfunction"
      : "INCOMING MISSILE";
    card.count.textContent = done ? "" : countdown(warning.ends - now);
    card.count.hidden = done;
    card.label.textContent = done
      ? ""
      : `until impact · detected ${timeOfDay(warning.detected)}`;
    card.detail.textContent = done
      ? `Warning detected ${timeOfDay(warning.detected)}.`
      : "";
    card.ack.hidden = done || acked.has(warning.id);
    return card.node;
  }

  const showReadiness = memo((mine: CountryStatus, theirs: CountryStatus) =>
    $("readiness").replaceChildren(
      ...[mine, theirs].map((country, i) =>
        el(
          "li",
          {},
          el("strong", { textContent: `${country.name}${i === 0 ? " (you)" : ""}: ` }),
          el("span", {
            className: country.started ? "ok" : country.claimed ? "warn" : "muted",
            textContent: country.started
              ? "pressed Start"
              : country.claimed
                ? "claimed, not started"
                : "not claimed yet",
          }),
        ),
      ),
    ),
  );
  const showSettings = memo((names: [string, string], settings: CountrySettings) =>
    renderSettingsSummary($("setupSettings"), names, settings),
  );
  const showLog = memo((entries: [number, string][]) =>
    $("log").replaceChildren(
      ...entries.map(([at, text]) =>
        el(
          "li",
          {},
          el("time", { textContent: timeOfDay(at) }),
          el("span", { textContent: text }),
        ),
      ),
    ),
  );
  const showReveal = memo(
    (reveal: Reveal, names: [string, string], settings: CountrySettings) =>
      renderReveal($("countryReveal"), reveal, names, settings),
  );

  let lastBeep = 0;
  let lastVibrate = 0;

  function render(): void {
    const connected = feed.connected();
    $("lostBanner").hidden = !view || connected || view.status === "over";
    $("unlockBanner").hidden =
      !view || sound.unlocked() || view.status === "over";
    if (!view) return;

    const now = feed.now();
    const { names, me } = view;
    $("me").textContent = names[me];
    $("other").textContent = names[1 - me];
    document.title = `{{DAY}} · ${names[me]}`;

    $("countrySetup").hidden = view.status !== "setup";
    $("countryLive").hidden = view.status !== "live";
    $("countryReveal").hidden = view.status !== "over";

    if (view.status === "setup" && view.countries) {
      const mine = view.countries[me];
      const theirs = view.countries[1 - me];
      showReadiness(version, mine, theirs);
      const start = byId<HTMLButtonElement>("startButton");
      start.hidden = view.watch_only;
      start.disabled = mine.started;
      start.textContent = mine.started ? "Started" : "Start";
      $("startHint").textContent =
        notice && Date.now() < notice.until
          ? notice.text
          : view.watch_only && !mine.started
            ? "Your country presses Start from a command link. The game goes live when both countries have pressed Start."
            : mine.started
              ? `Waiting for ${theirs.name}. The game goes live when both countries have pressed Start. Changing the settings resets Start.`
              : "Press Start when your country is gathered and ready. The game goes live when both countries have pressed Start.";
      showSettings(version, names, view.settings);
    }

    const active =
      view.status === "live" ? view.warnings.filter((w) => !w.result) : [];
    const sounding = active.filter(
      (w) => !acked.has(w.id) && !view?.destroyed,
    );

    if (view.status === "live") {
      const visible = view.warnings.filter(
        (w) => !w.result || now < w.ends + 60_000,
      );
      const cards = visible.map(warningCard);
      const container = $("warnings");
      if (
        cards.length !== container.children.length ||
        cards.some((card, i) => container.children[i] !== card)
      ) {
        container.replaceChildren(...cards);
      }

      if (view.outgoing) {
        const hit = now >= view.outgoing.impact;
        $("outgoing").replaceChildren(
          el(
            "div",
            { className: "outgoing" },
            el("h2", {
              textContent: hit
                ? `Impact confirmed on ${names[1 - me]}`
                : `Missile in flight to ${names[1 - me]}`,
            }),
            hit
              ? null
              : el("div", {
                  className: "big",
                  textContent: countdown(view.outgoing.impact - now),
                }),
            el("p", {
              className: "muted",
              textContent: `Launched ${timeOfDay(view.outgoing.launched)}.`,
            }),
          ),
        );
      } else {
        $("outgoing").replaceChildren();
      }

      $("console").hidden = view.watch_only;
      $("endPanel").hidden = view.watch_only;
      if (!view.can_launch) armed = false;
      button.disabled = !view.can_launch || !armed;
      $("cover").hidden = !view.can_launch;
      $("cover").textContent = armed ? "Close safety cover" : "Lift safety cover";
      const retaliate = active.length > 0;
      $("launchLabel").textContent = !view.can_launch
        ? view.outgoing
          ? "Launched"
          : view.destroyed
            ? "Destroyed"
            : "Locked"
        : !armed
          ? "Covered"
          : retaliate
            ? "Hold to retaliate"
            : "Hold to launch";
      const status = $("launchStatus");
      status.className = "status";
      if (notice && Date.now() < notice.until) {
        status.textContent = notice.text;
        if (notice.error) status.classList.add("bad");
      } else if (armed && view.can_launch) {
        status.textContent = `Hold the button for ${HOLD_MS / 1000} seconds to launch at ${names[1 - me]}. There is no recall.`;
        status.classList.add("bad");
      } else if (
        !view.can_launch &&
        view.end_requested &&
        !view.outgoing &&
        !view.destroyed
      ) {
        status.textContent =
          "After requesting the end, you can only launch while a warning is on screen.";
      } else {
        status.textContent = "";
      }

      $("endIdle").hidden = !!view.end_requested || confirmingEnd;
      $("endConfirm").hidden = !!view.end_requested || !confirmingEnd;
      $("endWaiting").hidden = !view.end_requested;

      const entries: [number, string][] = [];
      if (view.live_at) entries.push([view.live_at, "The game went live."]);
      for (const warning of view.warnings) {
        if (!warning.result) continue;
        entries.push([
          warning.detected,
          `Warning: incoming missile. ${warning.result === "impact" ? "Impact." : "No impact – detection system malfunction."}`,
        ]);
      }
      if (view.outgoing) {
        entries.push([
          view.outgoing.launched,
          `Your country launched at ${names[1 - me]}.`,
        ]);
        if (now >= view.outgoing.impact) {
          entries.push([
            view.outgoing.impact,
            `Your missile hit ${names[1 - me]}.`,
          ]);
        }
      }
      if (view.end_requested) {
        entries.push([view.end_requested, "Your country requested the end."]);
      }
      entries.sort((a, b) => a[0] - b[0]);
      showLog(`${version}|${entries.length}`, entries);
    }

    if (view.status === "over" && view.reveal) {
      showReveal(version, view.reveal, names, view.settings);
    }

    $("hit").hidden = !view.destroyed || hitClosed;
    $("hitConsequence").textContent =
      view.settings.consequences[me].trim() || "Carry out your consequence.";

    sound.alarm(sounding.length > 0);
    document.body.classList.toggle("alarm", sounding.length > 0);
    if (
      sounding.length > 0 &&
      navigator.vibrate &&
      Date.now() - lastVibrate > 1500
    ) {
      navigator.vibrate([700, 300]);
      lastVibrate = Date.now();
    }
    if (!$("lostBanner").hidden && Date.now() - lastBeep > 2000) {
      sound.beep();
      lastBeep = Date.now();
    }
  }

  setInterval(render, 200);
}

const path = location.pathname.replace(/\/+$/, "");
const where = /^\/(g|k)\/([A-Za-z0-9]+)$/.exec(path);
if (where && where[1] === "g") gamePage(where[2]);
else if (where && where[1] === "k") countryPage(where[2]);
else if (path === "/check") checkPage();
else if (path === "/groups") groupsPage();
else if (path === "/info") $("info").hidden = false;
else homePage();
