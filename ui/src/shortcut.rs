//! Application-level keyboard shortcut handling.

use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer, widget};
use iced::keyboard::{self, Key, Modifiers, key};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

use crate::app;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    GlobalSearch,
    Refresh,
    NavigateFinding,
    NavigateUpdates,
    NavigateInstalled,
    NavigateHealth,
    NavigateSettings,
    FocusPageSearch,
    Dismiss,
    PrimaryAction,
    SelectAll,
    MoveSelection(SelectionDirection),
    ToggleSelection,
    FocusNext,
    FocusPrevious,
}

/// Wraps the application view and captures shortcuts before text widgets can
/// interpret them as input. Contextual keys are emitted only when children do
/// not capture the event.
pub fn capture(content: Element<'_, app::Message>) -> Element<'_, app::Message> {
    Element::new(ShortcutArea { content })
}

struct ShortcutArea<'a> {
    content: Element<'a, app::Message>,
}

impl Widget<app::Message, iced::Theme, iced::Renderer> for ShortcutArea<'_> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<widget::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, app::Message>,
        viewport: &Rectangle,
    ) {
        if let Some(shortcut) = shortcut_before_children(event) {
            shell.publish(app::Message::Shortcut(shortcut));
            shell.capture_event();
            return;
        }

        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );

        if let Some(shortcut) = shortcut_after_children(event, shell.is_event_captured()) {
            shell.publish(app::Message::Shortcut(shortcut));
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, app::Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

fn shortcut_before_children(event: &Event) -> Option<Shortcut> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        physical_key,
        modifiers,
        repeat,
        ..
    }) = event
    else {
        return None;
    };

    if *repeat {
        return None;
    }

    if *modifiers == Modifiers::CTRL {
        return match physical_key {
            key::Physical::Code(key::Code::KeyK) => Some(Shortcut::GlobalSearch),
            key::Physical::Code(key::Code::KeyR) => Some(Shortcut::Refresh),
            _ if matches!(key, Key::Named(key::Named::Enter)) => Some(Shortcut::PrimaryAction),
            _ => None,
        };
    }

    if *modifiers == Modifiers::ALT {
        return match physical_key {
            key::Physical::Code(key::Code::Digit1) => Some(Shortcut::NavigateFinding),
            key::Physical::Code(key::Code::Digit2) => Some(Shortcut::NavigateUpdates),
            key::Physical::Code(key::Code::Digit3) => Some(Shortcut::NavigateInstalled),
            key::Physical::Code(key::Code::Digit4) => Some(Shortcut::NavigateHealth),
            key::Physical::Code(key::Code::Digit5) => Some(Shortcut::NavigateSettings),
            _ => None,
        };
    }

    None
}

fn shortcut_after_children(event: &Event, captured: bool) -> Option<Shortcut> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        physical_key,
        modifiers,
        repeat,
        ..
    }) = event
    else {
        return None;
    };

    if !*repeat && *modifiers == Modifiers::NONE && matches!(key, Key::Named(key::Named::Escape)) {
        return Some(Shortcut::Dismiss);
    }

    if captured {
        return None;
    }

    if *modifiers == Modifiers::CTRL && matches!(physical_key, key::Physical::Code(key::Code::KeyA))
    {
        return (!*repeat).then_some(Shortcut::SelectAll);
    }

    if modifiers.alt() || modifiers.control() || modifiers.logo() {
        return None;
    }

    match key.as_ref() {
        Key::Character("/") if !*repeat => Some(Shortcut::FocusPageSearch),
        Key::Character(" ") => Some(Shortcut::ToggleSelection),
        Key::Named(key::Named::ArrowUp) => {
            Some(Shortcut::MoveSelection(SelectionDirection::Previous))
        }
        Key::Named(key::Named::ArrowDown) => {
            Some(Shortcut::MoveSelection(SelectionDirection::Next))
        }
        Key::Named(key::Named::Tab) if modifiers.shift() => Some(Shortcut::FocusPrevious),
        Key::Named(key::Named::Tab) => Some(Shortcut::FocusNext),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Location, key::NativeCode};

    fn key_event(key: Key, physical_key: key::Physical, modifiers: Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key,
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn global_shortcuts_are_captured_before_text_input() {
        let ctrl_k = key_event(
            Key::Character("k".into()),
            key::Physical::Code(key::Code::KeyK),
            Modifiers::CTRL,
        );
        let alt_3 = key_event(
            Key::Character("3".into()),
            key::Physical::Code(key::Code::Digit3),
            Modifiers::ALT,
        );

        assert_eq!(
            shortcut_before_children(&ctrl_k),
            Some(Shortcut::GlobalSearch)
        );
        assert_eq!(
            shortcut_before_children(&alt_3),
            Some(Shortcut::NavigateInstalled)
        );
    }

    #[test]
    fn contextual_shortcuts_do_not_override_text_editing() {
        let slash = key_event(
            Key::Character("/".into()),
            key::Physical::Code(key::Code::Slash),
            Modifiers::NONE,
        );
        let select_all = key_event(
            Key::Character("a".into()),
            key::Physical::Code(key::Code::KeyA),
            Modifiers::CTRL,
        );

        assert_eq!(
            shortcut_after_children(&slash, false),
            Some(Shortcut::FocusPageSearch)
        );
        assert_eq!(shortcut_after_children(&slash, true), None);
        assert_eq!(shortcut_after_children(&select_all, true), None);
        assert_eq!(
            shortcut_after_children(&select_all, false),
            Some(Shortcut::SelectAll)
        );
    }

    #[test]
    fn escape_is_available_after_a_text_input_consumes_it() {
        let escape = key_event(
            Key::Named(key::Named::Escape),
            key::Physical::Unidentified(NativeCode::Unidentified),
            Modifiers::NONE,
        );

        assert_eq!(
            shortcut_after_children(&escape, true),
            Some(Shortcut::Dismiss)
        );
    }
}
