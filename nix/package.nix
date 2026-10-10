{
  lib,
  rustPlatform,
  makeWrapper,
  git,
  jdk21,
}:

let
  manifest = lib.importTOML ../Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "grounds";
  inherit (manifest.workspace.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "grounds-cli"
  ];
  cargoTestFlags = [ "--workspace" ];

  nativeBuildInputs = [ makeWrapper ];

  # the tests read JDK symbols and drive git in a temporary repository
  nativeCheckInputs = [
    git
    jdk21
  ];
  preCheck = ''
    export JAVA_HOME=${jdk21.home}
  '';

  # git is needed at runtime for changed-line detection; the user's own git
  # (earlier on PATH) wins. The JDK is not bundled: grounds finds the
  # project's JDK through JAVA_HOME or `java` on PATH.
  postInstall = ''
    wrapProgram $out/bin/grounds --suffix PATH : ${lib.makeBinPath [ git ]}
  '';

  meta = {
    description = "Fast, incremental static analysis for Java, built for coding agents";
    license = lib.licenses.asl20;
    mainProgram = "grounds";
    platforms = lib.platforms.unix;
  };
}
