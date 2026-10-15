{
  description = "batnotes — a minimal always-available notes overlay with first-class Hebrew";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      batnotes = pkgs.rustPlatform.buildRustPackage {
        pname = "batnotes";
        version = "0.2.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;

        # GTK4 is the reason for this build: Pango gives correct Hebrew
        # shaping and bidi, which egui's cmap-only text renderer cannot.
        nativeBuildInputs = with pkgs; [ pkg-config ];
        buildInputs = with pkgs; [ gtk4 glib pango cairo ];
        # Keep the release binary honest: no stripping, no panic=abort, so a
        # crash is diagnosable instead of an anonymous core dump.
        cargoBuildFlags = [ "--release" ];
        doCheck = false;
        meta = {
          mainProgram = "batnotes";
          description = "Minimal notes overlay with first-class Hebrew";
        };
      };
    in
    {
      packages.${system} = {
        default = batnotes;
        inherit batnotes;
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          rust-analyzer
          clippy
          rustfmt
          pkg-config
          gtk4
          glib
          pango
          cairo
        ];
        # gtk4-rs calls pkg-config at build time, so it must be on the path.
        shellHook = ''
          export PKG_CONFIG_PATH="''${PKG_CONFIG_PATH:+''${PKG_CONFIG_PATH}:}${
            toString pkgs.gtk4.dev.out}/lib/pkgconfig:${
            toString pkgs.glib.dev.out}/lib/pkgconfig:${
            toString pkgs.pango.dev.out}/lib/pkgconfig"
        '';
    };
  };
}