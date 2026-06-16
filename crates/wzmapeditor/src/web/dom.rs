//! Browser DOM helpers shared by the web build's file flows.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

/// A one-shot file-pick callback, shared between the `change` and `cancel`
/// listeners so whichever fires first consumes it.
type PickSlot = Rc<RefCell<Option<Box<dyn FnOnce(Option<web_sys::File>)>>>>;

/// Open a single-file picker and deliver the chosen file, or `None` if the
/// picker is dismissed without a selection.
///
/// `on_result` runs exactly once. The `change` and `cancel` events route
/// through a single-shot slot so whichever fires first consumes the callback;
/// the firing listener frees itself. Delivering `None` on cancel lets callers
/// release any state they latched before opening the picker (the captured
/// channel sender drops, so a frame poller sees the channel disconnect) — a
/// dismissed picker must not wedge the UI.
pub(crate) fn pick_file(accept: &str, on_result: impl FnOnce(Option<web_sys::File>) + 'static) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        log::warn!("No browser document available to open a file picker.");
        on_result(None);
        return;
    };
    let input = document
        .create_element("input")
        .ok()
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok());
    let Some(input) = input else {
        log::warn!("Could not create a file input element.");
        on_result(None);
        return;
    };
    input.set_type("file");
    let _ = input.set_attribute("accept", accept);

    let slot: PickSlot = Rc::new(RefCell::new(Some(Box::new(on_result))));

    let input_for_change = input.clone();
    let slot_change = Rc::clone(&slot);
    let on_change = Closure::once_into_js(move || {
        let file = input_for_change.files().and_then(|list| list.get(0));
        if let Some(cb) = slot_change.borrow_mut().take() {
            cb(file);
        }
    });

    let slot_cancel = Rc::clone(&slot);
    let on_cancel = Closure::once_into_js(move || {
        if let Some(cb) = slot_cancel.borrow_mut().take() {
            cb(None);
        }
    });

    let _ = input.add_event_listener_with_callback("change", on_change.unchecked_ref());
    let _ = input.add_event_listener_with_callback("cancel", on_cancel.unchecked_ref());
    input.click();
}
