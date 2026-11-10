import 'dart:io';

import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';
import 'package:logging/logging.dart';

/// The files a change to which rebuilds the bridge, relative to the app: Cargo lists only the
/// Rust sources it read, so the lock, the manifests of every local crate the bridge builds and
/// the toolchain pin are added.
const bridgeInputs = [
  '../../Cargo.lock',
  '../../Cargo.toml',
  '../liquid-bridge/Cargo.toml',
  '../liquid-bridge/rust-toolchain.toml',
  '../liquid-core/Cargo.toml',
  '../liquid-storage/Cargo.toml',
  '../../meta-harness/meta-harness-core/Cargo.toml',
];

/// Compiles the Rust bridge with the pinned toolchain and bundles it into the app.
void main(List<String> args) async {
  await build(args, (input, output) async {
    await const FlutterRustBridgeNativeAssetsBuilder(
      cratePath: '../liquid-bridge',
      extraCargoBuildArgs: ['--locked'],
    ).run(input: input, output: output, logger: buildLog(stdout));
    output.dependencies.addAll([
      for (final file in bridgeInputs) input.packageRoot.resolve(file),
    ]);
  });
}

/// Writes the build's progress without its configuration records, in which the Rust builder
/// repeats the hook's environment. Flutter's hook runner still records the variables it passes
/// to hooks (TESTING.md).
Logger buildLog(StringSink sink) => Logger.detached('liquid_bridge')
  ..level = Level.INFO
  ..onRecord.listen(sink.writeln);
