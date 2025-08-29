{
  pkgs ? import <nixpkgs> { },
}:
with pkgs;
mkShell {
  nativeBuildInputs = [
    cargo
    rustfmt
    rustPackages.clippy
    pkg-config
  ];
  buildInputs = [
    openssl.dev
    clang
    gdb
    lldb
    just
    grcov
    cargo-limit
    cargo-watch
  ]
  ++ pkgs.lib.optional pkgs.stdenv.isDarwin darwin.apple_sdk.frameworks.Security;
}
