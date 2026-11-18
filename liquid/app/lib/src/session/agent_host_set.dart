import 'package:flutter/foundation.dart';

import '../agent_hosts/agent_hosts.dart';
import 'backend.dart';

class AgentHostSet {
  final _projects = <String?, ProjectBackend>{};
  final _machines = <String?, String>{};
  final _names = <String?, String>{};

  final problems = ValueNotifier<Map<String?, String>>(const {});

  ProjectBackend? operator [](String? address) => _projects[address];

  bool answers(String? address) => _projects.containsKey(address);

  List<String?> get connected => _projects.keys.toList();

  List<String?> get addresses =>
      {..._projects.keys, ...problems.value.keys}.toList();

  String? machineOf(String? address) => _machines[address];

  bool hasName(String? address) => _names.containsKey(address);

  void setName(String? address, String name) => _names[address] = name;

  String nameOf(String? address) =>
      address == null ? thisMacName : _names[address] ?? address;

  Iterable<MapEntry<String?, String>> get names => _names.entries;

  void put(String? address, ProjectBackend project, String? machine) {
    _projects.remove(address)?.close();
    _projects[address] = project;
    if (machine == null) {
      _machines.remove(address);
    } else {
      _machines[address] = machine;
    }
  }

  void close(String? address) => _projects.remove(address)?.close();

  void closeAll() {
    for (final project in _projects.values) {
      project.close();
    }
    _projects.clear();
  }

  void setProblem(String? address, String? problem) {
    final next = {...problems.value};
    if (problem == null) {
      next.remove(address);
    } else {
      next[address] = problem;
    }
    problems.value = next;
  }
}
