use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc};

use crate::model::{
    self, AREAS, BLOCK, BLOCKS, BOUNDARY, KIND, LANGUAGE, NAME, PAGES, PARENT, SORT, STYLE, TEXT,
    TITLE, VIEWS,
};
use crate::order::Order;
use crate::store::Store;
use crate::workspace::{Workspace, Written};
use crate::workspace_view::move_boundary;
use crate::{
    AreaId, AttachmentId, BlockId, BlockKind, Changed, Created, Document, Error, Media, PageId,
    TextStyle, View, ViewId, calculation, drawing,
};

fn add_text_block(
    doc: &mut AutoCommit,
    order: &mut Order,
    sort: String,
    text: &str,
    style: TextStyle,
) -> Result<(BlockId, ViewId), Error> {
    let style = model::style_name(style)?;
    let block = BlockId::new()?;
    let entry = model::insert(
        doc,
        BLOCKS,
        block.as_str(),
        &[(KIND, "text"), (STYLE, &style)],
    )?;
    let content = doc.put_object(&entry, TEXT, ObjType::Text)?;
    doc.splice_text(&content, 0, 0, text)?;
    let view = add_view(doc, order, sort, &block, BlockKind::Text)?;
    Ok((block, view))
}

pub(crate) fn add_view(
    doc: &mut AutoCommit,
    order: &mut Order,
    sort: String,
    block: &BlockId,
    kind: BlockKind,
) -> Result<ViewId, Error> {
    let view = ViewId::new()?;
    model::insert(
        doc,
        VIEWS,
        view.as_str(),
        &[(BLOCK, block.as_str()), (SORT, &sort)],
    )?;
    doc.put(
        model::live(doc, VIEWS, view.as_str())
            .as_ref()
            .ok_or(Error::NotFound)?,
        BOUNDARY,
        false,
    )?;
    let placed = View {
        id: view.clone(),
        block: block.clone(),
        title: String::new(),
        boundary: false,
        kind,
        tabs: Vec::new(),
        tile: None,
        inner: None,
    };
    order.insert(sort, placed);
    Ok(view)
}

pub(crate) fn text_block(doc: &AutoCommit, block: &BlockId) -> Option<automerge::ObjId> {
    model::of_kind(doc, block.as_str(), BlockKind::Text)
}

fn text_of(doc: &AutoCommit, block: &BlockId) -> Result<automerge::ObjId, Error> {
    let entry = text_block(doc, block).ok_or(Error::NotFound)?;
    model::child(doc, &entry, TEXT).ok_or(Error::NotFound)
}

fn utf8_offset(text: &str, at: usize) -> usize {
    let mut units = 0;
    for (offset, c) in text.char_indices() {
        if units >= at {
            return offset;
        }
        units += c.len_utf16();
    }
    text.len()
}

pub(crate) fn block_changed(page: &PageId, block: &BlockId, created: Option<Created>) -> Written {
    let mut written = Written::new(
        Document::Page(page.clone()),
        Changed::Block(page.clone(), block.clone()),
    );
    written.created = created;
    written
}

impl<S: Store> Workspace<S> {
    pub(crate) fn write_create_area(&mut self, name: &str) -> Result<Written, Error> {
        let id = AreaId::new()?;
        let mut doc = model::document(&self.device);
        model::create_maps(&mut doc, &[PAGES])?;
        let workspace = self.doc(&Document::Workspace)?;
        let siblings = model::sorted(workspace, AREAS);
        let last = siblings.last().map(|entry| entry.id.as_str());
        let sort = model::sort_after(&siblings, last)?;
        model::insert(
            workspace,
            AREAS,
            id.as_str(),
            &[(NAME, name), (SORT, &sort)],
        )?;
        let mut written = Written::new(Document::Workspace, Changed::Areas);
        written.touched.push(Document::Area(id.clone()));
        written.created = Some(Created::Area(id.clone()));
        written.new.push((Document::Area(id), doc));
        Ok(written)
    }

