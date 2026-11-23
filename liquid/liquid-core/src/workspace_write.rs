use automerge::ROOT;
use automerge::transaction::Transactable;

use crate::model::{self, ICON, NAME};
use crate::store::Store;
use crate::workspace::{Workspace, Written};
use crate::workspace_page::{add_view, block_changed};
use crate::workspace_table::table_changed;
use crate::{
    BlockId, BlockKind, Changed, Created, Document, Error, Mutation, PageId, calculation, chart,
    database, drawing, tab,
};

const MAX_ICON: usize = 64;

impl<S: Store> Workspace<S> {
    pub(crate) fn write(&mut self, mutation: &Mutation) -> Result<Written, Error> {
        let page = match mutation {
            Mutation::AddTextBlock { page, .. }
            | Mutation::SplitTextBlock { page, .. }
            | Mutation::JoinTextBlock { page, .. }
            | Mutation::SetSectionBoundary { page, .. }
            | Mutation::TurnIntoCode { page, .. }
            | Mutation::TurnIntoText { page, .. }
            | Mutation::TurnIntoMath { page, .. }
            | Mutation::AddMediaBlock { page, .. }
            | Mutation::AddWhiteboardBlock { page, .. }
            | Mutation::AddCalculationBlock { page, .. }
            | Mutation::AddChartBlock { page, .. }
            | Mutation::AddDatabaseBlock { page, .. }
            | Mutation::AddTableView { page, .. }
            | Mutation::DuplicateTable { page, .. }
            | Mutation::RemoveView { page, .. }
            | Mutation::MoveViews { page, .. } => page,
            Mutation::DeleteTable { block } => return self.write_across_pages(mutation, block),
            _ => return self.write_mutation(mutation),
        };
        let order = self.page(page)?.1;
        if !order.arranged() {
            return self.write_mutation(mutation);
        }
        let before = order.sections();
        let mut written = self.write_mutation(mutation)?;
        if self.carry_tiles(page, &before)? {
            let document = Document::Page(page.clone());
            if !written.touched.contains(&document) {
                written.touched.push(document);
            }
            written.changed.push(Changed::Views(page.clone()));
        }
        Ok(written)
    }

    fn write_across_pages(
        &mut self,
        mutation: &Mutation,
        table: &BlockId,
    ) -> Result<Written, Error> {
        let mut pages: Vec<PageId> = self
            .table_views(table)?
            .into_iter()
            .map(|(page, _)| page)
            .collect();
        pages.sort();
        pages.dedup();
        let mut before = Vec::new();
        for page in pages {
            let order = match self.page(&page) {
                Ok((_, order)) => order,
                Err(Error::NotFound) => continue,
                Err(error) => return Err(error),
            };
            if order.arranged() {
                before.push((page, order.sections()));
            }
        }
        let mut written = self.write_mutation(mutation)?;
        for (page, sections) in &before {
            if self.carry_tiles(page, sections)? {
                let document = Document::Page(page.clone());
                if !written.touched.contains(&document) {
                    written.touched.push(document);
                }
                written.changed.push(Changed::Views(page.clone()));
            }
        }
        Ok(written)
    }

