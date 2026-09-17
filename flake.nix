{
  description = "Multiplayer Petrov Day";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    lib = nixpkgs.lib;
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
  in {
    formatter = forAllSystems (pkgs: pkgs.alejandra);

    packages = forAllSystems (pkgs: {
      default = pkgs.rustPlatform.buildRustPackage {
        pname = "petrov";
        version = "0.1.0";
        src = lib.cleanSource self;
        cargoLock.lockFile = ./Cargo.lock;
        meta.mainProgram = "petrov";
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [cargo clippy rustc rustfmt];
      };
    });

    nixosModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.services.petrov;
    in {
      options.services.petrov = {
        enable = lib.mkEnableOption "the multiplayer Petrov Day server";
        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        port = lib.mkOption {
          type = lib.types.port;
          default = 8095;
        };
        domain = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Serve through nginx with ACME on this domain.";
        };
      };

      config = lib.mkIf cfg.enable (lib.mkMerge [
        {
          systemd.services.petrov = {
            description = "Multiplayer Petrov Day";
            wantedBy = ["multi-user.target"];
            after = ["network.target"];
            environment = {
              PETROV_ADDR = "127.0.0.1:${toString cfg.port}";
              PETROV_STATE = "/var/lib/petrov/games.json";
            };
            serviceConfig = {
              ExecStart = lib.getExe cfg.package;
              DynamicUser = true;
              StateDirectory = "petrov";
              Restart = "always";
              RestartSec = 1;
            };
          };
        }
        (lib.mkIf (cfg.domain != null) {
          services.nginx.virtualHosts.${cfg.domain} = {
            forceSSL = true;
            enableACME = true;
            locations."/".proxyPass = "http://127.0.0.1:${toString cfg.port}";
          };
        })
      ]);
    };
  };
}
