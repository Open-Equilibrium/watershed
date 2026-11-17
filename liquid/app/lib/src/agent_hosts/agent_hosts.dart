import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';

import '../rust/api/hosts.dart';
import '../settings_file.dart';

/// An agent host Liquid pairs with, as Settings keep it. Its device token is in the keychain under
/// the address, never here.
@immutable
class SavedAgentHost {
  const SavedAgentHost({
    required this.name,
    required this.address,
    required this.client,
    this.active = false,
  });

  /// Reads one host from `json`, or none when it lacks a name, address or device name.
  static SavedAgentHost? fromJson(Object? json) {
    if (json is! Map<String, Object?>) return null;
    final name = json['name'];
    final address = json['address'];
    final client = json['client'];
    if (name is! String || address is! String || client is! String) {
      return null;
    }
    return SavedAgentHost(
      name: name,
      address: address,
      client: client,
      active: json['active'] == true,
    );
  }

  final String name;
  final String address;

  /// The name this device paired under on the host, which `meta host revoke` names to remove it.
  final String client;

  /// Whether Liquid's sessions run on this host (ADR-0295).
  final bool active;

  SavedAgentHost copyWith({required bool active}) => SavedAgentHost(
    name: name,
    address: address,
    client: client,
    active: active,
  );

  Map<String, Object?> toJson() => {
    'name': name,
    'address': address,
    'client': client,
    'active': active,
  };
}

/// The saved agent hosts, besides This Mac, which is always listed first. At most one host is
/// active; with none active, sessions run on This Mac.
@immutable
class AgentHostsSettings {
  const AgentHostsSettings([this.hosts = const []]);

  /// Reads `json`, skipping any host it cannot read.
  factory AgentHostsSettings.fromJson(Map<String, Object?> json) {
    final hosts = json['hosts'];
    return AgentHostsSettings([
      if (hosts is List)
        for (final host in hosts) ?SavedAgentHost.fromJson(host),
    ]);
  }

  /// The settings in the file, or none before the first save.
  static Future<AgentHostsSettings> load(SettingsFile file) async =>
      AgentHostsSettings.fromJson(await file.load() ?? const {});

  final List<SavedAgentHost> hosts;

  /// The address of the active host, or null when This Mac is active.
  String? get activeAddress =>
      hosts.where((host) => host.active).firstOrNull?.address;

  AgentHostsSettings added(SavedAgentHost host) =>
      AgentHostsSettings([...hosts, host]);

  AgentHostsSettings without(String address) => AgentHostsSettings([
    for (final host in hosts)
      if (host.address != address) host,
  ]);

  /// The host at `address` made the active one, or This Mac when `address` is null.
  AgentHostsSettings withActive(String? address) => AgentHostsSettings([
    for (final host in hosts) host.copyWith(active: host.address == address),
  ]);

  Map<String, Object?> toJson() => {
    'hosts': [for (final host in hosts) host.toJson()],
  };
}

/// What the Settings section needs from the bridge; tests substitute a fake. A null address is
/// This Mac.
abstract interface class AgentHostBridge {
  /// Pairs with the host at `address`, answering its name and the name this device pairs under.
  Future<BridgePairing> pair(String address, String code);

  /// Forgets the device token of the host at `address` on this Mac.
  Future<void> forget(String address);

  /// Whether the host answers now; an error names why the check failed.
  Future<BridgeReach> reach(String? address);
}

/// The bridge's agent hosts: This Mac's service in `home`, and the saved hosts through their device
/// tokens in the keychain.
class RustAgentHostBridge implements AgentHostBridge {
  RustAgentHostBridge({this.home});

  /// This Mac's Meta-Harness data folder, or the default one.
  final String? home;

  @override
  Future<BridgePairing> pair(String address, String code) =>
      pairAgentHost(address: address, code: code);

  @override
  Future<void> forget(String address) => forgetAgentHost(address: address);

  @override
  Future<BridgeReach> reach(String? address) => address == null
      ? reachLocalAgentHost(home: home)
      : reachRemoteAgentHost(address: address);
}

/// A failed change of the agent hosts: a sentence that says what failed and whether anything
/// changed, then the tool's own message.
class AgentHostsError implements Exception {
  const AgentHostsError(this.message);

  final String message;

  @override
  String toString() => message;
}

/// The message of `error` as Liquid shows it: a bridge error's or a file's own text.
String messageOf(Object error) => switch (error) {
  AnyhowException() => error.message,
  FileSystemException() => error.message,
  _ => '$error',
};

/// Settings → Agent hosts: the saved hosts in use, saved at each change. Each change runs its bridge
/// step first, so a failed step leaves the saved list as it was, and a failed save never leaves a
/// token behind.
class AgentHostsController extends ValueNotifier<AgentHostsSettings> {
  AgentHostsController({
    required AgentHostsSettings settings,
    required this.bridge,
    required this.running,
    required this.save,
  }) : super(settings);

  final AgentHostBridge bridge;

  /// The address of the host this session connected to at launch, or null for This Mac. A new
  /// active host applies at the next start.
  final String? running;

  /// Writes the settings to Liquid's settings file.
  final Future<void> Function(AgentHostsSettings settings) save;

  Future<T> _step<T>(String failed, Future<T> Function() step) async {
    try {
      return await step();
    } on Object catch (error) {
      throw AgentHostsError('$failed ${messageOf(error)}');
    }
  }

  Future<void> _write(AgentHostsSettings next) async {
    await save(next);
    value = next;
  }

  /// Pairs with the host at `address` using `code`, then saves it.
  Future<SavedAgentHost> pair(String address, String code) async {
    const failed = 'Could not add the agent host; nothing changed.';
    final paired = await _step(failed, () => bridge.pair(address, code));
    final host = SavedAgentHost(
      name: paired.host,
      address: address,
      client: paired.client,
    );
    try {
      await _write(value.added(host));
    } on Object catch (error) {
      try {
        await bridge.forget(address);
      } on Object catch (forgetting) {
        throw AgentHostsError(
          'Could not add the agent host; its device token could not be forgotten, so the keychain still holds it. ${messageOf(forgetting)}',
        );
      }
      throw AgentHostsError('$failed ${messageOf(error)}');
    }
    return host;
  }

  /// Forgets the token of `host` on this Mac, then drops it from the saved list.
  Future<void> remove(SavedAgentHost host) async {
    await _step(
      'Could not remove ${host.name}; nothing changed.',
      () => bridge.forget(host.address),
    );
    await _step(
      'Could not remove ${host.name} from the list; its token here is forgotten, so remove it again to finish.',
      () => _write(value.without(host.address)),
    );
  }

  /// Makes the host at `address` the active one, or This Mac for null.
  Future<void> use(String? address) => _step(
    'Could not change the agent host for sessions; nothing changed.',
    () => _write(value.withActive(address)),
  );

  /// Whether the host at `address` answers now; null is This Mac.
  Future<BridgeReach> reach(String? address) => bridge.reach(address);
}
