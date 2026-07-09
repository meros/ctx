{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "ctx";
  version = "0.1.0";

  src = ./.;

  cargoHash = "sha256-ukm9i8ancpMqsnd1jPVsRjRkqi4uP8Eg4hUT+ITpPag=";

  meta = {
    description = "Context gatherer CLI for AI-assisted coding — fast, token-aware, AST-powered";
    license = lib.licenses.mit;
    mainProgram = "ctx";
  };
}
