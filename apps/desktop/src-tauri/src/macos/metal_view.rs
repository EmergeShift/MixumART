//! macOS native surface overlay: MTKView + Metal.
//!
//! The overlay is hit-test transparent, so the WebView underneath keeps
//! handling mouse/keyboard/IME/clipboard. This module only turns MAP PNG
//! frames into a native Metal texture.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use image::ImageFormat;
use mixum_env_surface::Frame;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_02 as objc2;
use objc2_02::mutability::MainThreadOnly;
use objc2_02::{declare_class, msg_send, msg_send_id, ClassType, DeclaredClass};
use objc2_app_kit_02::{NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_foundation_02::{ns_string, CGPoint, CGRect, CGSize, MainThreadMarker, NSString};
use objc2_metal_02::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLLibrary, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLSamplerAddressMode,
    MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_metal_kit_02::{MTKView, MTKViewDelegate};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::AppState;

const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VOut {
    float4 pos [[position]];
    float2 uv;
};

vertex VOut vmain(uint vid [[vertex_id]]) {
    float2 p[4] = { float2(-1.0, -1.0), float2(1.0, -1.0), float2(-1.0, 1.0), float2(1.0, 1.0) };
    float2 uv[4] = { float2(0.0, 1.0), float2(1.0, 1.0), float2(0.0, 0.0), float2(1.0, 0.0) };
    VOut out;
    out.pos = float4(p[vid], 0.0, 1.0);
    out.uv = uv[vid];
    return out;
}

fragment float4 fmain(VOut in [[stage_in]],
                      texture2d<float> tex [[texture(0)]],
                      sampler s [[sampler(0)]]) {
    return tex.sample(s, in.uv);
}
"#;

declare_class!(
    struct SurfaceView;

    unsafe impl ClassType for SurfaceView {
        type Super = MTKView;
        type Mutability = MainThreadOnly;
        const NAME: &'static str = "MixumSurfaceView";
    }

    impl DeclaredClass for SurfaceView {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for SurfaceView {}

    unsafe impl MTKViewDelegate for SurfaceView {
        #[method(mtkView:drawableSizeWillChange:)]
        #[allow(non_snake_case)]
        unsafe fn mtkView_drawableSizeWillChange(&self, _view: &MTKView, _size: CGSize) {}

        #[method(drawInMTKView:)]
        #[allow(non_snake_case)]
        unsafe fn drawInMTKView(&self, view: &MTKView) {
            let ptr = self as *const Self as usize;
            if let Some(renderer) = views().lock().ok().and_then(|mut map| map.remove(&ptr)) {
                let mut renderer = renderer;
                renderer.draw(view);
                let _ = views().lock().map(|mut map| map.insert(ptr, renderer));
            }
        }
    }

    unsafe impl SurfaceView {
        #[method(hitTest:)]
        unsafe fn hit_test(&self, _point: CGPoint) -> *mut objc2_02::runtime::AnyObject {
            std::ptr::null_mut()
        }
    }
);

struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    texture: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Renderer {
    fn new(device: Retained<ProtocolObject<dyn MTLDevice>>) -> Result<Self, String> {
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| "MTLCommandQueue 创建失败".to_string())?;
        let source = NSString::from_str(SHADER);
        let library = device
            .newLibraryWithSource_options_error(&source, None)
            .map_err(|e| format!("Metal shader 编译失败: {e}"))?;
        let vertex = library
            .newFunctionWithName(ns_string!("vmain"))
            .ok_or_else(|| "vmain 不存在".to_string())?;
        let fragment = library
            .newFunctionWithName(ns_string!("fmain"))
            .ok_or_else(|| "fmain 不存在".to_string())?;

        let desc = MTLRenderPipelineDescriptor::new();
        desc.setVertexFunction(Some(&vertex));
        desc.setFragmentFunction(Some(&fragment));
        unsafe {
            desc.colorAttachments()
                .objectAtIndexedSubscript(0)
                .setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        }
        let pipeline = device
            .newRenderPipelineStateWithDescriptor_error(&desc)
            .map_err(|e| format!("Metal pipeline 创建失败: {e}"))?;

        let sampler_desc = MTLSamplerDescriptor::new();
        sampler_desc.setMinFilter(MTLSamplerMinMagFilter::Linear);
        sampler_desc.setMagFilter(MTLSamplerMinMagFilter::Linear);
        sampler_desc.setSAddressMode(MTLSamplerAddressMode::ClampToEdge);
        sampler_desc.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
        let sampler = device
            .newSamplerStateWithDescriptor(&sampler_desc)
            .ok_or_else(|| "Metal sampler 创建失败".to_string())?;

        Ok(Self {
            device,
            queue,
            pipeline,
            sampler,
            texture: None,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        full: bool,
        full_w: usize,
        full_h: usize,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        rgba: &[u8],
    ) -> Result<(), String> {
        if full_w == 0 || full_h == 0 || w == 0 || h == 0 {
            return Ok(());
        }
        if full
            || self.width != full_w
            || self.height != full_h
            || self.pixels.len() != full_w * full_h * 4
        {
            self.width = full_w;
            self.height = full_h;
            self.pixels = vec![0u8; full_w * full_h * 4];
            self.texture = None;
        }
        let expected = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or("frame overflow")?;
        if rgba.len() < expected || x + w > self.width || y + h > self.height {
            return Err("frame out of bounds".into());
        }
        if full {
            self.pixels.copy_from_slice(&rgba[..expected]);
        } else {
            for row in 0..h {
                let src = row * w * 4;
                let dst = ((y + row) * self.width + x) * 4;
                self.pixels[dst..dst + w * 4].copy_from_slice(&rgba[src..src + w * 4]);
            }
        }
        self.ensure_texture()?;
        self.upload()?;
        Ok(())
    }

    fn ensure_texture(&mut self) -> Result<(), String> {
        if self.texture.is_some() {
            return Ok(());
        }
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA8Unorm,
                self.width,
                self.height,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        let texture = self
            .device
            .newTextureWithDescriptor(&desc)
            .ok_or_else(|| "MTLTexture 创建失败".to_string())?;
        self.texture = Some(texture);
        Ok(())
    }

    fn upload(&self) -> Result<(), String> {
        let texture = self.texture.as_ref().ok_or("texture 未创建")?;
        let region = objc2_metal_02::MTLRegion {
            origin: objc2_metal_02::MTLOrigin { x: 0, y: 0, z: 0 },
            size: objc2_metal_02::MTLSize {
                width: self.width,
                height: self.height,
                depth: 1,
            },
        };
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                std::ptr::NonNull::new(self.pixels.as_ptr() as *mut std::ffi::c_void)
                    .ok_or("frame buffer empty")?,
                self.width * 4,
            );
        }
        Ok(())
    }

    fn draw(&mut self, view: &MTKView) {
        let (Some(pass), Some(drawable), Some(texture)) = (
            unsafe { view.currentRenderPassDescriptor() },
            unsafe { view.currentDrawable() },
            self.texture.as_ref(),
        ) else {
            return;
        };
        let Some(buffer) = self.queue.commandBuffer() else {
            return;
        };
        let Some(encoder) = buffer.renderCommandEncoderWithDescriptor(&pass) else {
            return;
        };
        encoder.setRenderPipelineState(&self.pipeline);
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(texture), 0);
            encoder.setFragmentSamplerState_atIndex(Some(&self.sampler), 0);
        }
        unsafe {
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::TriangleStrip, 0, 4);
            encoder.endEncoding();
            let _: () = msg_send![&*buffer, presentDrawable: &*drawable];
            buffer.commit();
        }
    }
}

