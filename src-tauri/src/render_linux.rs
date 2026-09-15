//! Linux video pipeline.
//!
//! Counterpart of the Windows DirectComposition compositor in `render.rs`.
//! mpv renders through the libmpv OpenGL render API into a `GtkGLArea`, and
//! the (transparent) WebKitGTK webview created by Tauri is re-parented on top
//! of it inside a `GtkOverlay`. The video therefore always fills the window
//! and the HTML UI is composited over it, exactly like on Windows:
//! `mpv_set_geometry` / `mpv_set_visible` stay no-ops and the page reveals the
//! video by making `body.is-player-open` transparent.
//!
//! Everything that touches GTK runs on the GTK main thread; the mpv update
//! callback (called from mpv threads) only marshals a `queue_render`.
//!
//! The render context talks to libmpv through `libmpv2_sys` directly instead
//! of `libmpv2::render::RenderContext`: that wrapper instantiates its
//! `get_proc_address` trampoline with the wrong type parameter
//! (`gpa_wrapper::<OpenGLInitParams<C>>` instead of `::<C>`), which only works
//! when the context type happens to share the layout of the outer struct — true
//! for the pointer-sized context used on Windows, not for the loader used here.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use gtk::glib;
use gtk::prelude::*;
use libmpv2_sys as sys;
use tauri::{Manager, WebviewWindow};

use crate::mpv::Mpv;
use crate::state::AppState;

const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

type GlGetIntegervFn = unsafe extern "C" fn(pname: u32, data: *mut i32);
type GetProcAddressFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;
type GdkDisplayGetterFn =
    unsafe extern "C" fn(display: *mut gtk::gdk::ffi::GdkDisplay) -> *mut c_void;

/// Resolves GL entry points for libmpv.
///
/// GTK owns the GL context (EGL on Wayland, GLX or EGL on X11). With libglvnd
/// both `eglGetProcAddress` and `glXGetProcAddressARB` hand out dispatch stubs
/// that work for whichever context is current, so the loader matching the
/// display is tried first, then the other one, then a plain symbol lookup for
/// core functions some loaders refuse to resolve.
pub struct GlLoader {
    egl: Option<libloading::Library>,
    gl: Option<libloading::Library>,
    prefer_egl: bool,
}

impl GlLoader {
    fn new(prefer_egl: bool) -> Self {
        let egl = unsafe { libloading::Library::new("libEGL.so.1") }.ok();
        let gl = unsafe { libloading::Library::new("libGL.so.1") }.ok();
        if egl.is_none() && gl.is_none() {
            tracing::warn!("[render] neither libEGL.so.1 nor libGL.so.1 could be loaded");
        }
        Self { egl, gl, prefer_egl }
    }

    fn via_loader(lib: Option<&libloading::Library>, loader: &[u8], name: &CStr) -> *mut c_void {
        let Some(lib) = lib else {
            return std::ptr::null_mut();
        };
        match unsafe { lib.get::<GetProcAddressFn>(loader) } {
            Ok(f) => unsafe { f(name.as_ptr()) },
            Err(_) => std::ptr::null_mut(),
        }
    }

    fn direct(lib: Option<&libloading::Library>, name: &CStr) -> *mut c_void {
        let Some(lib) = lib else {
            return std::ptr::null_mut();
        };
        match unsafe { lib.get::<unsafe extern "C" fn()>(name.to_bytes_with_nul()) } {
            Ok(sym) => {
                let f: unsafe extern "C" fn() = *sym;
                f as usize as *mut c_void
            }
            Err(_) => std::ptr::null_mut(),
        }
    }

    fn get(&self, name: &CStr) -> *mut c_void {
        let egl = (self.egl.as_ref(), &b"eglGetProcAddress\0"[..]);
        let glx = (self.gl.as_ref(), &b"glXGetProcAddressARB\0"[..]);
        let order = if self.prefer_egl { [egl, glx] } else { [glx, egl] };
        for (lib, loader) in order {
            let p = Self::via_loader(lib, loader, name);
            if !p.is_null() {
                return p;
            }
        }
        for lib in [self.gl.as_ref(), self.egl.as_ref()] {
            let p = Self::direct(lib, name);
            if !p.is_null() {
                return p;
            }
        }
        std::ptr::null_mut()
    }
}

/// `mpv_opengl_init_params.get_proc_address` trampoline; `ctx` is the
/// `GlLoader` owned by the `GlRenderContext`.
unsafe extern "C" fn gl_get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let loader = &*(ctx as *const GlLoader);
    loader.get(CStr::from_ptr(name))
}

