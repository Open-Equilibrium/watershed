import 'dart:io';

import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';
import 'package:logging/logging.dart';

const bridgeInputs = [
  '../../Cargo.lock',
  '../../Cargo.toml',
  '../liquid-bridge/Cargo.toml',
  '../liquid-bridge/rust-toolchain.toml',
  '../liquid-core/Cargo.toml',
  '../liquid-storage/Cargo.toml',
  '../../meta-harness/meta-harness-core/Cargo.toml',
];

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

Logger buildLog(StringSink sink) => Logger.detached('liquid_bridge')
  ..level = Level.INFO
  ..onRecord.listen(sink.writeln);
