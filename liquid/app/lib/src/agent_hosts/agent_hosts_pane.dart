import 'dart:async';

import 'package:flutter/material.dart';

import '../agent/session_menus.dart';
import '../controls.dart';
import '../rust/api/hosts.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'add_agent_host_dialog.dart';
import 'agent_hosts.dart';

/// One host's check: its answer, or the reason the check failed.
typedef _Check = ({BridgeReach? reach, String? error});

/// Settings → Agent hosts (ADR-0295): This Mac and the saved hosts, each with its status and the one
/// Liquid's sessions use. Opening the section checks every host.
class AgentHostsPane extends StatefulWidget {
  const AgentHostsPane({super.key, required this.hosts});

  final AgentHostsController hosts;

  @override
  State<AgentHostsPane> createState() => _AgentHostsPaneState();
}

class _AgentHostsPaneState extends State<AgentHostsPane> {
  /// The latest check of each host by address, with null for This Mac. A host without one is still
  /// being checked.
  final _checks = <String?, _Check>{};

  /// What the last change failed with, until the next change.
  String? _problem;

  @override
  void initState() {
    super.initState();
    for (final address in <String?>[
      null,
      for (final host in widget.hosts.value.hosts) host.address,
    ]) {
      unawaited(_check(address));
    }
  }

  Future<void> _check(String? address) async {
    final check = await widget.hosts
        .reach(address)
        .then<_Check>(
          (reach) => (reach: reach, error: null),
          onError: (Object error) => (reach: null, error: messageOf(error)),
        );
    if (mounted) setState(() => _checks[address] = check);
  }

  /// A host changed: its check runs again, and the last failure goes.
  void _changed(String? address) {
    setState(() {
      _problem = null;
      _checks.remove(address);
    });
    unawaited(_check(address));
  }

  Future<void> _add() async {
    final added = await showDialog<SavedAgentHost>(
      context: context,
      builder: (_) => AddAgentHostDialog(hosts: widget.hosts),
    );
    if (added != null && mounted) _changed(added.address);
  }

  Future<void> _use(String? address) async {
    setState(() => _problem = null);
    try {
      await widget.hosts.use(address);
    } on AgentHostsError catch (error) {
      if (mounted) setState(() => _problem = error.message);
    }
  }

  Future<void> _remove(SavedAgentHost host) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => DangerDialog(
        title: 'Remove ${host.name} from this device?',
        action: 'Remove',
        children: [
          Text(
            'This forgets its address and device token here. ${host.name} keeps this device paired as ${host.client}, until meta host revoke ${host.client} runs there.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    setState(() => _problem = null);
    try {
      await widget.hosts.remove(host);
    } on AgentHostsError catch (error) {
      if (mounted) setState(() => _problem = error.message);
    }
  }

  /// The status word of a host; it is text, so no color carries it alone.
  String _status(String? address) => switch (_checks[address]) {
    null => 'Connecting',
    (reach: _, error: final error?) => 'Connection failed: $error',
    (reach: BridgeReach.connected, error: _) => 'Connected',
    _ => 'Offline',
  };

  Widget _row(
    BuildContext context, {
    required String? address,
    required String name,
    required bool active,
    required IconData icon,
    SavedAgentHost? host,
  }) {
    final ws = context.ws;
    final failed = _checks[address]?.error != null;
    return RowMenu(
      key: ValueKey<String?>(address),
      actions: [
        RowAction(
          'Use for sessions',
          () => unawaited(_use(address)),
          enabled: !active,
        ),
        if (host case final host?) ...[
          null,
          RowAction(
            'Remove from this device…',
            () => unawaited(_remove(host)),
            danger: true,
          ),
        ],
      ],
      builder: (context, menu, _) => Padding(
        padding: const EdgeInsets.symmetric(vertical: WsMetrics.space2),
        child: Row(
          children: [
            Icon(icon, size: 16, color: ws.textMuted),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    name,
                    style: TextStyle(
                      color: ws.text,
                      fontSize: WsMetrics.fsCompact,
                      fontWeight: WsMetrics.fwMedium,
                    ),
                  ),
                  Text(
                    [?address, _status(address)].join(' · '),
                    style: TextStyle(
                      color: failed ? ws.danger : ws.textMuted,
                      fontSize: WsMetrics.fsSmall,
                    ),
                  ),
                ],
              ),
            ),
            if (active)
              Container(
                margin: const EdgeInsets.only(right: WsMetrics.space2),
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                  vertical: WsMetrics.space1,
                ),
                decoration: BoxDecoration(
                  color: ws.accentSubtle,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                ),
                child: Text(
                  'Used for sessions',
                  style: TextStyle(
                    color: ws.accent,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
            menu,
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: widget.hosts,
    builder: (context, settings, _) {
      final active = settings.activeAddress;
      final activeName =
          settings.hosts.where((host) => host.active).firstOrNull?.name ??
          'This Mac';
      return SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(
          WsMetrics.space6,
          WsMetrics.space5,
          WsMetrics.space6,
          WsMetrics.space5,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: WsMetrics.space4,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    'Agent hosts',
                    style: Theme.of(context).textTheme.titleMedium
                        ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                  ),
                ),
                GhostButton(
                  label: 'Add agent host',
                  icon: LucideIcons.plus,
                  onPressed: () => unawaited(_add()),
                ),
              ],
            ),
            _row(
              context,
              address: null,
              name: 'This Mac',
              active: active == null,
              icon: LucideIcons.laptop,
            ),
            for (final host in settings.hosts)
              _row(
                context,
                address: host.address,
                name: host.name,
                active: host.active,
                icon: LucideIcons.server,
                host: host,
              ),
            if (_problem case final problem?)
              WarningNotice(problem, danger: true),
            if (active != widget.hosts.running)
              WarningNotice(
                'Restart Liquid to use $activeName for sessions.',
                info: true,
              ),
          ],
        ),
      );
    },
  );
}