/// `mpv_render_context_set_update_callback` trampoline; `ctx` is the boxed
/// closure owned by the `GlRenderContext`.
unsafe extern "C" fn on_render_update(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let cb = &*(ctx as *const Box<dyn Fn() + Send + Sync>);
    cb();
}

fn mpv_error(code: c_int) -> String {
    unsafe {
        let s = sys::mpv_error_string(code);
        if s.is_null() {
            format!("mpv error {code}")
        } else {
            CStr::from_ptr(s).to_string_lossy().into_owned()
        }
    }
}

/// Thin owner of an `mpv_render_context` (OpenGL backend). Must be created,
/// used and dropped with the GL context current, on the GTK main thread.
struct GlRenderContext {
    raw: *mut sys::mpv_render_context,
    // Kept alive for the whole context lifetime: mpv resolves GL functions
    // during creation, but keeping the loader around costs nothing.
    _loader: Box<GlLoader>,
    update_cb: *mut Box<dyn Fn() + Send + Sync>,
}

impl GlRenderContext {
    fn new(
        mpv: *mut sys::mpv_handle,
        loader: Box<GlLoader>,
        kind: DisplayKind,
        native: *mut c_void,
    ) -> Result<Self> {
        let mut init = sys::mpv_opengl_init_params {
            get_proc_address: Some(gl_get_proc_address),
            get_proc_address_ctx: &*loader as *const GlLoader as *mut c_void,
        };
        let mut params = vec![
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: sys::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void,
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut sys::mpv_opengl_init_params as *mut c_void,
            },
        ];
        if !native.is_null() {
            let type_ = match kind {
                DisplayKind::X11 => Some(sys::mpv_render_param_type_MPV_RENDER_PARAM_X11_DISPLAY),
                DisplayKind::Wayland => Some(sys::mpv_render_param_type_MPV_RENDER_PARAM_WL_DISPLAY),
                DisplayKind::Other => None,
            };
            if let Some(type_) = type_ {
                params.push(sys::mpv_render_param { type_, data: native });
            }
        }
        params.push(sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: std::ptr::null_mut(),
        });

        let mut raw: *mut sys::mpv_render_context = std::ptr::null_mut();
        let err = unsafe { sys::mpv_render_context_create(&mut raw, mpv, params.as_mut_ptr()) };
        if err < 0 || raw.is_null() {
            return Err(anyhow!("mpv_render_context_create: {}", mpv_error(err)));
        }
        Ok(Self {
            raw,
            _loader: loader,
            update_cb: std::ptr::null_mut(),
        })
    }

    fn set_update_callback(&mut self, cb: impl Fn() + Send + Sync + 'static) {
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(cb));
        let ptr = Box::into_raw(boxed);
        unsafe {
            sys::mpv_render_context_set_update_callback(
                self.raw,
                Some(on_render_update),
                ptr as *mut c_void,
            );
        }
        self.update_cb = ptr;
    }

    /// Drains pending update flags; returns true when a new frame is ready.
    fn update(&self) -> bool {
        let flags = unsafe { sys::mpv_render_context_update(self.raw) };
        flags & u64::from(sys::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME) != 0
    }

    fn render(&self, fbo: i32, width: i32, height: i32, flip_y: bool) -> Result<()> {
        let mut fbo_param = sys::mpv_opengl_fbo {
            fbo,
            w: width,
            h: height,
            internal_format: 0,
        };
        let mut flip: c_int = c_int::from(flip_y);
        let mut params = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo_param as *mut sys::mpv_opengl_fbo as *mut c_void,
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: &mut flip as *mut c_int as *mut c_void,
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let err = unsafe { sys::mpv_render_context_render(self.raw, params.as_mut_ptr()) };
        if err < 0 {
            return Err(anyhow!("mpv_render_context_render: {}", mpv_error(err)));
        }
        Ok(())
    }
}

