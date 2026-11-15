import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

final _command = RegExp(r'^gh\s+pr\s+checkout\s+(\S+)$');
final _number = RegExp(r'^#?(\d+)$');

/// The number or https URL a typed pull request names, or null when it names none: `123`, `#123`,
/// either after `gh pr checkout`, or a URL. The bridge resolves it.
String? pullRequestReference(String input) {
  final text = input.trim();
  final argument = _command.firstMatch(text)?.group(1) ?? text;
  final number = _number.firstMatch(argument)?.group(1);
  if (number != null) return number;
  return argument.startsWith('https://') ? argument : null;
}

/// T3 Code's "Checkout pull request" dialog: resolves the reference typed, then checks the pull
/// request out in the project folder (Local) or in a worktree of its own (Worktree).
Future<void> showCheckoutPullRequestDialog(
  BuildContext context,
  SessionState state,
  BridgeGitStatus status, {
  String? reference,
}) => showDialog<void>(
  context: context,
  builder: (_) =>
      _CheckoutPullRequest(state: state, status: status, reference: reference),
);

class _CheckoutPullRequest extends StatefulWidget {
  const _CheckoutPullRequest({
    required this.state,
    required this.status,
    this.reference,
  });

  final SessionState state;
  final BridgeGitStatus status;
  final String? reference;

  @override
  State<_CheckoutPullRequest> createState() => _CheckoutPullRequestState();
}

class _CheckoutPullRequestState extends State<_CheckoutPullRequest> {
  static const _wait = Duration(milliseconds: 450);

  late final _input = TextEditingController(text: widget.reference);
  Timer? _timer;

  /// Counts the resolutions, so an answer for an earlier reference never lands.
  int _attempt = 0;
  bool _dirty = false;
  String? _reference;
  BridgeResolvedPullRequest? _pr;
  bool _resolving = false;
  String? _resolveError;

  /// `local` or `worktree` while that checkout runs.
  String? _preparing;
  String? _prepareError;

  @override
  void initState() {
    super.initState();
    final reference = widget.reference;
    if (reference != null) {
      _reference = pullRequestReference(reference);
      _resolving = _reference != null;
      if (_reference case final ref?) _schedule(ref);
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _input.dispose();
    super.dispose();
  }

  void _changed(String text) {
    _attempt++;
    _timer?.cancel();
    setState(() {
      _dirty = true;
      _pr = null;
      _resolveError = null;
      _reference = pullRequestReference(text);
      _resolving = _reference != null;
    });
    if (_reference case final ref?) _schedule(ref);
  }

  /// Resolves `reference` once the typing pauses, as T3 does.
  void _schedule(String reference) {
    _timer?.cancel();
    _timer = Timer(_wait, () => unawaited(_resolve(reference)));
  }

  Future<void> _resolve(String reference) async {
    final attempt = ++_attempt;
    setState(() => _resolving = true);
    try {
      final pr = await widget.state.git.value.backend.resolvePullRequest(
        reference,
      );
      if (!mounted || attempt != _attempt) return;
      setState(() {
        _pr = pr;
        _resolving = false;
      });
    } on Object catch (error) {
      if (!mounted || attempt != _attempt) return;
      setState(() {
        _resolving = false;
        _resolveError =
            'Could not resolve the ${terminologyFor(widget.status).singular}. $error';
      });
    }
  }

  bool get _ready => _pr != null && !_resolving && _preparing == null;

  Future<void> _checkOut({required bool worktree}) async {
    final reference = _reference;
    if (reference == null || !_ready) return;
    final state = widget.state;
    final singular = terminologyFor(widget.status).singular;
    setState(() {
      _preparing = worktree ? 'worktree' : 'local';
      _prepareError = null;
    });
    try {
      final checked = await state.git.value.backend.checkOutPullRequest(
        reference,
        worktree: worktree,
      );
      // T3 drafts a new thread on the checked-out branch.
      await state.draftFor(checked);
      if (mounted) Navigator.of(context).pop();
    } on Object catch (error) {
      if (!mounted) return;
      setState(() {
        _preparing = null;
        _prepareError = 'Failed to prepare $singular thread. $error';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final terms = terminologyFor(widget.status);
    final provider = widget.status.provider;
    final text = _input.text.trim();
    final validation = !_dirty
        ? null
        : text.isEmpty
        ? 'Paste a ${terms.singular} URL, checkout command, or enter 123 / #123.'
        : _reference == null
        ? 'Use a ${terms.singular} URL, checkout command, 123, or #123.'
        : null;
    final error = validation ?? _prepareError ?? _resolveError;
    return ActionDialog(
      title: 'Checkout ${terms.singular}',
      dismissible: _preparing == null,
      actions: [
        PrimaryButton(
          label: _preparing == 'worktree'
              ? 'Preparing worktree...'
              : 'Worktree',
          onPressed: _ready ? () => unawaited(_checkOut(worktree: true)) : null,
        ),
        SecondaryButton(
          label: _preparing == 'local' ? 'Preparing local...' : 'Local',
          onPressed: _ready
              ? () => unawaited(_checkOut(worktree: false))
              : null,
        ),
      ],
      children: [
        Text(
          'Resolve a ${provider == null ? '' : '$provider '}${terms.singular}, '
          'then create the draft thread in the main repo or in a dedicated worktree.',
          style: DangerDialog.text(context),
        ),
        Text(
          _capital(terms.singular),
          style: TextStyle(
            fontSize: WsMetrics.fsCompact,
            fontWeight: WsMetrics.fwMedium,
            color: ws.text,
          ),
        ),
        TextField(
          controller: _input,
          autofocus: true,
          onChanged: _changed,
          onSubmitted: (_) {
            if (_ready) unawaited(_checkOut(worktree: true));
          },
          decoration: InputDecoration(
            isDense: true,
            hintText: '${terms.short} URL, checkout command, or #42',
          ),
        ),
        if (_pr case final pr?) _Resolved(pr: pr),
        if (_resolving)
          Row(
            spacing: WsMetrics.space3,
            children: [
              const SizedBox.square(
                dimension: 14,
                child: CircularProgressIndicator(strokeWidth: 1.5),
              ),
              Text(
                'Resolving ${terms.singular}...',
                style: DangerDialog.text(context),
              ),
            ],
          ),
        if (error != null)
          Text(
            error,
            style: TextStyle(color: ws.danger, fontSize: WsMetrics.fsCompact),
          ),
      ],
    );
  }
}

/// The pull request the reference resolved to: its title, its branches and its state.
class _Resolved extends StatelessWidget {
  const _Resolved({required this.pr});

  final BridgeResolvedPullRequest pr;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(WsMetrics.space4),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  pr.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    fontWeight: WsMetrics.fwMedium,
                    color: ws.text,
                  ),
                ),
                Text(
                  '#${pr.number} · ${pr.head} to ${pr.base}',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textMuted,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space3),
          Text(
            _capital(pr.state),
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: pr.state == 'open' ? ws.success : ws.textMuted,
            ),
          ),
        ],
      ),
    );
  }
}

String _capital(String text) => '${text[0].toUpperCase()}${text.substring(1)}';