    pub(crate) fn write_create_page(
        &mut self,
        area: &AreaId,
        parent: Option<&PageId>,
        title: &str,
    ) -> Result<Written, Error> {
        let area_doc = self.doc(&Document::Area(area.clone()))?;
        if let Some(parent) = parent {
            model::live(area_doc, PAGES, parent.as_str()).ok_or(Error::NotFound)?;
        }
        let siblings: Vec<_> = model::sorted(area_doc, PAGES)
            .into_iter()
            .filter(|entry| entry.get(PARENT) == parent.map(PageId::as_str))
            .collect();
        let last = siblings.last().map(|entry| entry.id.as_str());
        let sort = model::sort_after(&siblings, last)?;
        let id = PageId::new()?;
        let mut fields = vec![(TITLE, title), (SORT, &sort)];
        if let Some(parent) = parent {
            fields.push((PARENT, parent.as_str()));
        }
        model::insert(area_doc, PAGES, id.as_str(), &fields)?;
        let mut doc = model::document(&self.device);
        model::create_maps(&mut doc, &[BLOCKS, VIEWS])?;
        let mut written = Written::new(Document::Area(area.clone()), Changed::Pages(area.clone()));
        written.touched.push(Document::Page(id.clone()));
        written.created = Some(Created::Page(id.clone()));
        written.new.push((Document::Page(id), doc));
        Ok(written)
    }

    pub(crate) fn write_rename_page(
        &mut self,
        page: &PageId,
        title: &str,
    ) -> Result<Written, Error> {
        self.write_page_field(page, TITLE, title)
    }

    pub(crate) fn write_page_field(
        &mut self,
        page: &PageId,
        field: &str,
        value: &str,
    ) -> Result<Written, Error> {
        let area = self.area_of(page)?;
        let area_doc = self.doc(&Document::Area(area.clone()))?;
        let entry = model::live(area_doc, PAGES, page.as_str()).ok_or(Error::NotFound)?;
        area_doc.put(&entry, field, value)?;
        Ok(Written::new(
            Document::Area(area.clone()),
            Changed::Pages(area),
        ))
    }

