{ pkgs, config, ... }:

let
  lyricsgenius = ps: ps.buildPythonPackage rec {
    pname = "lyricsgenius";
    version = "3.7.6";
    pyproject = true;

    src = ps.fetchPypi {
      inherit pname version;
      hash = "sha256-zQGrgZEz4o9RSYWmGXH8TcNXUcRSfmF+xJCROQ3cPJ4=";
    };

    nativeBuildInputs = [ ps.hatchling ];
    propagatedBuildInputs = [ ps.requests ps.beautifulsoup4 ];
    doCheck = false;
  };

  pythonEnv = pkgs.python3.withPackages (ps: [
    (lyricsgenius ps)
    ps.requests
    ps.beautifulsoup4
    ps.mutagen
    ps.xxhash
  ]);

  mkPyAction = name: scriptPath: pkgs.writeScriptBin name ''
    #!${pythonEnv}/bin/python3
    ${builtins.readFile scriptPath}
  '';

  pyLyrics = mkPyAction "lyrics" ./actions/python/lyrics/main.py;
  pyRename = mkPyAction "rename" ./actions/python/rename/main.py;
  pySearchCover = mkPyAction "search_cover" ./actions/python/search_cover/main.py;

  pythonActions = [
    pyLyrics
    pyRename
    pySearchCover
  ];
in
{
  languages.rust = {
    enable = true;
    channel = "nightly";
    version = "2026-10-22";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
  };

  languages.javascript = {
    enable = true;
    bun.enable = true;
  };

  languages.python = {
    enable = true;
    package = pythonEnv;
  };

  packages = with pkgs; [
    pkg-config
    openssl
    cargo-deny
    git
  ] ++ pythonActions;

  enterShell = ''
    export PATH="$PWD/interfaces/web-app/node_modules/.bin:$PATH"
  '';

  scripts.build.exec = ''
    set -euo pipefail

    ROOT=$(git rev-parse --show-toplevel)
    cd "$ROOT"

    # Just the artifacts: the binary and the actions it shells out to, which are
    # resolved as target/release/<name>. Add any new action crate to this list.
    cargo build --release \
      -p datamile \
      -p calculate_cover_metrics \
      -p collect \
      -p embed \
      -p musicbrainz_fetch \
      -p theme

    ln -sf "${pyLyrics}/bin/lyrics" "$ROOT/actions/lyrics"
    ln -sf "${pySearchCover}/bin/search_cover" "$ROOT/actions/search_cover"
    ln -sf "${pyRename}/bin/rename" "$ROOT/actions/rename"

    (cd "$ROOT/interfaces/web-app" && bun run build)
  '';

  scripts.check.exec = ''
    ROOT=$(git rev-parse --show-toplevel)
    cd "$ROOT"

    cargo check --workspace "$@"
    (cd "$ROOT/interfaces/web-app" && bun run check && bun run lint:logic)
  '';

  scripts.tests.exec = ''
    set -euo pipefail

    ROOT=$(git rev-parse --show-toplevel)
    cd "$ROOT"

    cargo test --workspace
    (cd "$ROOT/interfaces/web-app" && bun run lint)
  '';

  scripts.format.exec = ''
    set -euo pipefail

    cargo fmt --all "$@"
  '';

  scripts.lint.exec = ''
    set -euo pipefail

    ROOT=$(git rev-parse --show-toplevel)
    cd "$ROOT"

    cargo fmt --all --check
    cargo clippy --workspace --lib --bins -- --deny warnings

    if grep -rnE --include='*.rs' '\bfn get_[a-z]' core actions; then
      printf '\n' >&2
      echo 'lint: drop the get_ prefix, name the getter after the value' >&2
      exit 1
    fi
  '';
}
