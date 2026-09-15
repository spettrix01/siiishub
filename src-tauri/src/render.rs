use std::sync::Arc;

use anyhow::Result;
use tauri::{Manager, WebviewWindow};

use crate::mpv::Mpv;
use crate::state::AppState;

#[cfg(windows)]
mod compositor {
    use anyhow::{anyhow, Result};
    use windows::core::Interface;
    use windows::Win32::Foundation::{HMODULE, HWND};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_RESOURCE_MISC_SHARED, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::DirectComposition::{
        DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN,
        DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS,
        DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
        DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };

    pub struct Compositor {
        pub d3d_device: ID3D11Device,
        pub d3d_ctx: ID3D11DeviceContext,
        pub swap: IDXGISwapChain1,
        pub interop_tex: ID3D11Texture2D,
        pub dcomp_device: IDCompositionDevice,
        #[allow(dead_code)]
        pub dcomp_target: IDCompositionTarget,
        #[allow(dead_code)]
        pub dcomp_visual: IDCompositionVisual,
    }

    impl Compositor {
        pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
            let width = width.max(1);
            let height = height.max(1);

            let mut d3d_device: Option<ID3D11Device> = None;
            let mut d3d_ctx: Option<ID3D11DeviceContext> = None;
            let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&feature_levels),
                    D3D11_SDK_VERSION,
                    Some(&mut d3d_device),
                    None,
                    Some(&mut d3d_ctx),
                )?;
            }
            let d3d_device = d3d_device.ok_or_else(|| anyhow!("D3D11 device null"))?;
            let d3d_ctx = d3d_ctx.ok_or_else(|| anyhow!("D3D11 context null"))?;

            let dxgi_factory: IDXGIFactory2 =
                unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };

            let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap = unsafe {
                dxgi_factory.CreateSwapChainForComposition(&d3d_device, &swap_desc, None)?
            };

            let interop_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            };
            let mut interop_tex: Option<ID3D11Texture2D> = None;
            unsafe {
                d3d_device.CreateTexture2D(&interop_desc, None, Some(&mut interop_tex))?;
            }
            let interop_tex = interop_tex.ok_or_else(|| anyhow!("interop tex null"))?;

            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let dcomp_device: IDCompositionDevice =
                unsafe { DCompositionCreateDevice(&dxgi_device)? };

            let dcomp_target = unsafe { dcomp_device.CreateTargetForHwnd(hwnd, false)? };
            let dcomp_visual = unsafe { dcomp_device.CreateVisual()? };
            unsafe {
                dcomp_visual.SetContent(&swap)?;
                dcomp_target.SetRoot(&dcomp_visual)?;
                dcomp_device.Commit()?;
            }

            tracing::info!(
                "[render] D3D11+DComp pipeline up — {}x{} @ HWND {:?}",
                width,
                height,
                hwnd
            );

            Ok(Self {
                d3d_device,
                d3d_ctx,
                swap,
                interop_tex,
                dcomp_device,
                dcomp_target,
                dcomp_visual,
            })
        }

        pub fn present_from_interop(&self) -> Result<()> {
            unsafe {
                let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
                self.d3d_ctx.CopyResource(&back, &self.interop_tex);
                self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
                self.dcomp_device.Commit()?;
            }
            Ok(())
        }

        pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
            let width = width.max(1);
            let height = height.max(1);
            unsafe {
                self.swap
                    .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
            }
            let interop_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            };
            let mut interop_tex: Option<ID3D11Texture2D> = None;
            unsafe {
                self.d3d_device
                    .CreateTexture2D(&interop_desc, None, Some(&mut interop_tex))?;
            }
            self.interop_tex = interop_tex.ok_or_else(|| anyhow!("interop tex null on resize"))?;
            Ok(())
        }
    }
}

#[cfg(windows)]
#[allow(dead_code)]
mod gl {

