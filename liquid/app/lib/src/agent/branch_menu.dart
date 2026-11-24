import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/row_menu.dart';
import 'checkout_pull_request.dart';
import 'session_card_rows.dart';

class BranchRow extends StatefulWidget {
  const BranchRow({super.key, required this.state});

  final SessionState state;

  @override
  State<BranchRow> createState() => _BranchRowState();
}

class _BranchRowState extends State<BranchRow> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<GitState>(
    valueListenable: widget.state.git,
    builder: (context, git, _) => ListenableBuilder(
      listenable: Listenable.merge([git.status, git.problem]),
      builder: (context, _) {
        final ws = context.ws;
        if (git.problem.value case final problem?) {
          return Tooltip(
            message: problem,
            child: CardRow(
              children: [
                Icon(LucideIcons.triangleAlert, size: 16, color: ws.danger),
                Text('Git unavailable', style: TextStyle(color: ws.danger)),
              ],
            ),
          );
        }
        final status = git.status.value;
        if (status == null || !status.repository) {
          return const SizedBox.shrink();
        }
        final name = status.branch ?? 'Select ref';
        void open() => unawaited(showBranchMenu(context, widget.state));
        return Semantics(
          key: const Key('branch-row'),
          button: true,
          onTap: open,
          label: 'Branch $name',
          excludeSemantics: true,
          child: HoverRegion(
            onHover: (hovered, _) {
              if (mounted) setState(() => _hovered = hovered);
            },
            child: CardRow(
              onTap: open,
              children: [
                Icon(LucideIcons.gitBranch, size: 16, color: ws.textFaint),
                Flexible(
                  child: GestureDetector(
                    onSecondaryTapUp: (details) => unawaited(
                      _copyName(
                        context,
                        widget.state,
                        name,
                        details.globalPosition,
                      ),
                    ),
                    child: Tooltip(
                      message: name,
                      child: InkWell(
                        key: const Key('branch-name'),
                        hoverColor: Colors.transparent,
                        highlightColor: Colors.transparent,
                        onTap: open,
                        child: Text(
                          name,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(fontWeight: WsMetrics.fwRegular),
                        ),
                      ),
                    ),
                  ),
                ),
                Container(
                  key: const Key('branch-split'),
                  width: 1,
                  height: 16,
                  color: _hovered ? ws.border : Colors.transparent,
                ),
                InkWell(
                  key: const Key('branch-chevron'),
                  canRequestFocus: false,
                  hoverColor: ws.hover,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                  onTap: open,
                  child: Icon(
                    LucideIcons.chevronDown,
                    size: 16,
                    color: ws.textFaint,
                  ),
                ),
              ],
            ),
          ),
        );
      },
    ),
  );
}

Future<void> _copyName(
  BuildContext context,
  SessionState state,
  String name,
  Offset at,
) async {
  final overlay = Overlay.of(context).context.findRenderObject()! as RenderBox;
  await showMenu<void>(
    context: context,
    position: RelativeRect.fromRect(at & Size.zero, Offset.zero & overlay.size),
    items: [
      PopupMenuItem<void>(
        onTap: () => unawaited(_copy(state, name)),
        child: const Text('Copy branch name'),
      ),
    ],
  );
}

Future<void> _copy(SessionState state, String name) async {
  try {
    await Clipboard.setData(ClipboardData(text: name));
    state.notify(NoticeLevel.info, 'Branch name copied: $name');
  } on Object catch (error) {
    state.notify(NoticeLevel.error, 'Failed to copy branch name. $error');
  }
}

sealed class _Pick {}

final class _Switch extends _Pick {
  _Switch(this.ref);

  final BridgeRef ref;
}

final class _Create extends _Pick {
  _Create(this.name);

  final String name;
}

final class _Checkout extends _Pick {
  _Checkout(this.reference);

  final String reference;
}

Future<void> showBranchMenu(BuildContext context, SessionState state) async {
  final status = state.git.value.status.value;
  if (status == null) return;
  final picked = await showDialog<_Pick>(
    context: context,
    barrierColor: Colors.transparent,
    builder: (_) => _BranchPicker(state: state, status: status),
  );
  if (!context.mounted || picked == null) return;
  if (refusedWhileRunning(state)) return;
  switch (picked) {
    case _Switch(:final ref):
      await _switchTo(context, state, ref);
    case _Create(:final name):
      await _create(context, state, name);
    case _Checkout(:final reference):
      await showCheckoutPullRequestDialog(
        context,
        state,
        status,
        reference: reference,
      );
  }
}

