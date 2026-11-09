import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'cell_pickers.dart';
import 'field_menu.dart';
import 'field_panel.dart';
import 'row_menu.dart';
import 'table_cells.dart';
import 'table_selection.dart';
import 'follow.dart';
import 'page_views.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

/// Rows a Table shows before it scrolls inside, as the Chat Block keeps a fixed height.
const _shownRows = 10;
const _headerHeight = 32.0;
const _footerHeight = 30.0;

/// The narrowest the first field gets; it takes the width the others leave.
const _firstWidth = 200.0;

/// The column after the last field, with + for a new one.
const _addWidth = 40.0;

double _width(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 180,
  BridgeFieldKind.number => 104,
  BridgeFieldKind.select => 128,
  BridgeFieldKind.date => 120,
  BridgeFieldKind.checkbox => 72,
};

/// A Table on its Page, as the approved Table View mockups show it: its name, a header of
/// its fields, its rows, read in windows and scrolling inside a fixed height, and their
/// count. Fields wider than the reading column scroll sideways with the first field in
/// view. It follows only its own Block (client architecture rule 6).
class TableBlock extends StatefulWidget {
  const TableBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<TableBlock> createState() => _TableBlockState();
}

class _TableBlockState extends State<TableBlock> {
  BridgeDatabase? _database;
  late final _rows = WindowedList<BridgeRow>(
    (start, limit) async {
      final answer = await _state.workspace.query(
        query: query(
          BridgeQueryKind.rows,
          page: _page,
          block: _block,
          start: start,
          limit: limit,
        ),
      );
      return (answer.count, answer.rows);
    },
    _report,
    kept: 5,
  );
  final _sideways = ScrollController();
  final _down = ScrollController();

  /// The field panel + opens, under that +.
  final _panel = OverlayPortalController();
  final _plus = LayerLink();

  /// After + New the rows scroll to their end on a read begun once every row add on its
  /// way returned; the Table starts that read itself, so it holds the rows added.
  var _rowAdded = false;
  int _rowsWriting = 0;
  int _readsAtRowAdd = 0;

  /// The field the last add made, shown once a read holds it.
  String? _fieldAdded;

  /// The reads started when the last field add returned; a later one without it means
  /// the field went elsewhere.
  int _readsAtFieldAdd = 0;

  late final _cells = CellSelection(
    rowAt: _rows.at,
    rowCount: () => _rows.count ?? 0,
    fields: () => _database?.fields ?? const [],
    write: _setCell,
    reveal: _reveal,
    leave: _leave,
    // Also from dispose, so after the frame, when the Page may show it.
    report: (problem) => scheduleMicrotask(() => _state.report(problem)),
    addOption: _addOption,
  )..addListener(_changed);

  /// The grid's geometry as last built, to reveal a cell.
  List<double> _starts = const [0];
  double _first = 0;

  /// Times the field panel opened, so an add closes only the panel it came from.
  int _panelOpened = 0;

  /// Field adds on their way; + they push out of view comes back with the scroll to them.
  int _fieldsWriting = 0;

  /// Whether to scroll to the last row or field once the next build laid it out.
  var _toLastRow = false;
  var _toLastField = false;
  StreamSubscription<BridgeChange>? _followed;

  /// The name as typed; a name changed elsewhere shows unless the person is typing one.
  final _name = TextEditingController();
  final _nameFocus = FocusNode(debugLabel: 'Table name');

  /// Renames asked for, so only the latest refusal puts the stored name back.
  int _renames = 0;

  /// The name as last read or written, which typing changes once a composition ends.
  String _named = '';

  /// The renames sent when the name shown last was read.
  int _readSent = 0;
  StreamSubscription<String>? _settled;

