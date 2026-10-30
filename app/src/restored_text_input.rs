//! Native IME composition is local; only confirmed text crosses an exact source binding.
use super::*;
use crate::product_components::TextEditState;
use std::ops::Range;

pub(super) struct RemoteTextInput {
    pub binding: remote_core::workspace_session::TextInputOwner,
    view: SourceViewBinding,
    owner: Arc<OriginalOwner>,
    focus: FocusHandle,
    pub anchor: Option<Point<Pixels>>,
    edit: TextEditState,
}
impl RemoteTextInput {
    pub fn new(
        owner: Arc<OriginalOwner>,
        view: SourceViewBinding,
        binding: remote_core::workspace_session::TextInputOwner,
        focus: FocusHandle,
        anchor: Option<Point<Pixels>>,
    ) -> Self {
        Self {
            owner,
            view,
            binding,
            focus,
            anchor,
            edit: TextEditState::new(),
        }
    }
    pub fn composing(&self) -> bool {
        self.edit.marked_range().is_some()
    }
}
impl EntityInputHandler for RemoteTextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (text, adjusted) = self.edit.text_for_range_utf16(range);
        *actual = Some(adjusted);
        Some(text)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.edit.selected_text_range())
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.edit.marked_text_range_utf16()
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.edit = TextEditState::new();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus.is_focused(window) && !text.is_empty() {
            let _ = self
                .owner
                .commit_viewing_text(&self.view, self.binding, text);
        }
        self.edit = TextEditState::new();
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus.is_focused(window) && text.len() <= 64 * 1024 {
            self.edit
                .replace_and_mark_text_in_range(range, text, selected);
        } else {
            self.edit = TextEditState::new();
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // No remote caret readback: position candidates inside the current viewport.
        let anchor = self
            .anchor
            .unwrap_or(point(bounds.left() + px(12.), bounds.top() + px(36.)));
        let position = point(
            anchor.x.max(bounds.left()).min(bounds.right() - px(1.)),
            anchor
                .y
                .max(bounds.top())
                .min((bounds.bottom() - px(20.)).max(bounds.top())),
        );
        Some(Bounds::new(position, size(px(1.), px(20.))))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.edit.offset_to_utf16(self.edit.cursor_offset()))
    }
}
