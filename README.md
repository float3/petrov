# petrov

A Petrov Day ritual for two countries: one missile each, a warning window, and false alarms that look exactly like real launches.

## Run

```sh
cargo run
```

Listens on `127.0.0.1:8095` (`PETROV_ADDR`) and stores games in `games.json` (`PETROV_STATE`). Open `/`, create a game and send the game link to the other host. Each host claims their own country once and shares the resulting country link with everyone in that country. The game goes live when both countries press Start and ends when a country requests the end and nothing is in flight.

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