    fn write_mutation(&mut self, mutation: &Mutation) -> Result<Written, Error> {
        match mutation {
            Mutation::CreateArea { name } => self.write_create_area(name),
            Mutation::RenameWorkspace { name } => self.write_workspace(NAME, name),
            Mutation::SetWorkspaceIcon { icon } => {
                if icon.len() > MAX_ICON {
                    return Err(Error::TooLong);
                }
                self.write_workspace(ICON, icon)
            }
            Mutation::CreatePage {
                area,
                parent,
                title,
            } => self.write_create_page(area, parent.as_ref(), title),
            Mutation::RenamePage { page, title } => self.write_rename_page(page, title),
            Mutation::SetPageIcon { page, icon } => {
                if icon.len() > MAX_ICON {
                    return Err(Error::TooLong);
                }
                self.write_page_field(page, ICON, icon)
            }
            Mutation::AddSource {
                area,
                parent,
                name,
                kind,
            } => self.write_add_source(area, parent.as_ref(), name, *kind),
            Mutation::RenameSource { source, name } => self.write_source(source, Some(name)),
            Mutation::RemoveSource { source } => self.write_source(source, None),
            Mutation::AddTextBlock { page, after, text } => {
                self.write_add_text_block(page, after.as_ref(), text)
            }
            Mutation::SplitTextBlock {
                page,
                block,
                view,
                at,
                style,
            } => self.write_split_text_block(page, block, view, *at, *style),
            Mutation::JoinTextBlock { page, view } => self.write_join_text_block(page, view),
            Mutation::SetTextStyle { page, block, style } => {
                self.write_set_text_style(page, block, *style)
            }
            Mutation::SetSectionBoundary {
                page,
                view,
                boundary,
            } => self.write_set_section_boundary(page, view, *boundary),
            Mutation::EditText {
                page,
                block,
                at,
                delete,
                insert,
            } => self.write_edit_text(page, block, *at, *delete, insert),
            Mutation::TurnIntoCode { page, block } => {
                self.write_turn_into(page, block, BlockKind::Text, (BlockKind::Code, "code"))
            }
            Mutation::TurnIntoText { page, block } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let from = if model::math(doc, block.as_str()).is_some() {
                    BlockKind::Math
                } else {
                    BlockKind::Code
                };
                self.write_turn_into(page, block, from, (BlockKind::Text, "text"))
            }
            Mutation::TurnIntoMath { page, block } => {
                self.write_turn_into(page, block, BlockKind::Text, (BlockKind::Math, "math"))
            }
            Mutation::SetCodeLanguage {
                page,
                block,
                language,
            } => self.write_set_code_language(page, block, language),
            Mutation::AddMediaBlock {
                page,
                after,
                name,
                media_type,
                attachment,
            } => self.write_add_media_block(page, after.as_ref(), name, media_type, attachment),
            Mutation::AddWhiteboardBlock { page, after } => {
                self.write_add_whiteboard_block(page, after.as_ref())
            }
            Mutation::AddCalculationBlock { page, after } => {
                self.write_add_calculation_block(page, after.as_ref())
            }
            Mutation::AddChartBlock { page, after } => {
                let (doc, order) = self.page(page)?;
                let block = BlockId::new()?;
                chart::create(doc, &block)?;
                let sort = order.key_after(after.as_ref())?;
                let view = add_view(doc, order, sort, &block, BlockKind::Chart)?;
                let mut written =
                    Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
                written.created = Some(Created::Block { block, view });
                Ok(written)
            }
            Mutation::SetChart { page, block, setup } => {
                let areas = chart::set(self, page, block, setup)?;
                let mut written = block_changed(page, block, None);
                for area in areas {
                    written.touched.push(Document::Area(area.clone()));
                    written.changed.push(Changed::Pages(area));
                }
                Ok(written)
            }
            Mutation::AddLine {
                page,
                block,
                after,
                line,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let id = calculation::add_line(doc, block, after.as_ref(), line)?;
                Ok(block_changed(page, block, Some(Created::Line(id))))
            }
            Mutation::SetLine {
                page,
                block,
                line,
                setup,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                calculation::set_line(doc, block, line, setup)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::RemoveLine { page, block, line } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                calculation::remove_line(doc, block, line)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::SetDrawingStyle { page, block, style } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                drawing::set_style(doc, block, *style)?;
                Ok(block_changed(page, block, None))
            }
            Mutation::EditDrawing {
                page,
                block,
                add,
                change,
                remove,
            } => {
                let doc = self.doc(&Document::Page(page.clone()))?;
                let ids = drawing::edit(doc, block, add, change, remove)?;
                let created = (!ids.is_empty()).then_some(Created::Elements(ids));
                Ok(block_changed(page, block, created))
            }
            Mutation::AddDatabaseBlock {
                page,
                after,
                name,
                layout,
            } => self.write_add_database_block(page, after.as_ref(), name, *layout),
            Mutation::DuplicateTable {
                page,
                after,
                table,
                name,
                tabs,
            } => self.write_duplicate_table(page, after.as_ref(), table, name, tabs),
            Mutation::CreateTable { area, name } => {
                let (table, mut written) = self.write_create_table(area, name, None)?;
                written.created = Some(Created::Table(table));
                Ok(written)
            }
            Mutation::AddTableView {
                page,
                after,
                table,
                title,
                tabs,
            } => self.write_add_table_view(page, after.as_ref(), table, title, tabs),
            Mutation::RenameView { page, view, title } => self.write_rename_view(page, view, title),
            Mutation::DeleteTable { block } => self.write_delete_table(block),
            Mutation::RenameDatabase { block, name } => self.write_rename_database(block, name),
            Mutation::AddField {
                block,
                place,
                name,
                kind,
            } => {
                let doc = self.table_doc(block)?;
                let field = database::add_field(doc, block, place.as_ref(), name, *kind)?;
                Ok(table_changed(block, Some(Created::Field(field))))
            }
            Mutation::SetFieldFormula {
                block,
                field,
                formula,
            } => {
                database::set_formula(self.table_doc(block)?, block, field, formula)?;
                Ok(table_changed(block, None))
            }
            Mutation::RenameField { block, field, name } => {
                database::rename_field(self.table_doc(block)?, block, field, name)?;
                Ok(table_changed(block, None))
            }
            Mutation::RemoveField { block, field } => {
                let mut written = table_changed(block, None);
                self.field_removed(block, field, &mut written)?;
                database::remove_field(self.table_doc(block)?, block, field)?;
                Ok(written)
            }
            Mutation::AddOption {
                block,
                field,
                name,
                color,
            } => {
                let doc = self.table_doc(block)?;
                let option = database::add_option(doc, block, field, name, *color)?;
                Ok(table_changed(block, Some(Created::Option(option))))
            }
            Mutation::AddRow { block, place } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                let row = database::add_row(doc, index, block, place.as_ref())?;
                Ok(table_changed(block, Some(Created::Row(row))))
            }
            Mutation::MoveRow { block, row, place } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                database::move_row(doc, index, block, row, place.as_ref())?;
                Ok(table_changed(block, None))
            }
            Mutation::RemoveRow { block, row } => {
                self.table_doc(block)?;
                let (doc, index) = self.database_index(block)?;
                database::remove_row(doc, index, block, row)?;
                Ok(table_changed(block, None))
            }
            Mutation::SetCell {
                block,
                row,
                field,
                value,
            } => self.write_set_cell(block, row, field, value),
            Mutation::RemoveView { page, view } => self.write_remove_view(page, view),
            Mutation::SetTile { page, view, tile } => self.write_tile(page, view, *tile, false),
            Mutation::SetInnerTile { page, view, tile } => self.write_tile(page, view, *tile, true),
            Mutation::MoveViews {
                page,
                first,
                last,
                after,
            } => self.write_move_views(page, first, last, after.as_ref()),
            Mutation::AddTab {
                page,
                view,
                after,
                setup,
            } => self.write_add_tab(page, view, after.as_ref(), setup),
            Mutation::RenameTab {
                page,
                view,
                tab,
                name,
            } => self.write_tab(page, view, tab, |setup| setup.name.clone_from(name)),
            Mutation::SetTabLayout {
                page,
                view,
                tab,
                layout,
            } => self.write_tab(page, view, tab, |setup| setup.layout = *layout),
            Mutation::SetTabGroup {
                page,
                view,
                tab,
                group,
            } => self.write_tab(page, view, tab, |setup| setup.group.clone_from(group)),
            Mutation::SetTabDate {
                page,
                view,
                tab,
                date,
            } => self.write_tab(page, view, tab, |setup| setup.date.clone_from(date)),
            Mutation::SetTabSort {
                page,
                view,
                tab,
                sort,
            } => self.write_tab(page, view, tab, |setup| setup.sort.clone_from(sort)),
            Mutation::SetTabFilter {
                page,
                view,
                tab,
                filter,
            } => self.write_tab(page, view, tab, |setup| setup.filter.clone_from(filter)),
            Mutation::SetTabTotal {
                page,
                view,
                tab,
                field,
                total,
            } => self.write_tab(page, view, tab, |setup| {
                setup.totals.retain(|kept| kept.field != *field);
                if let Some(total) = total {
                    setup.totals.push(crate::FieldTotal {
                        field: field.clone(),
                        total: *total,
                    });
                }
            }),
            Mutation::SetTabFolded {
                page,
                view,
                tab,
                folded,
            } => self.write_tab(page, view, tab, |setup| setup.folded.clone_from(folded)),
            Mutation::MoveTab {
                page,
                view,
                tab,
                after,
            } => self.write_tabs(page, view, |doc, entry| {
                tab::move_after(doc, entry, tab, after.as_ref()).map(|()| None)
            }),
            Mutation::RemoveTab { page, view, tab } => self.write_tabs(page, view, |doc, entry| {
                tab::remove(doc, entry, tab).map(|()| None)
            }),
            Mutation::CreateAutomation { setup } => self.write_create_automation(setup),
            Mutation::SetAutomation { automation, setup } => {
                self.write_set_automation(automation, setup)
            }
            Mutation::SetAutomationOn { automation, on } => {
                self.write_automation_on(automation, *on)
            }
            Mutation::RemoveAutomation { automation } => self.write_remove_automation(automation),
            Mutation::SetAutomationRole { automation, role } => {
                self.write_automation_role(automation, role)
            }
            Mutation::CreateRole { setup } => self.write_create_role(setup),
            Mutation::SetRole { role, setup } => self.write_set_role(role, setup),
            Mutation::RemoveRole { role } => self.write_remove_role(role),
            Mutation::ProtectTable { table, roles } => self.write_protect(table, roles),
            Mutation::UnprotectTable { table } => self.write_unprotect(table),
        }
    }

    fn write_workspace(&mut self, key: &str, value: &str) -> Result<Written, Error> {
        self.doc(&Document::Workspace)?.put(ROOT, key, value)?;
        Ok(Written::new(Document::Workspace, Changed::Workspace))
    }
}