Future<void> _switchTo(
  BuildContext context,
  SessionState state,
  BridgeRef ref,
) async {
  try {
    if (ref.worktree case final worktree?) {
      await state.setBranch(ref.name, worktree: worktree);
      return;
    }
    final branch = ref.remote == null
        ? ref.name
        : ref.name.substring(ref.name.indexOf('/') + 1);
    if (!await _carryChanges(context, state, branch) || !context.mounted) {
      return;
    }
    if (refusedWhileRunning(state)) return;
    await state.git.value.backend.switchTo(branch);
  } on Object catch (error) {
    state.notify(NoticeLevel.error, 'Failed to switch ref. $error');
  }
}

Future<void> _create(
  BuildContext context,
  SessionState state,
  String name,
) async {
  try {
    if (!await _carryChanges(context, state, name) || !context.mounted) {
      return;
    }
    if (refusedWhileRunning(state)) return;
    await state.git.value.backend.createBranch(name);
  } on Object catch (error) {
    state.notify(NoticeLevel.error, 'Failed to create and switch ref. $error');
  }
}

Future<bool> _carryChanges(
  BuildContext context,
  SessionState state,
  String branch,
) async {
  if (state.git.value.status.value?.changes.isEmpty ?? true) return true;
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => ActionDialog(
      title: 'Switch to $branch?',
      actions: [
        PrimaryButton(
          label: 'Switch branch',
          onPressed: () => Navigator.of(context).pop(true),
        ),
      ],
      children: [
        Text(
          "You have uncommitted changes. They'll carry over to the other branch, or block the switch if they conflict.",
          style: DangerDialog.text(context),
        ),
      ],
    ),
  );
  return confirmed ?? false;
}

class _BranchPicker extends StatefulWidget {
  const _BranchPicker({required this.state, required this.status});

  final SessionState state;
  final BridgeGitStatus status;

  @override
  State<_BranchPicker> createState() => _BranchPickerState();
}

class _BranchPickerState extends State<_BranchPicker> {
  final _query = TextEditingController();
  BridgeRefs? _refs;
  String? _refsQuery;

  int _attempt = 0;

