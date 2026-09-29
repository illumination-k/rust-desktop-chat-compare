//! Shows MCP App views inside native (non-web) UIs as child webviews.
//!
//! A child webview is an OS-level window laid over the app's window, so the UI
//! reserves a placeholder where the view belongs and moves the webview there on
//! every frame with [`AppWebView::set_bounds`]. The webview cannot be clipped by
//! the UI's scroll area; instead the bounds are cut to the visible part and the
//! document is scrolled by the hidden amount, so the content stays in place.
//!
//! The view talks to `window.parent`, which in a top-level webview is the view
//! itself. An init script reroutes `postMessage` to wry's IPC channel and lets
//! the host deliver messages as `message` events whose source is `window`.

use std::sync::mpsc;

use chat_core::mcp_app::{AppHost, HostEvent};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

const BRIDGE_JS: &str = r#"(() => {
  const ipc = window.ipc;
  window.postMessage = (message) => ipc.postMessage(JSON.stringify(message));
  window.__mcpDeliver = (data) =>
    window.dispatchEvent(new MessageEvent("message", { data, source: window }));
  // The host scrolls the document when the view is partly scrolled out of the chat.
  document.addEventListener("DOMContentLoaded", () => {
    const style = document.createElement("style");
    style.textContent = "html { scrollbar-width: none } ::-webkit-scrollbar { display: none }";
    document.head.append(style);
  });
})();"#;

/// Placement of the view in logical window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    /// Cuts the placeholder `self` to the visible `viewport`. Returns the visible
    /// part and how much of the placeholder's top is hidden, or `None` if nothing shows.
    pub fn clip(self, viewport: Self) -> Option<(Self, f64)> {
        let top = self.y.max(viewport.y);
        let bottom = (self.y + self.height).min(viewport.y + viewport.height);
        let left = self.x.max(viewport.x);
        let right = (self.x + self.width).min(viewport.x + viewport.width);
        (bottom - top >= 1.0 && right - left >= 1.0).then(|| {
            let visible = Self {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            };
            (visible, top - self.y)
        })
    }
}

/// A copy of the app window's handle, for UIs that only lend the window inside
/// callbacks (iced). Only use it on the UI thread while the window is alive.
#[derive(Clone, Copy, Debug)]
pub struct ParentWindow(RawWindowHandle);

// SAFETY: the handle is only an identifier here; it is dereferenced (by wry) on
// the UI thread that owns the window, as documented on the type.
unsafe impl Send for ParentWindow {}

impl ParentWindow {
    pub fn of<W: HasWindowHandle + ?Sized>(window: &W) -> Option<Self> {
        window.window_handle().ok().map(|h| Self(h.as_raw()))
    }
}

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: see the type's documentation; the window outlives its views.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// One MCP App view in a child webview, with its protocol host.
pub struct AppWebView {
    webview: WebView,
    host: AppHost,
    inbox: mpsc::Receiver<String>,
    placement: Option<(Bounds, f64)>,
}

impl AppWebView {
    /// Creates a hidden webview on `window`. `wake` runs on the UI thread whenever
    /// the view sent a message; the UI should call [`pump`](Self::pump) soon after.
    pub fn new(
        window: &impl HasWindowHandle,
        host: AppHost,
        wake: impl Fn() + 'static,
    ) -> wry::Result<Self> {
        let (tx, inbox) = mpsc::channel();
        let loaded = std::cell::Cell::new(false);
        let webview = WebViewBuilder::new()
            .with_html(host.document())
            .with_initialization_script(BRIDGE_JS)
            .with_ipc_handler(move |request| {
                let _ = tx.send(request.into_body());
                wake();
            })
            // Only the initial document; links go through `ui/open-link`.
            .with_navigation_handler(move |_| !loaded.replace(true))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_visible(false)
            .with_bounds(Rect {
                position: LogicalPosition::new(0, 0).into(),
                size: LogicalSize::new(1, 1).into(),
            })
            .build_as_child(window)?;
        Ok(Self {
            webview,
            host,
            inbox,
            placement: None,
        })
    }