    use anyhow::{anyhow, Result};
    use std::ffi::{c_void, CString};
    use std::ptr;
    use windows::core::{PCSTR, PCWSTR};
    use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, HDC};
    use windows::Win32::Graphics::OpenGL::{
        wglCreateContext, wglDeleteContext, wglGetCurrentContext, wglGetProcAddress, wglMakeCurrent,
        ChoosePixelFormat, SetPixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW,
        PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, RegisterClassExW, CS_OWNDC, HCURSOR, HICON,
        WINDOW_EX_STYLE, WNDCLASSEXW, WS_OVERLAPPED,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    const WGL_CONTEXT_MAJOR_VERSION_ARB: i32 = 0x2091;
    const WGL_CONTEXT_MINOR_VERSION_ARB: i32 = 0x2092;
    const WGL_CONTEXT_PROFILE_MASK_ARB: i32 = 0x9126;
    const WGL_CONTEXT_CORE_PROFILE_BIT_ARB: i32 = 0x00000001;

    pub const WGL_ACCESS_READ_ONLY_NV: u32 = 0x0000;
    pub const WGL_ACCESS_READ_WRITE_NV: u32 = 0x0001;
    pub const WGL_ACCESS_WRITE_DISCARD_NV: u32 = 0x0002;

    pub const GL_TEXTURE_2D: u32 = 0x0DE1;
    pub const GL_RGBA8: u32 = 0x8058;
    pub const GL_RGBA: u32 = 0x1908;
    pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
    pub const GL_FRAMEBUFFER: u32 = 0x8D40;
    pub const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    pub const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;

    type GLuint = u32;
    type GLint = i32;
    type GLenum = u32;
    type GLsizei = i32;
    type GLboolean = u8;

    pub struct GlFns {
        pub gl_gen_textures: unsafe extern "system" fn(n: GLsizei, textures: *mut GLuint),
        pub gl_bind_texture: unsafe extern "system" fn(target: GLenum, texture: GLuint),
        pub gl_tex_image_2d: unsafe extern "system" fn(
            target: GLenum,
            level: GLint,
            internal_fmt: GLint,
            width: GLsizei,
            height: GLsizei,
            border: GLint,
            format: GLenum,
            type_: GLenum,
            pixels: *const c_void,
        ),
        pub gl_gen_framebuffers: unsafe extern "system" fn(n: GLsizei, fbos: *mut GLuint),
        pub gl_bind_framebuffer: unsafe extern "system" fn(target: GLenum, fbo: GLuint),
        pub gl_framebuffer_texture_2d: unsafe extern "system" fn(
            target: GLenum,
            attachment: GLenum,
            textarget: GLenum,
            texture: GLuint,
            level: GLint,
        ),
        pub gl_check_framebuffer_status: unsafe extern "system" fn(target: GLenum) -> GLenum,
        pub gl_viewport: unsafe extern "system" fn(x: GLint, y: GLint, w: GLsizei, h: GLsizei),
        pub gl_finish: unsafe extern "system" fn(),

        pub wgl_dx_open_device:
            unsafe extern "system" fn(d3d_device: *mut c_void) -> *mut c_void,
        pub wgl_dx_close_device: unsafe extern "system" fn(handle: *mut c_void) -> GLboolean,
        pub wgl_dx_register_object: unsafe extern "system" fn(
            device: *mut c_void,
            dx_object: *mut c_void,
            name: GLuint,
            type_: GLenum,
            access: u32,
        ) -> *mut c_void,
        pub wgl_dx_unregister_object:
            unsafe extern "system" fn(device: *mut c_void, object: *mut c_void) -> GLboolean,
        pub wgl_dx_lock_objects: unsafe extern "system" fn(
            device: *mut c_void,
            count: GLint,
            objects: *mut *mut c_void,
        ) -> GLboolean,
        pub wgl_dx_unlock_objects: unsafe extern "system" fn(
            device: *mut c_void,
            count: GLint,
            objects: *mut *mut c_void,
        ) -> GLboolean,
    }

    type WglCreateContextAttribsArbFn =
        unsafe extern "system" fn(hdc: HDC, share: HGLRC, attribs: *const i32) -> HGLRC;

    pub struct GlContext {
        hwnd: HWND,
        hdc: HDC,
        hglrc: HGLRC,
        pub opengl32: HMODULE,
    }

    #[derive(Clone, Copy)]
    pub struct GlProcCtx {
        pub opengl32: HMODULE,
    }

    unsafe impl Send for GlProcCtx {}
    unsafe impl Sync for GlProcCtx {}

    pub fn proc_static(ctx: &GlProcCtx, name: &str) -> *mut c_void {
        unsafe {
            let cname = CString::new(name).expect("GL proc name has interior NUL");
            let pcs = PCSTR(cname.as_ptr() as *const u8);
            if let Some(f) = wglGetProcAddress(pcs) {
                return f as *mut c_void;
            }
            if let Some(f) = GetProcAddress(ctx.opengl32, pcs) {
                return f as *mut c_void;
            }
            ptr::null_mut()
        }
    }

    impl GlContext {
        pub fn new() -> Result<Self> {
            unsafe {

                let class_name = wide("siiishub_gl");
                let class_pcw = PCWSTR(class_name.as_ptr());
                let hinstance = GetModuleHandleW(None)?;
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_OWNDC,
                    lpfnWndProc: Some(default_wnd_proc),
                    hInstance: hinstance.into(),
                    hIcon: HICON::default(),
                    hCursor: HCURSOR::default(),
                    lpszClassName: class_pcw,
                    ..Default::default()
                };

                let _ = RegisterClassExW(&wc);

                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class_pcw,
                    class_pcw,
                    WS_OVERLAPPED,
                    0,
                    0,
                    8,
                    8,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )?;

                let hdc = GetDC(Some(hwnd));
                if hdc.is_invalid() {
                    return Err(anyhow!("GetDC failed for offscreen GL window"));
                }

                let pfd = PIXELFORMATDESCRIPTOR {
                    nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                    nVersion: 1,
                    dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                    iPixelType: PFD_TYPE_RGBA,
                    cColorBits: 32,
                    cDepthBits: 24,
                    cStencilBits: 8,
                    iLayerType: PFD_MAIN_PLANE.0 as u8,
                    ..Default::default()
                };
                let pf = ChoosePixelFormat(hdc, &pfd);
                if pf == 0 {
                    return Err(anyhow!("ChoosePixelFormat failed"));
                }
                SetPixelFormat(hdc, pf, &pfd)?;

                let legacy = wglCreateContext(hdc)?;
                wglMakeCurrent(hdc, legacy)?;

                let create_attribs_name = CString::new("wglCreateContextAttribsARB")
                    .expect("static literal is a valid C string");
                let create_attribs_ptr =
                    wglGetProcAddress(PCSTR(create_attribs_name.as_ptr() as *const u8));
                let Some(create_attribs_raw) = create_attribs_ptr else {
                    let _ = wglMakeCurrent(hdc, HGLRC::default());
                    let _ = wglDeleteContext(legacy);
                    return Err(anyhow!(
                        "WGL_ARB_create_context not available (driver too old?)"
                    ));
                };
                let create_attribs: WglCreateContextAttribsArbFn =
                    std::mem::transmute(create_attribs_raw);

                let attribs: [i32; 7] = [
                    WGL_CONTEXT_MAJOR_VERSION_ARB,
                    3,
                    WGL_CONTEXT_MINOR_VERSION_ARB,
                    3,
                    WGL_CONTEXT_PROFILE_MASK_ARB,
                    WGL_CONTEXT_CORE_PROFILE_BIT_ARB,
                    0,
                ];
                let modern = create_attribs(hdc, HGLRC::default(), attribs.as_ptr());
                if modern.is_invalid() {
                    let _ = wglMakeCurrent(hdc, HGLRC::default());
                    let _ = wglDeleteContext(legacy);
                    return Err(anyhow!("wglCreateContextAttribsARB returned NULL"));
                }
                let _ = wglMakeCurrent(hdc, HGLRC::default());
                let _ = wglDeleteContext(legacy);
                wglMakeCurrent(hdc, modern)?;

                let opengl32_name = wide("opengl32.dll");
                let opengl32 = LoadLibraryW(PCWSTR(opengl32_name.as_ptr()))?;

                Ok(Self {
                    hwnd,
                    hdc,
                    hglrc: modern,
                    opengl32,
                })
            }
        }

        pub fn proc(&self, name: &str) -> *mut c_void {
            unsafe {
                let cname = CString::new(name).expect("GL proc name has interior NUL");
                let pcs = PCSTR(cname.as_ptr() as *const u8);
                if let Some(f) = wglGetProcAddress(pcs) {
                    return f as *mut c_void;
                }
                if let Some(f) = GetProcAddress(self.opengl32, pcs) {
                    return f as *mut c_void;
                }
                ptr::null_mut()
            }
        }

        // The target signatures are spelled out by the GlFns field types.
        #[allow(clippy::missing_transmute_annotations)]
        pub fn load_fns(&self) -> Result<GlFns> {
            unsafe {
                macro_rules! load {
                    ($name:literal) => {{
                        let p = self.proc($name);
                        if p.is_null() {
                            return Err(anyhow!("GL proc not found: {}", $name));
                        }
                        std::mem::transmute(p)
                    }};
                }
                Ok(GlFns {
                    gl_gen_textures: load!("glGenTextures"),
                    gl_bind_texture: load!("glBindTexture"),
                    gl_tex_image_2d: load!("glTexImage2D"),
                    gl_gen_framebuffers: load!("glGenFramebuffers"),
                    gl_bind_framebuffer: load!("glBindFramebuffer"),
                    gl_framebuffer_texture_2d: load!("glFramebufferTexture2D"),
                    gl_check_framebuffer_status: load!("glCheckFramebufferStatus"),
                    gl_viewport: load!("glViewport"),
                    gl_finish: load!("glFinish"),
                    wgl_dx_open_device: load!("wglDXOpenDeviceNV"),
                    wgl_dx_close_device: load!("wglDXCloseDeviceNV"),
                    wgl_dx_register_object: load!("wglDXRegisterObjectNV"),
                    wgl_dx_unregister_object: load!("wglDXUnregisterObjectNV"),
                    wgl_dx_lock_objects: load!("wglDXLockObjectsNV"),
                    wgl_dx_unlock_objects: load!("wglDXUnlockObjectsNV"),
                })
            }
        }

        pub fn make_current(&self) -> Result<()> {
            unsafe {
                wglMakeCurrent(self.hdc, self.hglrc)?;
            }
            Ok(())
        }
    }

    impl Drop for GlContext {
        fn drop(&mut self) {
            unsafe {
                if !wglGetCurrentContext().is_invalid() {
                    let _ = wglMakeCurrent(self.hdc, HGLRC::default());
                }
                if !self.hglrc.is_invalid() {
                    let _ = wglDeleteContext(self.hglrc);
                }
                if !self.hdc.is_invalid() {
                    ReleaseDC(Some(self.hwnd), self.hdc);
                }

                let _ = self.opengl32;
            }
        }
    }

    unsafe extern "system" fn default_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

