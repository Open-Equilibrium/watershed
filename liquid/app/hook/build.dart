import 'dart:io';

import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';
import 'package:logging/logging.dart';

/// Compiles the Rust bridge with the pinned toolchain and bundles it into the app.
void main(List<String> args) async {
  await build(args, (input, output) async {
    await const FlutterRustBridgeNativeAssetsBuilder(
      cratePath: '../liquid-bridge',
      extraCargoBuildArgs: ['--locked'],
    ).run(input: input, output: output, logger: buildLog(stdout));
    // Cargo lists only the Rust sources it read; a new lock, manifest or toolchain pin
    // rebuilds the bridge too.
    output.dependencies.addAll([
      for (final file in [
        '../../Cargo.lock',
        '../../Cargo.toml',
        '../liquid-bridge/Cargo.toml',
        '../liquid-bridge/rust-toolchain.toml',
      ])
        input.packageRoot.resolve(file),
    ]);
  });
}

/// Writes the build's progress without its configuration records, in which the Rust builder
/// repeats the hook's environment. Flutter's hook runner still records the variables it passes
/// to hooks (TESTING.md).
Logger buildLog(StringSink sink) => Logger.detached('liquid_bridge')
  ..level = Level.INFO
  ..onRecord.listen(sink.writeln);
