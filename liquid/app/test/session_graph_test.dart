import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_graph.dart';

void main() {
  group('session graph', () {
    // e1 prompt → e2 response → e3 tool result → e4 response (tip A)
    //                         ↘ b1 prompt → b2 response (tip B)
    //                         ↘ c1 prompt (tip C, labelled)
    final tree = [
      node('e0', null, BridgeEntryKind.other),
      node(
        'e1',
        'e0',
        BridgeEntryKind.user,
        preview: 'Add rate limiting',
        at: 1000,
      ),
      node('t1', 'e1', BridgeEntryKind.custom),
      node('e2', 't1', BridgeEntryKind.assistant, preview: 'I will look.'),
      node('e3', 'e2', BridgeEntryKind.toolResult, preview: 'bash'),
      node('e4', 'e3', BridgeEntryKind.assistant, preview: 'Done.', at: 26900),
      node('b1', 'e2', BridgeEntryKind.user, preview: 'Use a token bucket'),
      node('b2', 'b1', BridgeEntryKind.assistant, preview: 'Ok.', at: 19500),
      node('c1', 'e2', BridgeEntryKind.user, preview: 'Fixed', label: 'fixed'),
    ];

    test('draws one dot per step and one lane per branch', () {
      final graph = sessionGraph(tree, 'b2');
      expect(
        [
          for (final row in graph.rows)
            (row.node.id, row.lane, row.parentRow, row.step),
        ],
        [
          ('e1', 0, null, 1),
          ('e2', 0, 0, 2),
          ('e3', 0, 1, 3),
          ('e4', 0, 2, 4),
          ('b1', 1, 1, 3),
          ('b2', 1, 4, 4),
          ('c1', 2, 1, 3),
        ],
        reason: 'the step numbers match the ledger of each branch',
      );
      expect(graph.lanes, 3);
      expect([graph.rows[0].turn, graph.rows[4].turn], [1, 2]);
    });

    test(
      'names branches by label or latest prompt, timed from the first prompt',
      () {
        final graph = sessionGraph(tree, 'b2');
        expect(
          [
            for (final branch in graph.branches)
              (branch.ref, branch.tip, branch.name, branch.durationMs),
          ],
          [
            ('A', 'e4', 'Add rate limiting', 25900),
            ('B', 'b2', 'Use a token bucket', 18500),
            ('C', 'c1', 'fixed', null),
          ],
        );
        expect(
          graph.branchOf('e2')?.ref,
          'A',
          reason: 'follows first children',
        );
        expect(graph.branchOf('b1')?.ref, 'B');
      },
    );

    test('connects branches that fork before the first prompt', () {
      final graph = sessionGraph([
        node('e0', null, BridgeEntryKind.other),
        node('e1', 'e0', BridgeEntryKind.user, preview: 'First'),
        node('x1', 'e0', BridgeEntryKind.user, preview: 'Instead'),
      ], 'x1');
      expect(
        [
          for (final row in graph.rows)
            (row.node.id, row.lane, row.parentRow, row.step),
        ],
        [('e0', 0, null, 0), ('e1', 0, 0, 1), ('x1', 1, 0, 1)],
      );
    });

    test('marks a leaf with children as the start of the next branch', () {
      final graph = sessionGraph(tree, 'e3');
      final row = graph.rows.singleWhere((row) => row.node.id == 'e3');
      expect((row.pending, row.branch), (true, null));
      expect(graph.branchOf('e3')?.ref, 'A');
    });

    test(
      'a duration counts from the first prompt when that prompt has no time',
      () {
        final graph = sessionGraph([
          node('u1', null, BridgeEntryKind.user, preview: 'Go'),
          node('a1', 'u1', BridgeEntryKind.assistant, at: 1000),
          node('u2', 'a1', BridgeEntryKind.user, preview: 'Again', at: 2000),
          node('a2', 'u2', BridgeEntryKind.assistant, at: 3000),
        ], 'a2');
        expect(
          graph.branches.single.durationMs,
          2000,
          reason:
              'the base is the first timestamp on the path, not a later prompt',
        );
      },
    );

    test('a branch whose tip is stamped before its prompt lasts 0 ms', () {
      final graph = sessionGraph([
        node('u1', null, BridgeEntryKind.user, preview: 'Go', at: 2000),
        node('a1', 'u1', BridgeEntryKind.assistant, at: 1000),
      ], 'a1');
      expect(graph.branches.single.durationMs, 0);
    });

    test(
      'a duration starts at the time just before an untimed first prompt',
      () {
        final graph = sessionGraph([
          node('e1', null, BridgeEntryKind.system, at: 1000),
          node('e2', 'e1', BridgeEntryKind.system, at: 2000),
          node('u1', 'e2', BridgeEntryKind.user, preview: 'Read it'),
          node('a1', 'u1', BridgeEntryKind.assistant, at: 3000),
        ], 'a1');
        expect(
          graph.branches.single.durationMs,
          1000,
          reason:
              'the core takes the latest time before the prompt as its base',
        );
      },
    );
  });

  test('marks where the current branch leaves the others', () {
    // e1 → e2 → e3 → e4 (A);  e2 → b1 → b2 (B)
    final tree = [
      node('e1', null, BridgeEntryKind.user, preview: 'Go'),
      node('e2', 'e1', BridgeEntryKind.assistant),
      node('e3', 'e2', BridgeEntryKind.toolResult, preview: 'read'),
      node('e4', 'e3', BridgeEntryKind.assistant),
      node('b1', 'e2', BridgeEntryKind.user, preview: 'Instead'),
      node('b2', 'b1', BridgeEntryKind.assistant),
    ];
    final marker = branchMarker(tree, 'b2')!;
    expect(
      (marker.fork, marker.ref, marker.step, marker.other, marker.otherMore),
      ('e2', 'B', 2, 'A', 2),
    );
    final pending = branchMarker(tree, 'e2')!;
    expect(
      (pending.fork, pending.ref, pending.step, pending.other),
      ('e2', null, 2, 'A'),
    );
    expect(branchMarker(tree.take(4).toList(), 'e4'), isNull);
  });

  test('a stored parent cycle returns from the branch marker', () {
    // The stored parents loop, so walks up the tree must stop at a repeated entry.
    expect(
      () => branchMarker([
        node('a', 'b', BridgeEntryKind.assistant),
        node('b', 'a', BridgeEntryKind.user, preview: 'Go'),
      ], 'a'),
      returnsNormally,
    );
    expect(
      () => branchMarker([node('s', 's', BridgeEntryKind.assistant)], 's'),
      returnsNormally,
    );
  });

  test('a repeated entry id that is its own parent builds a finite graph', () {
    // Both entries are named x, and the second one is stored as its own parent.
    final graph = sessionGraph([
      node('x', null, BridgeEntryKind.user, preview: 'Go'),
      node('x', 'x', BridgeEntryKind.assistant),
    ], 'x');
    final ids = [for (final row in graph.rows) row.node.id];
    expect(ids.toSet(), hasLength(ids.length), reason: 'each entry is one row');
  });
}

BridgeTreeNode node(
  String id,
  String? parent,
  BridgeEntryKind kind, {
  String preview = '',
  String? label,
  int? at,
}) => BridgeTreeNode(
  failed: false,
  id: id,
  parent: parent,
  kind: kind,
  preview: preview,
  label: label,
  atMs: at,
);
