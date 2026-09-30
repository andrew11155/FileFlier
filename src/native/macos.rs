//! macOS: files on the general pasteboard (so Finder and other apps can paste
//! them) and dragging files out of the window, through AppKit.

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEventType, NSPasteboard, NSPasteboardWriting, NSView, NSWorkspace,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};

define_class!(
    /// Tells AppKit what a drag from File Flier may do: copy (the receiving app decides).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FileFlierDragSource"]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: plain NSObject subclass without ivars; `init` is its designated initializer.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

fn file_url(p: &std::path::Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
}

pub struct Handle {
    view: Retained<NSView>,
    /// Kept alive for the drag in progress.
    source: Option<Retained<DragSource>>,
}

impl Handle {
    /// # Safety
    /// `ns_view` must be the window's live NSView.
    pub unsafe fn new(ns_view: *mut std::ffi::c_void) -> Option<Self> {
        // SAFETY: the caller guarantees a valid NSView; retaining keeps it alive with us.
        let view = unsafe { Retained::retain(ns_view.cast::<NSView>()) }?;
        Some(Handle { view, source: None })
    }

    pub fn set_clipboard(&self, paths: &[PathBuf]) -> bool {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            paths.iter().map(|p| ProtocolObject::from_retained(file_url(p))).collect();
        pb.writeObjects(&NSArray::from_retained_slice(&objs))
    }

    /// Hands a drag that left the window to AppKit. Needs the mouse-drag event being handled.
    pub fn start_drag(&mut self, paths: &[PathBuf]) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else { return false };
        // AppKit only starts a drag session from a mouse-drag event.
        if event.r#type() != NSEventType::LeftMouseDragged {
            return false;
        }
        let at = self.view.convertPoint_fromView(event.locationInWindow(), None);
        let workspace = NSWorkspace::sharedWorkspace();
        let items: Vec<Retained<NSDraggingItem>> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let url = file_url(p);
                let item =
                    NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                let icon = workspace.iconForFile(&NSString::from_str(&p.to_string_lossy()));
                let off = (i.min(4) * 6) as f64;
                let frame = NSRect::new(NSPoint::new(at.x - 24.0 + off, at.y - 24.0 - off), NSSize::new(48.0, 48.0));
                let contents: &AnyObject = &icon;
                // SAFETY: an NSImage is valid dragging-frame contents.
                unsafe { item.setDraggingFrame_contents(frame, Some(contents)) };
                item
            })
            .collect();
        if items.is_empty() {
            return false;
        }
        let source = DragSource::new(mtm);
        let _session = self.view.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&items),
            &event,
            ProtocolObject::from_ref(&*source),
        );
        self.source = Some(source);
        true
    }
}