  @override
  void initState() {
    super.initState();
    _query.addListener(() => unawaited(_read()));
    unawaited(_read());
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  BridgeRefs? get _current => _refsQuery == _query.text.trim() ? _refs : null;

  Future<void> _read() async {
    final attempt = ++_attempt;
    final query = _query.text.trim();
    try {
      final refs = await widget.state.git.value.backend.refs(query);
      if (mounted && attempt == _attempt) {
        setState(() {
          _refs = refs;
          _refsQuery = query;
        });
      }
    } on Object catch (error) {
      if (!mounted || attempt != _attempt) return;
      widget.state.notify(NoticeLevel.error, 'Could not list the refs: $error');
      Navigator.of(context).pop();
    }
  }

  String? _newName() {
    final name = _query.text.trim().replaceAll(RegExp(r'[ \t\n\r\f\v]+'), '-');
    final refs = _current;
    if (name.isEmpty ||
        refs == null ||
        refs.refs.any((ref) => ref.name == name)) {
      return null;
    }
    return name;
  }

  void _submit() {
    if (widget.state.status.value != RunStatus.idle) return;
    final checkout = pullRequestReference(_query.text);
    final first = _current?.refs.firstOrNull;
    final create = _newName();
    if (checkout != null) {
      Navigator.of(context).pop(_Checkout(checkout));
    } else if (first != null) {
      Navigator.of(context).pop(first.current ? null : _Switch(first));
    } else if (create != null) {
      Navigator.of(context).pop(_Create(create));
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final refs = _refs;
    final checkout = pullRequestReference(_query.text);
    final create = _newName();
    final terms = terminologyFor(widget.status);
    return Stack(
      children: [
        Positioned(
          top: 76,
          right: WsMetrics.space7,
          width: 360,
          child: Material(
            color: ws.surface,
            elevation: 8,
            shadowColor: Colors.black54,
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            child: CallbackShortcuts(
              bindings: {
                const Keys(LogicalKeyboardKey.enter).activator: _submit,
              },
              child: Padding(
                padding: const EdgeInsets.all(WsMetrics.space2),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxHeight: 420),
                  child: ValueListenableBuilder<RunStatus>(
                    valueListenable: widget.state.status,
                    builder: (context, status, _) {
                      final running = status != RunStatus.idle;
                      return ListView(
                        shrinkWrap: true,
                        children: [
                          TextField(
                            key: const Key('branch-search'),
                            controller: _query,
                            autofocus: true,
                            style: TextStyle(
                              fontSize: WsMetrics.fsCompact,
                              color: ws.text,
                            ),
                            decoration: const InputDecoration(
                              isDense: true,
                              hintText: 'Search refs...',
                            ),
                          ),
                          const SizedBox(height: WsMetrics.space1),
                          if (running) const WarningNotice(stopFirstNotice),
                          if (checkout case final reference?)
                            _MenuRow(
                              leading: Icon(
                                LucideIcons.gitPullRequest,
                                size: 14,
                                color: ws.textMuted,
                              ),
                              name: Text('Checkout ${terms.singular}'),
                              detail: reference,
                              onTap: running
                                  ? null
                                  : () =>
                                        Navigator.of(context)
                                            .pop(_Checkout(reference)),
                            ),
                          if (refs != null)
                            for (final ref in refs.refs) _ref(ref, running),
                          if (create case final name?)
                            _MenuRow(
                              leading: Icon(
                                LucideIcons.gitBranch,
                                size: 14,
                                color: ws.textMuted,
                              ),
                              name: Text('Create new ref "$name"'),
                              onTap: running
                                  ? null
                                  : () =>
                                        Navigator.of(context)
                                            .pop(_Create(name)),
                            ),
                          if (refs == null)
                            const _Foot('Loading refs...')
                          else if (refs.refs.isEmpty &&
                              checkout == null &&
                              create == null)
                            const _Foot('No refs found.')
                          else if (refs.next != null)
                            _Foot(
                              'Showing ${refs.refs.length} of ${refs.total} refs',
                            ),
                        ],
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _ref(BridgeRef ref, bool running) {
    final ws = context.ws;
    return GestureDetector(
      onSecondaryTapUp: (details) => unawaited(
        _copyName(context, widget.state, ref.name, details.globalPosition),
      ),
      child: _MenuRow(
        leading: ref.current
            ? Icon(LucideIcons.check, size: 14, color: ws.accent)
            : const SizedBox.shrink(),
        name: Text(ref.shown),
        detail: switch ((ref.current, ref.worktree, ref.remote, ref.default_)) {
          (true, _, _, _) => 'current',
          (_, _?, _, _) => 'worktree',
          (_, _, _?, _) => 'remote',
          (_, _, _, true) => 'default',
          _ => null,
        },
        onTap: running
            ? null
            : () =>
                  Navigator.of(context).pop(ref.current ? null : _Switch(ref)),
      ),
    );
  }
}

class _MenuRow extends StatelessWidget {
  const _MenuRow({
    required this.leading,
    required this.name,
    this.detail,
    this.onTap,
  });

  final Widget leading;
  final Widget name;
  final String? detail;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      onTap: onTap,
      child: SizedBox(
        height: 28,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          child: Row(
            spacing: WsMetrics.space3,
            children: [
              SizedBox(width: 14, child: leading),
              Expanded(
                child: DefaultTextStyle.merge(
                  style: monoStyle(
                    context,
                    color: onTap == null ? ws.textFaint : ws.text,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  child: name,
                ),
              ),
              if (detail case final detail?)
                Flexible(
                  child: Text(
                    detail,
                    style: TextStyle(
                      color: ws.textFaint,
                      fontSize: WsMetrics.fsSmall,
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Foot extends StatelessWidget {
  const _Foot(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(
      WsMetrics.space3,
      WsMetrics.space1,
      WsMetrics.space3,
      WsMetrics.space1,
    ),
    child: Text(
      text,
      style: TextStyle(
        color: context.ws.textFaint,
        fontSize: WsMetrics.fsSmall,
      ),
    ),
  );
}
