use std::cell::Cell;
use std::io;
use std::rc::Rc;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSApplication,
    NSControlStateValueOn, NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConfirmationResponse {
    #[default]
    Cancelled,
    Confirmed {
        dont_ask_again: bool,
    },
}

#[derive(Debug)]
pub struct Confirmation {
    alert: Retained<NSAlert>,
    parent: Retained<NSWindow>,
    completed: Rc<Cell<bool>>,
}

impl Drop for Confirmation {
    fn drop(&mut self) {
        if !self.completed.replace(true) {
            self.parent
                .endSheet_returnCode(&self.alert.window(), NSAlertSecondButtonReturn);
        }
    }
}

pub fn confirm(
    title: &str,
    message: &str,
    confirm_label: &str,
    suppression_label: Option<&str>,
    on_complete: impl FnOnce(ConfirmationResponse) + 'static,
) -> io::Result<Confirmation> {
    let main_thread = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("native dialogs require the main thread"))?;
    let app = NSApplication::sharedApplication(main_thread);
    let parent = app
        .keyWindow()
        .or_else(|| app.mainWindow())
        .ok_or_else(|| io::Error::other("native dialog has no application window"))?;
    if parent.attachedSheet().is_some() {
        return Err(io::Error::other("an application dialog is already open"));
    }
    let alert = NSAlert::new(main_thread);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert
        .addButtonWithTitle(&NSString::from_str(confirm_label))
        .setKeyEquivalent(&NSString::from_str("\r"));
    alert
        .addButtonWithTitle(&NSString::from_str("Cancel"))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    alert.setShowsSuppressionButton(suppression_label.is_some());
    let suppression = alert.suppressionButton();
    if let (Some(button), Some(label)) = (&suppression, suppression_label) {
        button.setTitle(&NSString::from_str(label));
    }
    let completed = Rc::new(Cell::new(false));
    let finished = Rc::clone(&completed);
    let callback = Cell::new(Some(on_complete));
    let handler = RcBlock::new(move |response| {
        finished.set(true);
        let dont_ask_again = suppression
            .as_ref()
            .is_some_and(|button| button.state() == NSControlStateValueOn);
        if let Some(callback) = callback.take() {
            callback(classify(response, dont_ask_again));
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&parent, Some(&handler));
    Ok(Confirmation {
        alert,
        parent,
        completed,
    })
}

fn classify(response: NSModalResponse, dont_ask_again: bool) -> ConfirmationResponse {
    if response == NSAlertFirstButtonReturn {
        ConfirmationResponse::Confirmed { dont_ask_again }
    } else {
        ConfirmationResponse::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_confirmation_button_can_suppress_future_prompts() {
        for dont_ask_again in [false, true] {
            assert_eq!(
                classify(NSAlertFirstButtonReturn, dont_ask_again),
                ConfirmationResponse::Confirmed { dont_ask_again }
            );
            for response in [NSAlertSecondButtonReturn, -1000, 0, 1002] {
                assert_eq!(
                    classify(response, dont_ask_again),
                    ConfirmationResponse::Cancelled
                );
            }
        }
    }
}

#[derive(Debug)]
pub struct FolderPicker {
    panel: Retained<objc2_app_kit::NSOpenPanel>,
    completed: Rc<Cell<bool>>,
}

impl Drop for FolderPicker {
    #[allow(
        unsafe_code,
        reason = "AppKit target/action cancellation accepts a nil sender."
    )]
    fn drop(&mut self) {
        if !self.completed.replace(true) {
            // SAFETY: No sender is passed; the retained panel is confined to the main thread.
            unsafe {
                self.panel.cancel(None);
            }
        }
    }
}

pub fn choose_folder(
    message: &str,
    directory: &std::path::Path,
    on_complete: impl FnOnce(Option<std::path::PathBuf>) + 'static,
) -> io::Result<FolderPicker> {
    let main_thread = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("native dialogs require the main thread"))?;
    let panel = objc2_app_kit::NSOpenPanel::openPanel(main_thread);
    panel.setMessage(Some(&NSString::from_str(message)));
    panel.setCanChooseFiles(false);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setDirectoryURL(Some(&objc2_foundation::NSURL::fileURLWithPath(
        &NSString::from_str(&directory.to_string_lossy()),
    )));
    let completed = Rc::new(Cell::new(false));
    let finished = Rc::clone(&completed);
    let result_panel = panel.clone();
    let callback = Cell::new(Some(on_complete));
    let handler = RcBlock::new(move |response| {
        finished.set(true);
        let path = (response == objc2_app_kit::NSModalResponseOK)
            .then(|| {
                result_panel
                    .URL()
                    .and_then(|url| url.path())
                    .map(|path| path.to_string().into())
            })
            .flatten();
        if let Some(callback) = callback.take() {
            callback(path);
        }
    });
    panel.beginWithCompletionHandler(&handler);
    Ok(FolderPicker { panel, completed })
}
