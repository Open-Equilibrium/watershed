import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_graph.dart';

void main() {
  group('session graph', () {
    // e1 prompt → e2 response → e3 tool result → e4 response (tip A)
    //                         ↘ b1 prompt → b2 response (tip B)
    //                         ↘ c1 prompt (tip C)
    final tree = [
      node('e0', null, BridgeEntryKind.other),
      node('e1', 'e0', BridgeEntryKind.user),
      node('t1', 'e1', BridgeEntryKind.custom),
      node('e2', 't1', BridgeEntryKind.assistant),
      node('e3', 'e2', BridgeEntryKind.toolResult),
      node('e4', 'e3', BridgeEntryKind.assistant),
      node('b1', 'e2', BridgeEntryKind.user),
      node('b2', 'b1', BridgeEntryKind.assistant),
      node('c1', 'e2', BridgeEntryKind.user),
    ];

    test('numbers each row by its step on its own branch', () {
      final graph = sessionGraph(tree, 'b2');
      expect(
        [for (final row in graph.rows) (row.node.id, row.step)],
        [
          ('e1', 1),
          ('e2', 2),
          ('e3', 3),
          ('e4', 4),
          ('b1', 3),
          ('b2', 4),
          ('c1', 3),
        ],
        reason: 'the step numbers match the ledger of each branch',
      );
    });

    test('refers to each branch by its tip and follows first children', () {
      final graph = sessionGraph(tree, 'b2');
      expect(
        [for (final branch in graph.branches) (branch.ref, branch.tip)],
        [('A', 'e4'), ('B', 'b2'), ('C', 'c1')],
      );
      expect(graph.branchOf('e2')?.ref, 'A', reason: 'follows first children');
      expect(graph.branchOf('b1')?.ref, 'B');
    });

    test('keeps a row for a fork before the first prompt', () {
      final graph = sessionGraph([
        node('e0', null, BridgeEntryKind.other),
        node('e1', 'e0', BridgeEntryKind.user),
        node('x1', 'e0', BridgeEntryKind.user),
      ], 'x1');
      expect(
        [for (final row in graph.rows) (row.node.id, row.step)],
        [('e0', 0), ('e1', 1), ('x1', 1)],
      );
    });

    test('follows the first child of a leaf that has children', () {
      expect(sessionGraph(tree, 'e3').branchOf('e3')?.ref, 'A');
    });
  });

  test('marks where the current branch leaves the others', () {
    // e1 → e2 → e3 → e4 (A);  e2 → b1 → b2 (B)
    final tree = [
      node('e1', null, BridgeEntryKind.user),
      node('e2', 'e1', BridgeEntryKind.assistant),
      node('e3', 'e2', BridgeEntryKind.toolResult),
      node('e4', 'e3', BridgeEntryKind.assistant),
      node('b1', 'e2', BridgeEntryKind.user),
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
        node('b', 'a', BridgeEntryKind.user),
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
      node('x', null, BridgeEntryKind.user),
      node('x', 'x', BridgeEntryKind.assistant),
    ], 'x');
    final ids = [for (final row in graph.rows) row.node.id];
    expect(ids.toSet(), hasLength(ids.length), reason: 'each entry is one row');
  });
}

BridgeTreeNode node(String id, String? parent, BridgeEntryKind kind) =>
    BridgeTreeNode(id: id, parent: parent, kind: kind);