static VIEWS: OnceLock<Mutex<HashMap<usize, Renderer>>> = OnceLock::new();

fn views() -> &'static Mutex<HashMap<usize, Renderer>> {
    VIEWS.get_or_init(|| Mutex::new(HashMap::new()))
}

// Metal objects are Objective-C handles; rendering is serialized by VIEWS.
unsafe impl Send for Renderer {}

pub fn attach(
    window: &WebviewWindow,
    _surface: u32,
    width: u32,
    height: u32,
) -> Result<usize, String> {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let device_ptr = unsafe { MTLCreateSystemDefaultDevice() };
    let device = unsafe { Retained::retain(device_ptr) }.ok_or("没有 Metal device")?;
    let frame = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width.max(1) as f64, height.max(1) as f64),
    );
    let view: Retained<SurfaceView> =
        unsafe { msg_send_id![mtm.alloc::<SurfaceView>(), initWithFrame: frame, device: &*device] };
    let renderer = Renderer::new(device)?;
    unsafe {
        view.setColorPixelFormat(MTLPixelFormat::BGRA8Unorm);
        view.setClearColor(objc2_metal_02::MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        });
        view.setEnableSetNeedsDisplay(true);
        view.setPaused(true);
        view.setPreferredFramesPerSecond(30);
        let delegate: &ProtocolObject<dyn MTKViewDelegate> = ProtocolObject::from_ref(&*view);
        view.setDelegate(Some(delegate));
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewWidthSizable
                | NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
    }

    let ns_window = window.ns_window().map_err(|e| e.to_string())? as *mut NSWindow;
    let content: Retained<NSView> = unsafe { msg_send_id![ns_window, contentView] };
    unsafe {
        view.setFrame(content.bounds());
        content.addSubview(&view);
    }

    let ptr = Retained::as_ptr(&view) as usize;
    views().lock().expect("native views").insert(ptr, renderer);
    // Keep the view alive via the content view; the AppState map owns the raw
    // pointer for frame delivery/removal.
    std::mem::forget(view);
    Ok(ptr)
}

pub fn push_frame(app: &AppHandle, surface: u32, frame: &Frame) {
    let ptr = {
        let state = app.state::<AppState>();
        let views = state.native_surfaces.lock().ok();
        views
            .and_then(|views| views.get(&surface).copied())
            .unwrap_or(0)
    };
    if ptr == 0 {
        return;
    }
    let Ok(image) = image::load_from_memory_with_format(&frame.data, ImageFormat::Png) else {
        return;
    };
    let rgba = image.to_rgba8();
    let full = frame.x == 0 && frame.y == 0 && frame.w == frame.full_w && frame.h == frame.full_h;
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let bytes = rgba.into_raw();
    let (full_w, full_h, px, py) = (
        frame.full_w as usize,
        frame.full_h as usize,
        frame.x as usize,
        frame.y as usize,
    );
    let app = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut views = views().lock().expect("native views");
        let Some(renderer) = views.get_mut(&ptr) else {
            return;
        };
        if let Err(error) = renderer.apply(full, full_w, full_h, px, py, w, h, &bytes) {
            eprintln!("native surface apply: {error}");
            return;
        }
        let view = unsafe { &*(ptr as *const SurfaceView) };
        unsafe { view.setNeedsDisplay(true) };
    });
}

pub fn detach(app: &AppHandle, surface: u32) {
    let ptr = {
        let state = app.state::<AppState>();
        state
            .native_surfaces
            .lock()
            .ok()
            .and_then(|mut views| views.remove(&surface))
            .unwrap_or(0)
    };
    if ptr == 0 {
        return;
    }
    let app = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = views().lock().map(|mut map| map.remove(&ptr));
        let view = unsafe { &*(ptr as *const SurfaceView) };
        unsafe {
            view.removeFromSuperview();
            let _: () = msg_send![view, release];
        }
    });
}