impl Drop for GlRenderContext {
    fn drop(&mut self) {
        unsafe {
            // Detach the callback first so mpv can no longer call into the
            // closure, then free the context (must happen before the mpv
            // handle is destroyed and with the GL context current).
            sys::mpv_render_context_set_update_callback(self.raw, None, std::ptr::null_mut());
            sys::mpv_render_context_free(self.raw);
            if !self.update_cb.is_null() {
                drop(Box::from_raw(self.update_cb));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DisplayKind {
    X11,
    Wayland,
    Other,
}

/// Native display handle mpv can use for hwdec interop (VA-API / VDPAU).
/// Resolved at runtime from the already-loaded libgdk-3, so the binary has no
/// hard link-time dependency on the GDK X11 / Wayland backends.
fn native_display() -> (DisplayKind, *mut c_void) {
    let Some(display) = gtk::gdk::Display::default() else {
        return (DisplayKind::Other, std::ptr::null_mut());
    };
    let type_name = display.type_().name();
    let (kind, getter): (DisplayKind, &[u8]) = match type_name {
        "GdkX11Display" => (DisplayKind::X11, b"gdk_x11_display_get_xdisplay\0"),
        "GdkWaylandDisplay" => (DisplayKind::Wayland, b"gdk_wayland_display_get_wl_display\0"),
        other => {
            tracing::info!("[render] unknown GDK display type {other}; no native handle for hwdec");
            return (DisplayKind::Other, std::ptr::null_mut());
        }
    };
    let this = libloading::os::unix::Library::this();
    let raw = match unsafe { this.get::<GdkDisplayGetterFn>(getter) } {
        Ok(f) => unsafe { f(display.as_ptr()) },
        Err(e) => {
            tracing::debug!("[render] {} not resolvable: {e}", String::from_utf8_lossy(getter));
            std::ptr::null_mut()
        }
    };
    (kind, raw)
}

/// Creates the mpv handle and hooks the video surface into the Tauri window.
/// Called from the Tauri `setup` hook, i.e. on the GTK main thread.
pub fn attach_to_window(state: Arc<AppState>, window: WebviewWindow) -> Result<()> {
    // GTK's init calls setlocale(LC_ALL, ""), but libmpv refuses to create a
    // handle unless LC_NUMERIC is "C" (it parses/prints numbers itself). Same
    // fix every GTK mpv front-end applies (e.g. Celluloid); JavaScript number
    // formatting in WebKit goes through ICU and is not affected.
    unsafe {
        libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
    }
    let mpv = Arc::new(Mpv::new(None)?);
    mpv.start_event_pump(window.app_handle().clone());
    state.set_mpv(mpv.clone());

    window
        .with_webview(move |platform_webview| {
            let webview = platform_webview.inner();
            match install_video_surface(webview, mpv) {
                Ok(()) => {
                    tracing::info!("[render] GtkGLArea video surface installed under the webview")
                }
                Err(e) => tracing::error!("[render] linux video pipeline failed: {e:#}"),
            }
        })
        .map_err(|e| anyhow!("with_webview: {e}"))?;
    Ok(())
}

fn install_video_surface(webview: webkit2gtk::WebView, mpv: Arc<Mpv>) -> Result<()> {
    // Tauri builds: GtkApplicationWindow -> GtkBox (tao's "default vbox") ->
    // WebKitWebView. tauri-runtime-wry's undecorated-resize handler assumes
    // `webview.parent().parent()` is the GtkWindow, so the overlay must take
    // the vbox's place directly under the window: GtkApplicationWindow ->
    // GtkOverlay -> [GtkGLArea, WebKitWebView]. The detached vbox is only used
    // by tao for a menu bar, which this app does not have.
    let vbox = webview
        .parent()
        .ok_or_else(|| anyhow!("webview has no parent widget"))?
        .downcast::<gtk::Container>()
        .map_err(|_| anyhow!("webview parent is not a GtkContainer"))?;
    let window = vbox
        .parent()
        .ok_or_else(|| anyhow!("webview container has no parent"))?
        .downcast::<gtk::Window>()
        .map_err(|w| anyhow!("expected the GtkWindow above the webview, found {}", w.type_().name()))?;

    let overlay = gtk::Overlay::new();
    let gl_area = gtk::GLArea::new();
    gl_area.set_has_alpha(false);
    gl_area.set_has_depth_buffer(false);
    gl_area.set_has_stencil_buffer(false);
    gl_area.set_auto_render(true);
    gl_area.set_can_focus(false);
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);

    // Re-parent: the GL area becomes the main child of the overlay, the
    // webview (background alpha 0, set by wry for transparent windows) floats
    // on top and keeps receiving all input.
    vbox.remove(&webview);
    window.remove(&vbox);
    overlay.add(&gl_area);
    overlay.add_overlay(&webview);
    window.add(&overlay);

    let render_ctx: Rc<RefCell<Option<GlRenderContext>>> = Rc::new(RefCell::new(None));
    let get_integerv: Rc<Cell<Option<GlGetIntegervFn>>> = Rc::new(Cell::new(None));
    let render_failures: Rc<Cell<u32>> = Rc::new(Cell::new(0));

    // The GL context only exists once the area is realized: everything mpv
    // related is created there (and torn down again in `unrealize`).
    let init_render = {
        let render_ctx = render_ctx.clone();
        let get_integerv = get_integerv.clone();
        let mpv = mpv.clone();
        Rc::new(move |area: &gtk::GLArea| {
            if render_ctx.borrow().is_some() {
                return;
            }
            area.make_current();
            if let Some(e) = area.error() {
                tracing::error!("[render] GtkGLArea failed to create a GL context: {e}");
                return;
            }
            match create_render_context(area, &mpv) {
                Ok((ctx, gi)) => {
                    *render_ctx.borrow_mut() = Some(ctx);
                    get_integerv.set(Some(gi));
                    tracing::info!("[render] libmpv render context attached to GtkGLArea");
                }
                Err(e) => tracing::error!("[render] mpv render context: {e:#}"),
            }
        })
    };
    {
        let init_render = init_render.clone();
        gl_area.connect_realize(move |area| init_render(area));
    }

    {
        let render_ctx = render_ctx.clone();
        let get_integerv = get_integerv.clone();
        let render_failures = render_failures.clone();
        let first_frame_logged: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        gl_area.connect_render(move |area, _gl_ctx| {
            let guard = render_ctx.borrow();
            let (Some(ctx), Some(gi)) = (guard.as_ref(), get_integerv.get()) else {
                return glib::Propagation::Proceed;
            };
            let scale = area.scale_factor().max(1);
            let w = (area.allocated_width() * scale).max(1);
            let h = (area.allocated_height() * scale).max(1);
            let mut fbo: i32 = 0;
            unsafe { gi(GL_FRAMEBUFFER_BINDING, &mut fbo) };
            // ADVANCED_CONTROL is off, so `update` is optional; it still drains
            // the pending update flags so mpv does not re-signal stale frames.
            let _ = ctx.update();
            match ctx.render(fbo, w, h, true) {
                Ok(()) => {
                    if !first_frame_logged.get() {
                        first_frame_logged.set(true);
                        tracing::info!(
                            "[render] first mpv frame rendered into GtkGLArea ({w}x{h}, fbo {fbo}, scale {scale})"
                        );
                    }
                }
                Err(e) => {
                    let n = render_failures.get();
                    if n < 5 {
                        tracing::warn!("[render] mpv render frame failed: {e:#}");
                    }
                    render_failures.set(n.saturating_add(1));
                }
            }
            glib::Propagation::Stop
        });
    }

    {
        let render_ctx = render_ctx.clone();
        gl_area.connect_unrealize(move |area| {
            // mpv_render_context_free must run with the GL context current and
            // before the mpv handle itself is destroyed.
            area.make_current();
            if render_ctx.borrow_mut().take().is_some() {
                tracing::info!("[render] libmpv render context released");
            }
        });
    }

    // Handlers are in place: show now. The window is already mapped, so this
    // realizes the GL area synchronously and creates the render context; the
    // explicit call covers the case where GTK realized it even earlier.
    overlay.show_all();
    webview.grab_focus();
    if gl_area.is_realized() {
        init_render(&gl_area);
    }

    Ok(())
}

fn create_render_context(
    area: &gtk::GLArea,
    mpv: &Mpv,
) -> Result<(GlRenderContext, GlGetIntegervFn)> {
    let (kind, native) = native_display();
    let loader = Box::new(GlLoader::new(kind == DisplayKind::Wayland));

    let gi_ptr = loader.get(c"glGetIntegerv");
    if gi_ptr.is_null() {
        return Err(anyhow!("glGetIntegerv not resolvable: no usable GL loader"));
    }
    let gi: GlGetIntegervFn =
        unsafe { std::mem::transmute::<*mut c_void, GlGetIntegervFn>(gi_ptr) };

    tracing::info!(
        "[render] creating mpv render context (display: {kind:?}, native handle: {})",
        if native.is_null() { "none" } else { "yes" }
    );
    let handle = mpv.raw_handle() as *mut sys::mpv_handle;
    let mut ctx = GlRenderContext::new(handle, loader, kind, native)?;

    // mpv calls this from its own threads: hop to the GTK main loop and ask
    // the GL area for a redraw; the actual render happens in the `render`
    // signal with the GL context current.
    let weak: glib::SendWeakRef<gtk::GLArea> = area.downgrade().into();
    ctx.set_update_callback(move || {
        let weak = weak.clone();
        glib::MainContext::default().invoke(move || {
            if let Some(area) = weak.upgrade() {
                area.queue_render();
            }
        });
    });

    Ok((ctx, gi))
}