    /// Handles everything the view sent since the last call.
    pub fn pump(&mut self) -> Vec<HostEvent> {
        let mut events = Vec::new();
        while let Ok(raw) = self.inbox.try_recv() {
            tracing::debug!(%raw, "view → host");
            let reply = self.host.handle_json(&raw);
            for message in &reply.messages {
                self.deliver(message);
            }
            events.extend(reply.events);
        }
        events
    }

    /// Places the webview over the visible part of `placeholder` (see [`Bounds::clip`]).
    pub fn place(&mut self, placeholder: Bounds, viewport: Bounds) {
        let placement = placeholder.clip(viewport);
        if placement == self.placement {
            return;
        }
        tracing::debug!(
            ?placeholder,
            ?viewport,
            ?placement,
            "placing MCP App webview"
        );
        let result = match placement {
            Some((bounds, hidden_top)) => self
                .webview
                .set_bounds(Rect {
                    position: LogicalPosition::new(bounds.x, bounds.y).into(),
                    size: LogicalSize::new(bounds.width, bounds.height).into(),
                })
                .and_then(|()| {
                    self.webview
                        .evaluate_script(&format!("window.scrollTo(0, {hidden_top})"))
                })
                .and_then(|()| self.webview.set_visible(true)),
            None => self.webview.set_visible(false),
        };
        if let Err(e) = result {
            tracing::warn!("failed to place MCP App webview: {e}");
        }
        self.placement = placement;
    }

    /// Hides the webview (e.g. while a dialog covers the chat).
    pub fn hide(&mut self) {
        if self.placement.take().is_some() {
            let _ = self.webview.set_visible(false);
        }
    }

    fn deliver(&self, message: &serde_json::Value) {
        if let Err(e) = self
            .webview
            .evaluate_script(&format!("window.__mcpDeliver({message})"))
        {
            tracing::warn!("failed to deliver message to MCP App view: {e}");
        }
    }
}

impl Drop for AppWebView {
    fn drop(&mut self) {
        // Best effort: the webview may be gone before the view can answer.
        if let Some(teardown) = self.host.teardown() {
            self.deliver(&teardown);
        }
    }
}

/// Prepares the platform for child webviews. Call once on the UI thread before
/// creating any [`AppWebView`]. Returns `false` where they are unsupported (Wayland).
pub fn init() -> bool {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::DisplayExtManual;

        if gtk::init().is_err() {
            return false;
        }
        if gtk::gdk::Display::default().is_none_or(|d| d.backend().is_wayland()) {
            return false;
        }
        // Reparenting the webview triggers BadMatch (170), which winit would treat as fatal.
        winit::platform::x11::register_xlib_error_hook(Box::new(|_display, error| {
            let error = error.cast::<x11_dl::xlib::XErrorEvent>();
            // SAFETY: winit passes a valid `XErrorEvent` pointer to the hook.
            unsafe { (*error).error_code == 170 }
        }));
    }
    true
}

/// Runs pending webview events. Call on every UI frame / event loop iteration.
pub fn pump_platform() {
    #[cfg(target_os = "linux")]
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Bounds = Bounds {
        x: 0.0,
        y: 100.0,
        width: 500.0,
        height: 400.0,
    };

    fn at(y: f64, height: f64) -> Bounds {
        Bounds {
            x: 10.0,
            y,
            width: 300.0,
            height,
        }
    }

    #[test]
    fn fully_visible_is_unchanged() {
        assert_eq!(
            at(150.0, 200.0).clip(VIEWPORT),
            Some((at(150.0, 200.0), 0.0))
        );
    }

    #[test]
    fn scrolled_past_the_top_hides_the_top_part() {
        assert_eq!(
            at(50.0, 200.0).clip(VIEWPORT),
            Some((at(100.0, 150.0), 50.0))
        );
    }

    #[test]
    fn below_the_viewport_is_cut_at_the_bottom() {
        assert_eq!(
            at(400.0, 200.0).clip(VIEWPORT),
            Some((at(400.0, 100.0), 0.0))
        );
    }

    #[test]
    fn outside_the_viewport_is_hidden() {
        assert_eq!(at(0.0, 100.0).clip(VIEWPORT), None);
        assert_eq!(at(500.0, 100.0).clip(VIEWPORT), None);
    }
}