  /// Reads asked for, so only the latest shows.
  int _reads = 0;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _rows.addListener(_changed);
    // A sideways scroll moves + away from the field panel, which closes.
    _sideways
      ..addListener(_closePanel)
      ..addListener(_scrolled);
    _down.addListener(_scrolled);
    _name.addListener(_typed);
    _state.finishing.add(_typed);
    _state.finishing.add(_cells.finish);
    _state.finishing.add(_fieldMenus.renameTyped);
    _nameFocus.addListener(_left);
    // Once every rename arrived, the name is read again.
    _settled = widget.views.settled
        .where((settled) => settled == _block)
        .listen((_) => unawaited(_read()));
    widget.views.caret.addListener(_takeCaret);
    // Built after the caret was handed to it, as a new Table is.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _takeCaret();
    });
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
  }

  /// The Page's scroll and the window's size, which move the Table and so close what
  /// hangs under it, as Flutter's menus close.
  ScrollPosition? _pageScroll;
  Size? _window;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final window = MediaQuery.sizeOf(context);
    if (_window != null && window != _window) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _moved();
      });
    }
    _window = window;
    final scroll = Scrollable.maybeOf(context)?.position;
    if (scroll == _pageScroll) return;
    _pageScroll?.removeListener(_moved);
    _pageScroll = scroll?..addListener(_moved);
  }

  void _moved() {
    if (_scrollingItself) return _replacePick();
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
    _rowHandles.closeMenu();
    _cells.closePick();
  }

  @override
  void dispose() {
    _pageScroll?.removeListener(_moved);
    // A composition still open is written as it stands.
    _write();
    _state.finishing.remove(_typed);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    _rows.dispose();
    _sideways.dispose();
    _down.dispose();
    _name.dispose();
    _nameFocus.dispose();
    // A name typed in a field's menu is kept as it stands.
    _fieldMenus.renameTyped();
    _state.finishing.remove(_fieldMenus.renameTyped);
    _fieldMenus.dispose();
    _rowHandles.dispose();
    // An edit still open is written as it stands.
    _cells.removeListener(_changed);
    _cells.finish();
    _state.finishing.remove(_cells.finish);
    _cells.dispose();
    super.dispose();
  }

  void _panelOutOfView() {
    if (!_panel.isShowing || _fieldsWriting > 0) return;
    // Checked once laid out, after any scroll to a new field, which brings + back.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_sideways.hasClients) return;
      final position = _sideways.position;
      if (position.pixels <= position.maxScrollExtent - _addWidth) {
        _closePanel();
      }
    });
  }

  void _closePanel() {
    if (_scrollingItself) return;
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
  }

  /// While the Table scrolls itself, to a new row or field or to reveal a cell, which
  /// moves + and a picker's cell along.
  var _scrollingItself = false;

  /// The person scrolling the rows closes a select cell's picker, as its cell moves away.
  void _scrolled() {
    if (_scrollingItself) return _replacePick();
    _cells.closePick();
    _rowHandles.closeMenu();
  }

  /// Chooses the picker's side and room again once the Table is laid out, after the
  /// Table moved its cell.
  void _replacePick() {
    if (!_cells.picking) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) setState(() {});
    });
  }

  void _changed() {
    if (!mounted) return;
    int? rowIndex(String id) => _rows.indexWhere((row) => row.id == id);
    _cells
      ..relocate(rowIndex)
      ..read(rowIndex);
    _rowHandles.read();
    setState(() {});
    _replacePick();
    if (_cells.picking != _pick.isShowing) {
      _cells.picking ? _pick.show() : _pick.hide();
    }
  }

  /// A hovered row's handle, left of the Table, and the row's menu (approved Table View
  /// mockups, section 5).
  late final _rowHandles = RowHandles(
    indexOf: (row) => _rows.indexWhere((shown) => shown.id == row),
    idAt: (index) async => (await _rows.load(index))?.id,
    insert: (after) => _writeRows(
      mutation(
        BridgeMutationKind.addRow,
        page: _page,
        block: _block,
        after: after,
      ),
    ),
    remove: (row) => _writeRows(
      mutation(
        BridgeMutationKind.removeRow,
        page: _page,
        block: _block,
        row: row,
      ),
    ),
  )..addListener(_menusChanged);

  /// Writes a change to rows; a refused one shows on the Page.
  Future<void> _writeRows(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {}, onError: (Object _) {});

  /// A select cell's picker, under the cell or above it, which a click outside closes.
  final _pick = OverlayPortalController();
  final _pickAnchor = LayerLink();
  final _pickCell = GlobalKey();

  Future<String> _addOption(BridgeField field, String name, int color) async {
    final added = await widget.views.write(
      mutation(
        BridgeMutationKind.addOption,
        page: _page,
        block: _block,
        field: field.id,
        text: name,
        color: color,
      ),
      const [],
    );
    return added.option!;
  }

  Widget _picker(BuildContext context) {
    final field = _cells.picked;
    if (field == null) return const SizedBox.shrink();
    final cell = _pickCell.currentContext?.findRenderObject() as RenderBox?;
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    // The picker shows once its cell is laid out, which tells so.
    if (cell == null || !cell.hasSize || overlay == null) {
      return const SizedBox.shrink();
    }
    // It hangs under the cell, or above it when all options do not fit under it and
    // there is more room above; they scroll in the room it has.
    final top = cell.localToGlobal(Offset.zero, ancestor: overlay).dy;
    final below =
        overlay.size.height - top - cell.size.height - 2 * WsMetrics.space1;
    final above = top - 2 * WsMetrics.space1;
    final date = field.kind == BridgeFieldKind.date;
    final height = date
        ? DatePicker.height
        : SelectPicker.heightFor(field.options.length);
    final up = height > below && above > below;
    return Align(
      alignment: Alignment.topLeft,
      child: CompositedTransformFollower(
        link: _pickAnchor,
        showWhenUnlinked: false,
        targetAnchor: up ? Alignment.topLeft : Alignment.bottomLeft,
        followerAnchor: up ? Alignment.bottomLeft : Alignment.topLeft,
        offset: Offset(0, up ? -WsMetrics.space1 : WsMetrics.space1),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: math.max(0, up ? above : below),
          ),
          child: TapRegion(
            groupId: _pick,
            onTapOutside: (_) => _cells.leavePick(),
            // A click in the picker beside its field keeps the field's keys.
            child: TextFieldTapRegion(
              child: date
                  ? DatePicker(
                      text: _cells.pickText,
                      locale: _cells.days,
                      onDay: _cells.pickDay,
                      onEnter: _cells.enterDay,
                      onClear: () => _cells.pickDay(null),
                      onClose: _cells.closePick,
                      onTab: _cells.tabFromPick,
                      onKeys: (keys) => _cells.pickKeys = keys,
                    )
                  : SelectPicker(
                      field: field,
                      text: _cells.pickText,
                      onPick: _cells.pick,
                      onCreate: _cells.create,
                      onClose: _cells.closePick,
                      onTab: _cells.tabFromPick,
                      onKeys: (keys) => _cells.pickKeys = keys,
                    ),
            ),
          ),
        ),
      ),
    );
  }

  /// Brings the picked cell into the Page's view, as an editor's caret does, and shows
  /// its picker.
  void _placed() {
    if (!mounted || !_cells.picking) return;
    _scrollingItself = true;
    _pickCell.currentContext?.findRenderObject()?.showOnScreen();
    _scrollingItself = false;
    setState(() {});
  }

  /// + New adds a row last, which the core places as it applies the write (ADR-0247),
  /// and shows it.
  Future<void> _addRow() async {
    _rowsWriting++;
    try {
      await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          last: true,
        ),
        const [],
      );
      _rowAdded = true;
    } on Object catch (error) {
      _report(error);
    } finally {
      _rowsWriting--;
      _readsAtRowAdd = _reads;
      if (_rowAdded && _rowsWriting == 0 && mounted) unawaited(_read());
    }
  }

  void _togglePanel() {
    if (_panel.isShowing) return _panel.hide();
    _fieldMenus.close();
    _panelOpened++;
    _panel.show();
  }

  /// A field's menu, under its header (approved Table View mockups, section 5).
  late final _fieldMenus = FieldMenus(
    fieldOf: (id) =>
        _database?.fields.where((field) => field.id == id).firstOrNull,
    rename: (field, name) => unawaited(
      _writeField(
        mutation(
          BridgeMutationKind.renameField,
          page: _page,
          block: _block,
          field: field,
          text: name,
        ),
      ),
    ),
    remove: (field) => _writeField(
      mutation(
        BridgeMutationKind.removeField,
        page: _page,
        block: _block,
        field: field,
      ),
    ),
    confirm: (field) => confirmDeleteField(context, field, _rows.count),
  )..addListener(_menusChanged);

  /// Writes a change to a field and reads the Table again; a refused one shows on the
  /// Page.
  Future<void> _writeField(BridgeMutation change) =>
      widget.views.write(change, const []).then((_) {
        if (mounted) unawaited(_read());
      }, onError: (Object _) {});

  void _menusChanged() {
    if (mounted) setState(() {});
  }

  /// Adds a field last (ADR-0247) and shows it once a read holds it; a refused one keeps
  /// the panel open.
  Future<void> _addField(String name, BridgeFieldKind kind) async {
    final opened = _panelOpened;
    final BridgeApplied added;
    _fieldsWriting++;
    try {
      added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: name,
          fieldKind: kind,
          last: true,
        ),
        const [],
      );
    } finally {
      _fieldsWriting--;
      // + may have left the view meanwhile; the header checks again.
      if (mounted && _panel.isShowing) setState(() {});
    }
    if (!mounted) return;
    if (opened == _panelOpened) _panel.hide();
    if (_database?.fields.any((field) => field.id == added.field) ?? false) {
      setState(() => _toLastField = true);
    } else {
      // A read begun now holds the field, or it went elsewhere.
      _fieldAdded = added.field;
      _readsAtFieldAdd = _reads;
      unawaited(_read());
    }
  }

  Future<void> _setCell(BridgeRow row, BridgeField field, BridgeCell cell) =>
      widget.views.write(
        mutation(
          BridgeMutationKind.setCell,
          page: _page,
          block: _block,
          row: row.id,
          field: field.id,
          cell: cell,
        ),
        const [],
      );

  /// Scrolls a cell into view, below and beside the first field.
  void _reveal(int row, int field) =>
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _scrollingItself = true;
        if (_down.hasClients) {
          final top = row * WsMetrics.rowTable;
          final view = _down.position.viewportDimension;
          if (top < _down.offset) {
            _down.jumpTo(top);
          } else if (top + WsMetrics.rowTable > _down.offset + view) {
            _down.jumpTo(top + WsMetrics.rowTable - view);
          }
        }
        _scrollingItself = false;
        if (_sideways.hasClients && field > 0 && field + 1 < _starts.length) {
          final view = _sideways.position.viewportDimension;
          final (left, right) = (_starts[field], _starts[field + 1]);
          _scrollingItself = true;
          if (left < _sideways.offset + _first) {
            _sideways.jumpTo(left - _first);
          } else if (right > _sideways.offset + view) {
            _sideways.jumpTo(right - view);
          }
          _scrollingItself = false;
        }
      });

  /// esc leaves the Table; the caret goes to the Text Block below it, if any.
  void _leave() {
    final views = widget.views;
    final at = views.indexOf(widget.view.id);
    final next = at == null ? null : views.textFrom(at + 1, 1);
    if (next != null) {
      views.moveCaret(Caret(next.id, 0));
    } else {
      _cells.cellsFocus.unfocus();
    }
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// A caret handed to this Table goes to its name, as for a new Table.
  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null || caret.view != widget.view.id) return;
    widget.views.caret.value = null;
    // What was typed in the line it took the place of is its name.
    final (text, written) =
        widget.views.naming.remove(widget.view.id)?.call() ?? ('', false);
    if (text.isNotEmpty) {
      if (written) _named = text;
      _name.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      );
    }
    _nameFocus.requestFocus();
  }

  /// Each change of the name, once any composition ends, is one rename; a refused one puts
  /// the stored name back.
  void _typed() {
    // On quitting, a composition still open is written as it stands.
    if (!_name.value.composing.isCollapsed && !_state.quitting) return;
    _write();
  }

  void _write() {
    final name = _name.text;
    if (name == _named) return;
    _named = name;
    final rename = ++_renames;
    unawaited(
      widget.views
          .write(
            mutation(
              BridgeMutationKind.renameDatabase,
              page: _page,
              block: _block,
              text: name,
            ),
            [_block],
          )
          .then<void>(
            (_) {},
            onError: (Object _) {
              if (!mounted || rename != _renames) return;
              _show(_database?.name ?? '');
            },
          ),
    );
  }

  /// Leaving the name writes a composition still open as it stands; a name changed
  /// elsewhere meanwhile then shows, unless the person's own rename is on its way.
  void _left() {
    if (_nameFocus.hasFocus) return;
    _write();
    final views = widget.views;
    if (views.pending(_block) > 0 || views.sent(_block) != _readSent) return;
    _show(_database?.name ?? '');
  }

  void _show(String name) {
    _named = name;
    if (_name.text != name) _name.text = name;
  }

  /// Reads the name and fields, then the rows read so far.
  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    // A read that crosses a rename on its way may miss it, whenever it answers.
    final clean = views.pending(_block) == 0;
    final sent = views.sent(_block);
    try {
      final answer = await _state.workspace.query(
        query: query(BridgeQueryKind.database, page: _page, block: _block),
      );
      if (!mounted || read != _reads) return;
      if (_fieldAdded case final field?) {
        // A read that holds it shows it; one begun after the add without it means the
        // field went elsewhere, so there is nothing to show.
        if (answer.database?.fields.any((shown) => shown.id == field) ??
            false) {
          _fieldAdded = null;
          _toLastField = true;
        } else if (read > _readsAtFieldAdd) {
          _fieldAdded = null;
        }
      }
      if (clean && views.pending(_block) == 0 && sent == views.sent(_block)) {
        _readSent = sent;
        if (!_nameFocus.hasFocus) _show(answer.database?.name ?? '');
      }
      setState(() => _database = answer.database);
      _fieldMenus.read();
      _cells.relocate((id) => _rows.indexWhere((row) => row.id == id));
    } on Object catch (error) {
      // What an add made is not shown later by surprise.
      if (read > _readsAtRowAdd) _rowAdded = false;
      if (read > _readsAtFieldAdd) _fieldAdded = null;
      _report(error);
      return;
    }
    await _rows.reread();
    if (mounted && _rowAdded && _rowsWriting == 0 && read > _readsAtRowAdd) {
      _rowAdded = false;
      setState(() => _toLastRow = true);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final database = _database;
    final locale = Localizations.localeOf(context);
    final numbers = intlLocale(locale, NumberFormat.localeExists);
    _cells.numbers = NumberFormat.decimalPattern(numbers);
    final days = intlLocale(locale, DateFormat.localeExists);
    _cells.days = days;
    final count = _rows.count ?? 0;
    if (_toLastRow || _toLastField) {
      final (down, sideways) = (_toLastRow, _toLastField);
      _toLastRow = _toLastField = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        // The rows' end follows from their fixed height.
        if (down && _down.hasClients) {
          _scrollingItself = true;
          _down.jumpTo(
            (count - math.min(count, _shownRows)) * WsMetrics.rowTable,
          );
          _scrollingItself = false;
        }
        if (sideways && _sideways.hasClients) {
          _scrollingItself = true;
          _sideways.jumpTo(_sideways.position.maxScrollExtent);
          _scrollingItself = false;
        }
      });
    }
    // The field panel opens under +, and clicking it or + keeps it open.
    return OverlayPortal(
      controller: _rowHandles.handle,
      overlayChildBuilder: _rowHandles.handleView,
      child: OverlayPortal(
        controller: _rowHandles.menu,
        overlayChildBuilder: _rowHandles.menuView,
        child: OverlayPortal(
          controller: _panel,
          overlayChildBuilder: (context) => Align(
            alignment: Alignment.topLeft,
            child: CompositedTransformFollower(
              link: _plus,
              showWhenUnlinked: false,
              targetAnchor: Alignment.bottomRight,
              followerAnchor: Alignment.topRight,
              offset: const Offset(0, WsMetrics.space1),
              child: TapRegion(
                groupId: _panel,
                onTapOutside: (_) => _panel.hide(),
                // A click in the panel beside its field keeps the field's keys.
                child: TextFieldTapRegion(
                  child: FieldPanel(onAdd: _addField, onClose: _panel.hide),
                ),
              ),
            ),
          ),
          child: OverlayPortal(
            controller: _fieldMenus.overlay,
            overlayChildBuilder: _fieldMenus.view,
            child: OverlayPortal(
              controller: _pick,
              overlayChildBuilder: _picker,
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: WsMetrics.space3),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    SizedBox(
                      height: 32,
                      child: Row(
                        children: [
                          Icon(
                            LucideIcons.table,
                            size: 16,
                            color: ws.textFaint,
                          ),
                          const SizedBox(width: WsMetrics.space3),
                          Expanded(
                            child: TextField(
                              controller: _name,
                              focusNode: _nameFocus,
                              // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
                              selectAllOnFocus: false,
                              style: TextStyle(
                                fontSize: WsMetrics.fsProse,
                                fontWeight: WsMetrics.fwSemibold,
                                color: ws.text,
                              ),
                              decoration: InputDecoration.collapsed(
                                hintText: 'Untitled',
                                hintStyle: TextStyle(color: ws.textFaint),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                    if (database != null)
                      Focus(
                        focusNode: _cells.cellsFocus,
                        // The keys of a selected cell, without a node of its own for readers.
                        includeSemantics: false,
                        child: ClipRRect(
                          borderRadius: BorderRadius.circular(
                            WsMetrics.radiusLg,
                          ),
                          child: Container(
                            foregroundDecoration: BoxDecoration(
                              border: Border.all(color: ws.border),
                              borderRadius: BorderRadius.circular(
                                WsMetrics.radiusLg,
                              ),
                            ),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                                // A Table may lose all its fields, as when another device removes
                                // them; its rows still count.
                                if (database.fields.isEmpty)
                                  Container(
                                    height: _headerHeight,
                                    color: ws.subtle,
                                    alignment: Alignment.centerRight,
                                    child: SizedBox(
                                      width: _addWidth,
                                      child: _AddField(
                                        link: _plus,
                                        panel: _panel,
                                        onTap: _togglePanel,
                                      ),
                                    ),
                                  )
                                else
                                  LayoutBuilder(
                                    builder: (context, constraints) => _grid(
                                      database.fields,
                                      constraints.maxWidth,
                                      NumberFormat.decimalPattern(numbers),
                                      shownDays(days),
                                    ),
                                  ),
                                _NewRow(onTap: () => unawaited(_addRow())),
                                if (count > 0)
                                  Container(
                                    height: _footerHeight,
                                    color: ws.subtle,
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: 10,
                                    ),
                                    alignment: Alignment.centerLeft,
                                    child: Text(
                                      '${NumberFormat.decimalPattern(numbers).format(count)} '
                                      '${count == 1 ? 'row' : 'rows'}',
                                      style: TextStyle(
                                        fontSize: WsMetrics.fsCompact,
                                        color: ws.textFaint,
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    // A value refused in place says why, under the Table (approved mockups).
                    if (_cells.error case final error?)
                      Padding(
                        padding: const EdgeInsets.only(top: WsMetrics.space2),
                        child: Text(
                          error,
                          style: TextStyle(
                            fontSize: WsMetrics.fsSmall,
                            color: ws.danger,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// The header and the rows, scrolling sideways together when wider than `width`.
  Widget _grid(
    List<BridgeField> fields,
    double width,
    NumberFormat numbers,
    DateFormat days,
  ) {
    final ws = context.ws;
    final rest = [
      for (final field in fields.skip(1)) _width(field.kind),
      _addWidth,
    ];
    final others = rest.fold(0.0, (sum, width) => sum + width);
    final first = math.max(_firstWidth, width - others);
    final widths = [first, ...rest];
    // Where each field starts, to build only the fields in view (client architecture
    // rule 8).
    final starts = [0.0];
    for (final width in widths) {
      starts.add(starts.last + width);
    }
    _starts = starts;
    _first = first;
    final count = _rows.count ?? 0;
    final options = {
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select)
          for (final option in field.options) option.id: option,
    };
    Widget content(BridgeField field, BridgeCell? cell) => CellView(
      cell: cell,
      kind: field.kind,
      options: options,
      numbers: numbers,
      days: days,
    );

    Widget line({
      required double height,
      required Color? color,
      required Widget Function(int index) cell,
      VoidCallback? addOutOfView,
      Decoration? Function(int index)? mark,
      void Function(int index)? onTap,
      Key? Function(int index)? keyOf,
      bool Function(int index)? editing,
      bool Function(int index)? picking,
      bool Function(int index)? menu,
      LayerLink? handle,
    }) => ListenableBuilder(
      listenable: _sideways,
      builder: (context, _) {
        final offset = _sideways.hasClients ? _sideways.offset : 0.0;
        Widget box(int index) {
          final box = Container(
            key: keyOf?.call(index),
            width: widths[index],
            height: height,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            foregroundDecoration: mark?.call(index),
            decoration: BoxDecoration(
              color: color,
              border: index > 0
                  ? Border(left: BorderSide(color: ws.border))
                  : null,
              boxShadow: index == 0 && offset > 0
                  ? const [
                      BoxShadow(
                        color: Color(0x40000000),
                        blurRadius: 6,
                        offset: Offset(4, 0),
                        spreadRadius: -4,
                      ),
                    ]
                  : null,
            ),
            child: cell(index),
          );
          if (onTap == null) return box;
          final tapped = GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => onTap(index),
            child: box,
          );
          // A field's menu hangs under its header, and a click on it is no click
          // outside.
          if (menu?.call(index) ?? false) {
            return TapRegion(
              groupId: _fieldMenus.overlay,
              child: CompositedTransformTarget(
                link: _fieldMenus.anchor,
                child: tapped,
              ),
            );
          }
          // The picker hangs under its cell, and a click in it is no click outside.
          if (picking?.call(index) ?? false) {
            return TapRegion(
              groupId: _pick,
              // Nor does it take the keys from the picker's filter.
              child: TextFieldTapRegion(
                child: _PickedCell(
                  onPlaced: _placed,
                  child: CompositedTransformTarget(
                    key: _pickCell,
                    link: _pickAnchor,
                    child: tapped,
                  ),
                ),
              ),
            );
          }
          // A click in the edited cell beside its text is no click outside the editor.
          return editing?.call(index) ?? false
              ? TextFieldTapRegion(child: tapped)
              : tapped;
        }

        // The first field stays in view while the others scroll sideways; fields out of
        // view are not built.
        var from = 1;
        while (from < widths.length && starts[from + 1] <= offset + first) {
          from++;
        }
        var to = from;
        while (to < widths.length && starts[to] < offset + width) {
          to++;
        }
        if (to < widths.length) addOutOfView?.call();
        return Stack(
          children: [
            Row(
              children: [
                // Also gives the line its height when only the first field is built.
                SizedBox(width: starts[from], height: height),
                for (var index = from; index < to; index++) box(index),
              ],
            ),
            Positioned(
              left: offset,
              top: 0,
              bottom: 0,
              // A hovered row's handle hangs left of its first field.
              child: handle == null
                  ? box(0)
                  : CompositedTransformTarget(link: handle, child: box(0)),
            ),
          ],
        );
      },
    );

    return SingleChildScrollView(
      controller: _sideways,
      scrollDirection: Axis.horizontal,
      child: SizedBox(
        width: first + others,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Over the cells, so the first field's cell keeps the line too.
            DecoratedBox(
              position: DecorationPosition.foreground,
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: DefaultTextStyle.merge(
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textMuted,
                ),
                child: line(
                  height: _headerHeight,
                  color: ws.subtle,
                  // The field panel follows +, so it closes once + is out of view.
                  addOutOfView: _panelOutOfView,
                  onTap: (index) {
                    if (index >= fields.length) return;
                    if (_panel.isShowing) _panel.hide();
                    _fieldMenus.toggle(fields[index]);
                  },
                  menu: (index) =>
                      index < fields.length &&
                      fields[index].id == _fieldMenus.open,
                  cell: (index) => index == fields.length
                      ? _AddField(
                          link: _plus,
                          panel: _panel,
                          onTap: _togglePanel,
                        )
                      : Row(
                          children: [
                            Icon(
                              fieldKindIcon(fields[index].kind),
                              size: 14,
                              color: ws.textFaint,
                            ),
                            const SizedBox(width: WsMetrics.space2),
                            Flexible(child: cellText(fields[index].name)),
                          ],
                        ),
                ),
              ),
            ),
            SizedBox(
              height: math.min(count, _shownRows) * WsMetrics.rowTable,
              child: ListView.builder(
                key: const Key('table rows'),
                controller: _down,
                itemCount: count,
                itemExtent: WsMetrics.rowTable,
                itemBuilder: (context, rowIndex) {
                  final row = _rows.at(rowIndex);
                  final cells = {
                    for (final cell in row?.cells ?? const <BridgeFieldCell>[])
                      cell.field: cell.value,
                  };
                  final hovered =
                      row != null && row.id == _rowHandles.handleRow;
                  final shown = DecoratedBox(
                    position: DecorationPosition.foreground,
                    decoration: BoxDecoration(
                      border: Border(bottom: BorderSide(color: ws.border)),
                    ),
                    child: line(
                      height: WsMetrics.rowTable,
                      color: hovered && _rowHandles.handle.isShowing
                          ? ws.hover
                          : ws.canvas,
                      handle: hovered ? _rowHandles.handleAnchor : null,
                      keyOf: (index) => row == null || index == fields.length
                          ? null
                          : ValueKey('cell ${row.id} ${fields[index].id}'),
                      onTap: row == null
                          ? null
                          : (index) {
                              if (index < fields.length) {
                                _cells.tap(rowIndex, index);
                              }
                            },
                      mark: (index) => _cells.mark(context, rowIndex, index),
                      picking: (index) =>
                          _cells.picking &&
                          _cells.selected == (rowIndex, index),
                      editing: (index) =>
                          _cells.editing &&
                          _cells.selected == (rowIndex, index),
                      cell: (index) {
                        if (index == fields.length) {
                          return const SizedBox.shrink();
                        }
                        final field = fields[index];
                        if (_cells.editing &&
                            _cells.selected == (rowIndex, index) &&
                            row != null) {
                          return _cells.editor(context, field);
                        }
                        return Align(
                          alignment: switch (field.kind) {
                            BridgeFieldKind.number => Alignment.centerRight,
                            BridgeFieldKind.checkbox => Alignment.center,
                            _ => Alignment.centerLeft,
                          },
                          child: content(field, cells[field.id]),
                        );
                      },
                    ),
                  );
                  if (row == null) return shown;
                  // A hovered row shows its handle (section 5).
                  // Keyed by row, so a row moving under a still pointer is entered anew.
                  return HoverRegion(
                    key: ValueKey('row ${row.id}'),
                    onHover: (hovered, region) =>
                        _rowHandles.hoverRow(row.id, hovered, region),
                    child: shown,
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// + after the last field, which opens the field panel.
class _AddField extends StatelessWidget {
  const _AddField({
    required this.link,
    required this.panel,
    required this.onTap,
  });

  final LayerLink link;
  final OverlayPortalController panel;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => CompositedTransformTarget(
    link: link,
    child: TapRegion(
      groupId: panel,
      child: Semantics(
        button: true,
        label: 'Add field',
        child: GestureDetector(
          key: const Key('add field'),
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Center(
            child: Icon(
              LucideIcons.plus,
              size: 14,
              color: context.ws.textMuted,
            ),
          ),
        ),
      ),
    ),
  );
}

/// + New under the last row, which adds a row there.
class _NewRow extends StatelessWidget {
  const _NewRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          height: WsMetrics.rowTable,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: Row(
            children: [
              Icon(LucideIcons.plus, size: 14, color: ws.textFaint),
              const SizedBox(width: WsMetrics.space2),
              Text(
                'New',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textFaint,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The cell a select picker hangs from, which tells once it is first laid out.
class _PickedCell extends StatefulWidget {
  const _PickedCell({required this.onPlaced, required this.child});

  final VoidCallback onPlaced;
  final Widget child;

  @override
  State<_PickedCell> createState() => _PickedCellState();
}

class _PickedCellState extends State<_PickedCell> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) widget.onPlaced();
    });
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