    pub(crate) fn write_add_text_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        text: &str,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let sort = order.key_after(after)?;
        let (block, view) = add_text_block(doc, order, sort, text, TextStyle::Paragraph)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    pub(crate) fn write_split_text_block(
        &mut self,
        page: &PageId,
        block: &BlockId,
        view: &ViewId,
        at: usize,
        style: TextStyle,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        if order.view(view).is_none_or(|shown| &shown.block != block) {
            return Err(Error::NotFound);
        }
        let content = text_of(doc, block)?;
        if !model::on_boundary(doc, &content, at) {
            return Err(Error::OutOfRange);
        }
        let (new_block, new_view) = if at == 0 {
            let starts_section = order.view(view).is_some_and(|shown| shown.boundary);
            let sort = order.key_before(view)?;
            let added = add_text_block(doc, order, sort, "", style)?;
            if starts_section {
                move_boundary(doc, order, view, &added.1)?;
            }
            added
        } else {
            let text = doc.text(&content)?;
            let tail = text[utf8_offset(&text, at)..].to_string();
            let length =
                isize::try_from(doc.length(&content) - at).map_err(|_| Error::OutOfRange)?;
            doc.splice_text(&content, at, length, "")?;
            let sort = order.key_after(Some(view))?;
            add_text_block(doc, order, sort, &tail, style)?
        };
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), block.clone()));
        written.created = Some(Created::Block {
            block: new_block,
            view: new_view,
        });
        Ok(written)
    }

    pub(crate) fn write_join_text_block(
        &mut self,
        page: &PageId,
        view: &ViewId,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let joining = order.view(view).ok_or(Error::NotFound)?.clone();
        if joining.boundary {
            return Err(Error::SectionBoundary);
        }
        let above = order.above(view).ok_or(Error::NotFound)?.clone();
        let text = model::text(doc, joining.block.as_str()).ok_or(Error::NotFound)?;
        let target = text_of(doc, &above.block)?;
        let end = doc.length(&target);
        doc.splice_text(&target, end, 0, &text)?;
        let entry = model::live(doc, VIEWS, view.as_str()).ok_or(Error::NotFound)?;
        model::mark_deleted(doc, &entry)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), above.block.clone()));
        if !order.shows(&joining.block, view) {
            if let Some(block_entry) = model::live(doc, BLOCKS, joining.block.as_str()) {
                model::mark_deleted(doc, &block_entry)?;
            }
            written
                .changed
                .push(Changed::Block(page.clone(), joining.block));
        }
        order.remove(view);
        Ok(written)
    }

    pub(crate) fn write_set_text_style(
        &mut self,
        page: &PageId,
        block: &BlockId,
        style: TextStyle,
    ) -> Result<Written, Error> {
        let name = model::style_name(style)?;
        let (doc, _) = self.page(page)?;
        let entry = text_block(doc, block).ok_or(Error::NotFound)?;
        doc.put(&entry, STYLE, name)?;
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    pub(crate) fn write_edit_text(
        &mut self,
        page: &PageId,
        block: &BlockId,
        at: usize,
        delete: usize,
        insert: &str,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Page(page.clone()))?;
        let entry = model::of_kind(doc, block.as_str(), BlockKind::Text)
            .or_else(|| model::of_kind(doc, block.as_str(), BlockKind::Code))
            .or_else(|| model::of_kind(doc, block.as_str(), BlockKind::Math))
            .ok_or(Error::NotFound)?;
        let content = model::child(doc, &entry, TEXT).ok_or(Error::NotFound)?;
        let end = at.checked_add(delete).ok_or(Error::OutOfRange)?;
        if !model::on_boundary(doc, &content, at) || !model::on_boundary(doc, &content, end) {
            return Err(Error::OutOfRange);
        }
        let delete = isize::try_from(delete).map_err(|_| Error::OutOfRange)?;
        doc.splice_text(&content, at, delete, insert)?;
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    pub(crate) fn write_turn_into(
        &mut self,
        page: &PageId,
        block: &BlockId,
        from: BlockKind,
        (to, name): (BlockKind, &str),
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let entry = model::of_kind(doc, block.as_str(), from).ok_or(Error::NotFound)?;
        doc.put(&entry, KIND, name)?;
        match to {
            BlockKind::Text => doc.put(&entry, STYLE, model::style_name(TextStyle::Paragraph)?)?,
            _ => doc.delete(&entry, LANGUAGE)?,
        }
        order.set_kind(block, to);
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written
            .changed
            .push(Changed::Block(page.clone(), block.clone()));
        Ok(written)
    }

    pub(crate) fn write_set_code_language(
        &mut self,
        page: &PageId,
        block: &BlockId,
        language: &str,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Page(page.clone()))?;
        let entry = model::of_kind(doc, block.as_str(), BlockKind::Code).ok_or(Error::NotFound)?;
        if language.is_empty() {
            doc.delete(&entry, LANGUAGE)?;
        } else {
            doc.put(&entry, LANGUAGE, language)?;
        }
        Ok(Written::new(
            Document::Page(page.clone()),
            Changed::Block(page.clone(), block.clone()),
        ))
    }

    pub(crate) fn write_add_media_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
        name: &str,
        media_type: &str,
        attachment: &AttachmentId,
    ) -> Result<Written, Error> {
        let size = self
            .store
            .attachment_size(attachment)?
            .ok_or(Error::NotFound)?;
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        let entry = model::insert(doc, BLOCKS, block.as_str(), &[(KIND, "media")])?;
        let file = Media {
            name: name.into(),
            media_type: media_type.into(),
            attachment: attachment.clone(),
            size,
        };
        model::put_file(doc, &entry, &file)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Media)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    pub(crate) fn write_add_whiteboard_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        drawing::create(doc, &block)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Whiteboard)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }

    pub(crate) fn write_add_calculation_block(
        &mut self,
        page: &PageId,
        after: Option<&ViewId>,
    ) -> Result<Written, Error> {
        let (doc, order) = self.page(page)?;
        let block = BlockId::new()?;
        calculation::create(doc, &block)?;
        let sort = order.key_after(after)?;
        let view = add_view(doc, order, sort, &block, BlockKind::Calculation)?;
        let mut written = Written::new(Document::Page(page.clone()), Changed::Views(page.clone()));
        written.created = Some(Created::Block { block, view });
        Ok(written)
    }
}
