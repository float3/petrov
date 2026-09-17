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
      site = lib.types.submodule {
        options = {
          brand = lib.mkOption {
            type = lib.types.enum ["petrov" "arkhipov"];
            default = "petrov";
          };
          port = lib.mkOption {
            type = lib.types.port;
          };
          domains = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = "Serve through nginx with ACME on each of these domains.";
          };
        };
      };
    in {
      options.services.petrov = {
        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        sites = lib.mkOption {
          type = lib.types.attrsOf site;
          default = {};
          description = "Each site is its own service with its state in /var/lib/<name>.";
        };
      };

      config = {
        systemd.services =
          lib.mapAttrs (name: site: {
            description = "${site.brand} ritual server (${name})";
            wantedBy = ["multi-user.target"];
            after = ["network.target"];
            environment = {
              PETROV_ADDR = "127.0.0.1:${toString site.port}";
              PETROV_BRAND = site.brand;
              PETROV_STATE = "/var/lib/${name}/games.json";
            };
            serviceConfig = {
              ExecStart = lib.getExe cfg.package;
              DynamicUser = true;
              StateDirectory = name;
              Restart = "always";
              RestartSec = 1;
            };
          })
          cfg.sites;

        services.nginx.virtualHosts = lib.mkMerge (lib.mapAttrsToList (name: site:
          lib.genAttrs site.domains (domain: {
            forceSSL = true;
            enableACME = true;
            locations."/".proxyPass = "http://127.0.0.1:${toString site.port}";
          }))
        cfg.sites);
      };
    };
  };
}
