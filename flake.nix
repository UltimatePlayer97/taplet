{
  description = "Taplet - Wayland-native autoclicker";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        runtimeLibs = with pkgs; [
          wayland
          libxkbcommon
        ];

        taplet = pkgs.rustPlatform.buildRustPackage {
          pname = "taplet";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
          ];

          buildInputs = runtimeLibs;

          postInstall = ''
            install -Dm644 packaging/taplet.desktop $out/share/applications/taplet.desktop
            wrapProgram $out/bin/taplet \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
          '';

          meta = {
            description = "Wayland-native autoclicker with a friendly GUI";
            license = pkgs.lib.licenses.mit;
            mainProgram = "taplet";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      in
      {
        packages = {
          default = taplet;
          taplet = taplet;
        };

        apps.default = flake-utils.lib.mkApp { drv = taplet; };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ taplet ];
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
        };
      }) // {
      overlays.default = final: prev: {
        taplet = self.packages.${final.stdenv.hostPlatform.system}.default;
      };
    };
}
