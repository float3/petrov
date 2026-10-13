# petrov

A Petrov Day ritual for two countries: one missile each, a warning window, and false alarms that look exactly like real launches.

Built on [rooms](https://github.com/float3/rooms), which holds the parts
all three of these tools share: the keys, the event streams, the store,
the QR codes, and the browser code under this page. This repository holds
the ritual itself.

## Run

```sh
nix develop --command cargo run
```

Listens on `127.0.0.1:8095` (`PETROV_ADDR`) and stores games in `games.json` (`PETROV_STATE`). Open `/`, create a game and send the game link to the other host. Each host claims their own country once and shares the resulting country link with everyone in that country. The game goes live when both countries press Start and ends when a country requests the end and nothing is in flight.

The page is TypeScript in `client/`, compiled into the binary by the build
script. `esbuild` bundles it and `tsc` checks it, both from the dev shell,
and no JavaScript is written by hand. The styles stay in
`static/index.html`, because this page uses none of the shared
stylesheet.

## Brands

`PETROV_BRAND=arkhipov` serves the same app as Arkhipov Day instead of Petrov Day.

## NixOS

```nix
inputs.petrov.url = "github:float3/petrov";

imports = [inputs.petrov.nixosModules.default];
services.petrov.sites = {
  petrov = {
    port = 8095;
    domains = ["petrov.example.com"];
  };
  arkhipov = {
    brand = "arkhipov";
    port = 8096;
    domains = ["arkhipov.example.com"];
  };
};
```

Each site's port is held by a systemd socket unit and passed to the server,
which then runs with no network of its own and a tight sandbox: an exploit
cannot reach the internet, other services on loopback, or other state.
Outside systemd, `PETROV_ADDR` is bound as usual.
