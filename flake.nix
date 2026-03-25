{
  description = "RustMail — a modern, self-hosted SMTP mail catcher";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Build the SolidJS UI first
        uiDist = pkgs.stdenv.mkDerivation {
          pname = "rustmail-ui";
          version = "0.1.0";
          src = ./ui;
          nativeBuildInputs = [ pkgs.nodejs_22 pkgs.pnpm_10 ];

          pnpmDeps = pkgs.pnpm_10.fetchDeps {
            pname = "rustmail-ui";
            version = "0.1.0";
            src = ./ui;
            # First `nix build` will fail and print the correct hash — paste it here.
            hash = "";
          };

          configurePhase = ''
            runHook preConfigure
            export HOME=$TMPDIR
            pnpm config set store-dir $TMPDIR/.pnpm-store
            pnpm install --offline --frozen-lockfile
            runHook postConfigure
          '';

          buildPhase = ''
            runHook preBuild
            pnpm build
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            cp -r dist $out
            runHook postInstall
          '';
        };

        # Filter source to only include Rust/Cargo files
        rustSrc = let
          rawSrc = craneLib.cleanCargoSource ./.;
        in
          pkgs.stdenv.mkDerivation {
            name = "rustmail-src-with-ui";
            src = rawSrc;
            buildCommand = ''
              cp -r $src $out
              chmod -R u+w $out
              mkdir -p $out/ui
              cp -r ${uiDist} $out/ui/dist
            '';
          };

        commonArgs = {
          src = rustSrc;
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        rustmail = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "--package rustmail-server --no-default-features";
          meta = {
            description = "A modern, self-hosted SMTP mail catcher with web UI";
            homepage = "https://github.com/rustmailapp/rustmail";
            license = with pkgs.lib.licenses; [ mit asl20 ];
            mainProgram = "rustmail";
          };
        });
      in
      {
        packages = {
          default = rustmail;
          inherit rustmail;
        };

        devShells.default = craneLib.devShell {
          inputsFrom = [ rustmail ];
          packages = [
            pkgs.nodejs_22
            pkgs.pnpm_10
            pkgs.cargo-watch
            pkgs.sqlx-cli
          ];
        };
      }
    );
}
