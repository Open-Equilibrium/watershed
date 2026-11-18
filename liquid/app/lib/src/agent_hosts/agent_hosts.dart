import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';

import '../rust/api/hosts.dart';
import '../settings_file.dart';

const thisMacName = 'This Mac';

@immutable
class SavedAgentHost {
  const SavedAgentHost({
    required this.name,
    required this.address,
    required this.client,
    this.active = false,
  });

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

  final String client;

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

@immutable
class AgentHostsSettings {
  const AgentHostsSettings([this.hosts = const []]);

  factory AgentHostsSettings.fromJson(Map<String, Object?> json) {
    final hosts = json['hosts'];
    return AgentHostsSettings([
      if (hosts is List)
        for (final host in hosts) ?SavedAgentHost.fromJson(host),
    ]);
  }

  static Future<AgentHostsSettings> load(SettingsFile file) async =>
      AgentHostsSettings.fromJson(await file.load() ?? const {});

  final List<SavedAgentHost> hosts;

  String? get activeAddress =>
      hosts.where((host) => host.active).firstOrNull?.address;

  AgentHostsSettings added(SavedAgentHost host) =>
      AgentHostsSettings([...hosts, host]);

  AgentHostsSettings without(String address) => AgentHostsSettings([
    for (final host in hosts)
      if (host.address != address) host,
  ]);

  AgentHostsSettings withActive(String? address) => AgentHostsSettings([
    for (final host in hosts) host.copyWith(active: host.address == address),
  ]);

  Map<String, Object?> toJson() => {
    'hosts': [for (final host in hosts) host.toJson()],
  };
}

abstract interface class AgentHostBridge {
  Future<BridgePairing> pair(String address, String code);

  Future<void> forget(String address);

  Future<BridgeReach> reach(String? address);
}

class RustAgentHostBridge implements AgentHostBridge {
  RustAgentHostBridge({this.home});

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

class AgentHostsError implements Exception {
  const AgentHostsError(this.message);

  final String message;

  @override
  String toString() => message;
}

String messageOf(Object error) {
  final text = switch (error) {
    AnyhowException() => error.message,
    FileSystemException() => error.message,
    _ => '$error',
  };
  return text.endsWith('.') ? text : '$text.';
}

String hostProblem(String? address, Object error) => address == null
    ? messageOf(error)
    : 'Liquid could not connect to the agent host $address: ${messageOf(error)}';

abstract interface class AgentHostLinks {
  Future<void> connect(String? address);

  void disconnect(String address);

  Future<String?> startOn(String? address);
}

class AgentHostsController extends ValueNotifier<AgentHostsSettings> {
  AgentHostsController({
    required AgentHostsSettings settings,
    required this.bridge,
    required this.save,
    required this.links,
  }) : super(settings);

  final AgentHostBridge bridge;

  final Future<void> Function(AgentHostsSettings settings) save;

  final AgentHostLinks links;

  int _choices = 0;

  int _changes = 0;

  bool get changing => _changes > 0;

  Future<T> _change<T>(Future<T> Function() change) async {
    _changes++;
    notifyListeners();
    try {
      return await change();
    } finally {
      _changes--;
      notifyListeners();
    }
  }

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

  Future<SavedAgentHost> pair(String address, String code) async {
    const failed = 'Could not add the agent host; nothing changed in Liquid.';
    final paired = await _step(failed, () => bridge.pair(address, code));
    final host = SavedAgentHost(
      name: paired.host,
      address: address,
      client: paired.client,
    );
    try {
      await _write(value.added(host));
      await links.connect(address);
    } on Object catch (error) {
      final kept =
          '${paired.host} keeps this device paired as ${paired.client}; run meta host revoke ${paired.client} there to remove it.';
      try {
        await bridge.forget(address);
      } on Object catch (forgetting) {
        throw AgentHostsError(
          'Could not add the agent host. $kept Its device token could not be forgotten, so the keychain still holds it. ${messageOf(forgetting)}',
        );
      }
      throw AgentHostsError(
        'Could not add the agent host. $kept ${messageOf(error)}',
      );
    }
    return host;
  }

  Future<void> remove(SavedAgentHost host) => _change(() async {
    await _step(
      'Could not remove ${host.name}; nothing changed.',
      () => bridge.forget(host.address),
    );
    await _step(
      'Could not remove ${host.name} from the list; its token here is forgotten, so remove it again to finish.',
      () => _write(value.without(host.address)),
    );
    links.disconnect(host.address);
    if (!host.active) return;
    final choice = ++_choices;
    final failure = await links.startOn(null);
    if (failure != null && choice == _choices) {
      throw AgentHostsError(
        'Removed ${host.name} from this device, but new sessions could not start on $thisMacName. $failure',
      );
    }
  });

  Future<void> use(String? address) => _change(() async {
    final choice = ++_choices;
    final previous = value;
    final failed =
        'Could not use ${_nameOf(address, previous)} for sessions; nothing changed.';
    await _step(failed, () => _write(previous.withActive(address)));
    final failure = await links.startOn(address);
    if (failure == null || choice != _choices) return;
    try {
      await _write(value.withActive(previous.activeAddress));
    } on Object catch (error) {
      throw AgentHostsError(
        'Could not use ${_nameOf(address, previous)} for sessions, and the saved choice could not be restored, so the next start uses ${_nameOf(address, previous)}. ${messageOf(error)}',
      );
    }
    throw AgentHostsError('$failed $failure');
  });

  Future<BridgeReach> reach(String? address) => bridge.reach(address);

  static String _nameOf(String? address, AgentHostsSettings settings) =>
      address == null
      ? thisMacName
      : settings.hosts.firstWhere((host) => host.address == address).name;
}