pub async fn attach_to_window(state: Arc<AppState>, window: WebviewWindow) -> Result<()> {
    let parent_hwnd = state.main_hwnd();
    let mpv = Arc::new(Mpv::new(parent_hwnd)?);

    let app = window.app_handle().clone();
    mpv.start_event_pump(app);
    state.set_mpv(mpv.clone());

    #[cfg(windows)]
    {
        if let Some(hwnd_isize) = parent_hwnd {

            let (w, h) = match window.current_monitor() {
                Ok(Some(m)) => {
                    let s = m.size();
                    (s.width.max(1), s.height.max(1))
                }
                _ => match window.inner_size() {
                    Ok(s) => (s.width.max(1), s.height.max(1)),
                    Err(_) => (1920, 1080),
                },
            };
            tracing::info!("[render] sizing compositor swap chain to {}x{} (monitor)", w, h);
            let mpv_clone = mpv.clone();
            std::thread::spawn(move || {
                if let Err(e) = run_render_thread(hwnd_isize, w, h, mpv_clone) {
                    tracing::error!("[render] render thread exited: {e:#}");
                }
            });
        } else {
            tracing::warn!("[render] main HWND not captured — compositor disabled");
        }
    }

    Ok(())
}

#[cfg(windows)]
fn window_monitor_size(hwnd: windows::Win32::Foundation::HWND) -> Option<(u32, u32)> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    unsafe {
        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let mw = (mi.rcMonitor.right - mi.rcMonitor.left).max(1) as u32;
            let mh = (mi.rcMonitor.bottom - mi.rcMonitor.top).max(1) as u32;
            Some((mw, mh))
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn run_render_thread(hwnd_isize: isize, w: u32, h: u32, mpv: Arc<Mpv>) -> Result<()> {
    use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
    use windows::core::Interface;

    let hwnd = windows::Win32::Foundation::HWND(hwnd_isize as *mut _);
    tracing::info!("[render] starting render thread @ HWND {:?} {}x{}", hwnd, w, h);
    let mut comp = compositor::Compositor::new(hwnd, w, h)?;
    tracing::info!("[render] compositor created");
    let glx = gl::GlContext::new()?;
    tracing::info!("[render] GL context created");
    glx.make_current()?;
    tracing::info!("[render] GL context made current");
    let fns = glx.load_fns()?;
    tracing::info!("[render] GL function pointers loaded");

    let mut gl_tex: u32 = 0;
    let mut gl_fbo: u32 = 0;
    unsafe {
        (fns.gl_gen_textures)(1, &mut gl_tex);
        (fns.gl_gen_framebuffers)(1, &mut gl_fbo);
    }
    if gl_tex == 0 || gl_fbo == 0 {
        return Err(anyhow::anyhow!(
            "glGenTextures/glGenFramebuffers returned 0 (tex={}, fbo={}) \
             — GL context probably not actually current",
            gl_tex,
            gl_fbo
        ));
    }
    tracing::info!("[render] GL texture {} + FBO {} allocated", gl_tex, gl_fbo);

    let interop_device = unsafe {
        (fns.wgl_dx_open_device)(comp.d3d_device.as_raw())
    };
    if interop_device.is_null() {
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        return Err(anyhow::anyhow!(
            "wglDXOpenDeviceNV returned NULL (GetLastError = 0x{:X}) — \
             driver likely lacks WGL_NV_DX_interop2",
            err.0
        ));
    }
    tracing::info!("[render] WGL_NV_DX_interop2 device opened");

    let mut interop_obj = unsafe {
        (fns.wgl_dx_register_object)(
            interop_device,
            comp.interop_tex.as_raw(),
            gl_tex,
            gl::GL_TEXTURE_2D,
            gl::WGL_ACCESS_READ_WRITE_NV,
        )
    };
    if interop_obj.is_null() {
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        return Err(anyhow::anyhow!(
            "wglDXRegisterObjectNV failed (GetLastError = 0x{:X})",
            err.0
        ));
    }
    tracing::info!("[render] interop object registered");

    let proc_ctx = gl::GlProcCtx { opengl32: glx.opengl32 };
    let init = OpenGLInitParams::<gl::GlProcCtx> {
        get_proc_address: |ctx: &gl::GlProcCtx, name: &str| gl::proc_static(ctx, name),
        ctx: proc_ctx,
    };
    let mpv_handle = mpv.raw_handle() as *mut libmpv2_sys::mpv_handle;
    let mut render_ctx = unsafe {
        RenderContext::new(
            &mut *mpv_handle,
            [
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(init),
            ],
        )
    }
    .map_err(|e| anyhow::anyhow!("mpv RenderContext::new failed: {e:?}"))?;
    tracing::info!("[render] libmpv RenderContext attached");

    glx.make_current()?;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

    let wake_event =
        unsafe { CreateEventW(None, false, false, windows::core::PCWSTR(std::ptr::null())) }
            .map_err(|e| anyhow::anyhow!("CreateEventW failed: {e}"))?;
    let wake_raw = wake_event.0 as isize;
    render_ctx.set_update_callback(move || unsafe {
        let _ = SetEvent(HANDLE(wake_raw as *mut core::ffi::c_void));
    });

    tracing::info!(
        "[render] WGL/DX interop ready ({}x{}). Entering frame loop (mpv-driven).",
        w,
        h
    );

    const WAKE_TIMEOUT_MS: u32 = 250;
    let mut last_size: (u32, u32) = (0, 0);
    let mut cur_w = w;
    let mut cur_h = h;

    loop {
        unsafe { WaitForSingleObject(wake_event, WAKE_TIMEOUT_MS) };

        let new_frame = render_ctx
            .update()
            .map(|flags| (flags & libmpv2::render::mpv_render_update::Frame) != 0)
            .unwrap_or(true);

        let mut realloc = false;
        if let Some((mon_w, mon_h)) = window_monitor_size(hwnd) {
            if mon_w != cur_w || mon_h != cur_h {
                tracing::info!(
                    "[render] monitor size {}x{} -> {}x{}; reallocating swap/interop",
                    cur_w, cur_h, mon_w, mon_h
                );
                unsafe { (fns.wgl_dx_unregister_object)(interop_device, interop_obj) };
                let resize_res = comp.resize(mon_w, mon_h);
                interop_obj = unsafe {
                    (fns.wgl_dx_register_object)(
                        interop_device,
                        comp.interop_tex.as_raw(),
                        gl_tex,
                        gl::GL_TEXTURE_2D,
                        gl::WGL_ACCESS_READ_WRITE_NV,
                    )
                };
                if interop_obj.is_null() {
                    tracing::error!("[render] re-register interop after resize failed; aborting");
                    break;
                }
                match resize_res {
                    Ok(()) => {
                        cur_w = mon_w;
                        cur_h = mon_h;
                        realloc = true;
                    }
                    Err(e) => tracing::error!("[render] compositor resize failed: {e:#}"),
                }
            }
        }

        let (rw, rh) = {
            let mut rect = windows::Win32::Foundation::RECT::default();
            let ok = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect)
            };
            if ok.is_ok() {
                (
                    ((rect.right - rect.left) as u32).max(1).min(cur_w),
                    ((rect.bottom - rect.top) as u32).max(1).min(cur_h),
                )
            } else {
                (cur_w, cur_h)
            }
        };
        let resized = (rw, rh) != last_size;

        if !new_frame && !resized && !realloc {
            continue;
        }
        last_size = (rw, rh);

        let mut objs = [interop_obj];
        let lock_ok =
            unsafe { (fns.wgl_dx_lock_objects)(interop_device, 1, objs.as_mut_ptr()) };
        if lock_ok == 0 {
            tracing::error!("[render] wglDXLockObjectsNV failed; aborting");
            break;
        }

        unsafe {
            (fns.gl_bind_framebuffer)(gl::GL_FRAMEBUFFER, gl_fbo);
            (fns.gl_framebuffer_texture_2d)(
                gl::GL_FRAMEBUFFER,
                gl::GL_COLOR_ATTACHMENT0,
                gl::GL_TEXTURE_2D,
                gl_tex,
                0,
            );
            (fns.gl_viewport)(0, 0, rw as i32, rh as i32);
        }

        if let Err(e) = render_ctx.render::<gl::GlContext>(gl_fbo as i32, rw as i32, rh as i32, false)
        {
            tracing::warn!("[render] mpv render frame failed: {e:?}");
        }
        unsafe {
            (fns.gl_finish)();
        }

        unsafe {
            (fns.wgl_dx_unlock_objects)(interop_device, 1, objs.as_mut_ptr());
        }

        if let Err(e) = comp.present_from_interop() {
            tracing::warn!("[render] present failed: {e:#}");
        }
    }

    Ok(())
}
