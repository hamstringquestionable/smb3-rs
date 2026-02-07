{
  pkgs ? import <nixpkgs> { },
}:

pkgs.mkShell {
  buildInputs = with pkgs; [
    rustup
    gcc
    pkg-config
    openssl
    wasm-pack
  ];

  shellHook = ''
    export PATH="$HOME/.cargo/bin:$PATH"

    if ! rustup show active-toolchain &>/dev/null 2>&1; then
      rustup default stable
    fi

    if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
      rustup target add wasm32-unknown-unknown
    fi
  '';
}
