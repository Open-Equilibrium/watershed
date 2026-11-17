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
/// Liquid's sessions use. Opening the section checks every host, again every 10 s and when Liquid
/// comes back to the front; a failed change shows at the top of Settings.
class AgentHostsPane extends StatefulWidget {
  const AgentHostsPane({
    super.key,
    required this.hosts,
    required this.saveOutcome,
  });

  final AgentHostsController hosts;

  /// Receives the message of a failed change, or null once a change went through.
  final void Function(String? failure) saveOutcome;

  @override
  State<AgentHostsPane> createState() => _AgentHostsPaneState();
}

class _AgentHostsPaneState extends State<AgentHostsPane> {
  /// The latest check of each host by address, with null for This Mac. A host without one is still
  /// being checked.
  final _checks = <String?, _Check>{};

  /// The hosts whose check runs now, so checks do not pile up behind a slow host.
  final _checking = <String?>{};

  /// Whether a change runs; the rows wait for it.
  bool _changing = false;

  late final Timer _recheck;
  late final AppLifecycleListener _lifecycle;

  @override
  void initState() {
    super.initState();
    _recheck = Timer.periodic(const Duration(seconds: 10), (_) => _checkAll());
    _lifecycle = AppLifecycleListener(onResume: _checkAll);
    _checkAll();
  }

  @override
  void dispose() {
    _recheck.cancel();
    _lifecycle.dispose();
    super.dispose();
  }

  void _checkAll() {
    for (final address in <String?>[
      null,
      for (final host in widget.hosts.value.hosts) host.address,
    ]) {
      unawaited(_check(address));
    }
  }

  Future<void> _check(String? address) async {
    if (!_checking.add(address)) return;
    final check = await widget.hosts
        .reach(address)
        .then<_Check>(
          (reach) => (reach: reach, error: null),
          onError: (Object error) => (reach: null, error: messageOf(error)),
        );
    _checking.remove(address);
    if (mounted) setState(() => _checks[address] = check);
  }

  Future<void> _add() async {
    final added = await showDialog<SavedAgentHost>(
      context: context,
      builder: (_) => AddAgentHostDialog(hosts: widget.hosts),
    );
    if (added == null || !mounted) return;
    widget.saveOutcome(null);
    setState(() => _checks.remove(added.address));
    unawaited(_check(added.address));
  }

  /// Runs a change of the hosts: its rows wait for it, and its failure shows at the top of Settings.
  Future<void> _change(Future<void> Function() change) async {
    setState(() => _changing = true);
    try {
      await change();
      if (mounted) widget.saveOutcome(null);
    } on AgentHostsError catch (error) {
      if (mounted) widget.saveOutcome(error.message);
    } finally {
      if (mounted) setState(() => _changing = false);
    }
  }

  Future<void> _use(String? address) =>
      _change(() => widget.hosts.use(address));

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
    await _change(() => widget.hosts.remove(host));
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
          enabled: !active && !_changing,
        ),
        if (host case final host?) ...[
          null,
          RowAction(
            'Remove from this device…',
            () => unawaited(_remove(host)),
            danger: true,
            enabled: !_changing,
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
          ],
        ),
      );
    },
  );
}
