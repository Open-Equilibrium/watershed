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
  });
}

/// Writes the build's progress, but not its configuration records: the Rust builder lists the
/// whole environment there, and the environment can hold credentials.
Logger buildLog(StringSink sink) => Logger.detached('liquid_bridge')
  ..level = Level.INFO
  ..onRecord.listen(sink.writeln);
