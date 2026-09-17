# petrov

Multiplayer Petrov Day: two houses, one red button each, a retaliation window and false alarms.

## Run

```sh
cargo run
```

Listens on `127.0.0.1:8095` (`PETROV_ADDR`) and stores games in `games.json` (`PETROV_STATE`). Open `/`, create a game, and open each house's secret link on the device in that house.

## NixOS

```nix
inputs.petrov.url = "github:float3/petrov";

imports = [inputs.petrov.nixosModules.default];
services.petrov = {
  enable = true;
  domain = "petrov.example.com";
};
```
