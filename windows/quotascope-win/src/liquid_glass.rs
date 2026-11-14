//! Experimental live desktop glass for the Pulse windows.
//!
//! Desktop Duplication is used only while the opt-in is enabled. Its window
//! exclusion attribute prevents the rail and card feeding back into the next
//! frame. Capture failure always leaves the existing translucent fallback.
use std::time::{Duration, Instant};

use windows::core::{s, w, Interface};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
#[cfg(test)]
use windows::Win32::Graphics::Direct2D::D2D1_DEVICE_CONTEXT_OPTIONS_NONE;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1DeviceContext, ID2D1Geometry, ID2D1Layer, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS,
};
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_MODE_ROTATION_IDENTITY, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGISurface, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

#[repr(C)]
struct CompositionAttribute {
    attribute: i32,
    value: *const core::ffi::c_void,
    size: usize,
}

/// This attribute excludes only Desktop Duplication, not ordinary window
/// painting. Both surfaces must be excluded before either captures.
pub fn exclude_from_duplication(hwnd: HWND, enabled: bool) -> bool {
    let value: i32 = enabled.into();
    let data = CompositionAttribute {
        attribute: 24, // WCA_EXCLUDED_FROM_DDA
        value: &value as *const i32 as _,
        size: std::mem::size_of_val(&value),
    };
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
            return false;
        };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
            return false;
        };
        let set_attribute: unsafe extern "system" fn(HWND, *const CompositionAttribute) -> i32 =
            std::mem::transmute(proc);
        set_attribute(hwnd, &data) != 0
    }
}

#[derive(Default)]
pub struct DesktopGlass {
    bitmap: Option<ID2D1Bitmap>,
    bitmap_bounds: RECT,
    last_attempt: Option<Instant>,
    gpu: Option<GlassGpu>,
}

impl DesktopGlass {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn frame(&mut self, hwnd: HWND, rt: &ID2D1DeviceContext) -> Option<ID2D1Bitmap> {
        let mut bounds = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut bounds).ok()? };
        let changed = self.bitmap_bounds != bounds;
        // Capture is throttled globally, but a moving window must re-sample
        // the cached desktop immediately instead of flashing the fallback.
        if !changed
            && self
                .last_attempt
                .is_some_and(|t| t.elapsed() < Duration::from_millis(125))
        {
            return self.bitmap.clone();
        }
        self.last_attempt = Some(Instant::now());
        if changed {
            self.bitmap = None;
        }
        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return None;
        }
        let Some((source, output)) = shared_frame(bounds) else {
            if changed {
                eprintln!("quotascope: glass capture unavailable at {bounds:?}");
            }
            self.bitmap = None;
            return None;
        };
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut source_desc) };
        if self.gpu.as_ref().is_none_or(|gpu| {
            gpu.width != (bounds.right - bounds.left) as u32
                || gpu.height != (bounds.bottom - bounds.top) as u32
                || gpu.input_format != source_desc.Format
        }) {
            self.gpu = unsafe {
                match GlassGpu::new(
                    (bounds.right - bounds.left) as u32,
                    (bounds.bottom - bounds.top) as u32,
                    source_desc.Format,
                    rt,
                ) {
                    Ok(gpu) => Some(gpu),
                    Err(error) => {
                        eprintln!(
                            "quotascope: glass GPU {}x{} {:?} failed: {error:?}",
                            bounds.right - bounds.left,
                            bounds.bottom - bounds.top,
                            source_desc.Format
                        );
                        None
                    }
                }
            };
        }
        let Some(gpu) = self.gpu.as_ref() else {
            self.bitmap = None;
            return None;
        };
        let result = unsafe {
            let region = D3D11_BOX {
                left: (bounds.left - output.left) as u32,
                top: (bounds.top - output.top) as u32,
                right: (bounds.right - output.left) as u32,
                bottom: (bounds.bottom - output.top) as u32,
                back: 1,
                ..Default::default()
            };
            match gpu.render(&source, &region) {
                Ok(()) => Some(gpu.bitmap.clone()),
                Err(error) => {
                    eprintln!("quotascope: glass render failed: {error:?}");
                    None
                }
            }
        };
        self.bitmap = result;
        if self.bitmap.is_some() {
            self.bitmap_bounds = bounds;
        }
        self.bitmap.clone()
    }
}

fn contains(outer: RECT, inner: RECT) -> bool {
    inner.right > inner.left
        && inner.bottom > inner.top
        && inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
}

/// Apply the same bitmap through each part of the Pulse silhouette.
pub fn paint_masked(
    painter: &crate::d2d::Painter,
    bitmap: &ID2D1Bitmap,
    geometry: &ID2D1Geometry,
    width: f32,
    height: f32,
    opacity: f32,
) {
    let layer = D2D1_LAYER_PARAMETERS {
        contentBounds: crate::d2d::rect(0.0, 0.0, width, height),
        geometricMask: std::mem::ManuallyDrop::new(Some(geometry.clone())),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: crate::d2d::identity_matrix(),
        opacity: opacity.clamp(0.0, 1.0),
        opacityBrush: std::mem::ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS_NONE,
    };
    unsafe {
        painter.rt.PushLayer(&layer, None::<&ID2D1Layer>);
        painter.rt.DrawBitmap(
            bitmap,
            Some(&crate::d2d::rect(0.0, 0.0, width, height)),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
        painter.rt.PopLayer();
    }
}

fn open_output(bounds: RECT) -> Option<(IDXGIOutputDuplication, RECT)> {
    unsafe {
        let device = &crate::d2d::global_engine().d3d;
        let Ok(adapter) = device
            .cast::<IDXGIDevice>()
            .and_then(|device| device.GetAdapter())
        else {
            eprintln!("quotascope: glass: no DXGI adapter");
            return None;
        };
        let mut outputs = 0;
        for index in 0..32 {
            let Ok(output) = adapter.EnumOutputs(index) else {
                break;
            };
            outputs += 1;
            let desc = output.GetDesc().ok()?;
            if desc.AttachedToDesktop.as_bool()
                && desc.Rotation == DXGI_MODE_ROTATION_IDENTITY
                && contains(desc.DesktopCoordinates, bounds)
            {
                let output1: IDXGIOutput1 = output.cast().ok()?;
                return match output1.DuplicateOutput(device) {
                    Ok(duplication) => Some((duplication, desc.DesktopCoordinates)),
                    Err(error) => {
                        eprintln!("quotascope: glass: DuplicateOutput failed: {error:?}");
                        None
                    }
                };
            }
        }
        eprintln!("quotascope: glass: no output contains {bounds:?} ({outputs} outputs)");
        None
    }
}

/// Per-window GPU resources, reused until the window changes size.
struct GlassGpu {
    width: u32,
    height: u32,
    input_format: DXGI_FORMAT,
    input: ID3D11Texture2D,
    input_view: ID3D11ShaderResourceView,
    /// Horizontal blur output, then vertical blur input.
    temp_view: ID3D11ShaderResourceView,
    temp_rt: ID3D11RenderTargetView,
    /// Fully frosted desktop image sampled by the optical pass.
    blurred_view: ID3D11ShaderResourceView,
    blurred_rt: ID3D11RenderTargetView,
    output_view: ID3D11RenderTargetView,
    #[cfg(test)]
    output: ID3D11Texture2D,
    bitmap: ID2D1Bitmap,
    vertex: ID3D11VertexShader,
    blur_h: ID3D11PixelShader,
    blur_v: ID3D11PixelShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
}

impl GlassGpu {
    unsafe fn new(
        width: u32,
        height: u32,
        input_format: DXGI_FORMAT,
        rt: &ID2D1DeviceContext,
    ) -> windows::core::Result<Self> {
        if input_format != DXGI_FORMAT_B8G8R8A8_UNORM
            && input_format != DXGI_FORMAT_R16G16B16A16_FLOAT
        {
            return Err(windows::core::Error::from_win32());
        }
        let device = &crate::d2d::global_engine().d3d;
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: input_format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let mut input = None;
        device.CreateTexture2D(&desc, None, Some(&mut input))?;
        let input = input.ok_or_else(windows::core::Error::from_win32)?;
        let mut input_view = None;
        device.CreateShaderResourceView(&input, None, Some(&mut input_view))?;
        desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        desc.BindFlags = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
        let mut temp = None;
        device.CreateTexture2D(&desc, None, Some(&mut temp))?;
        let temp = temp.ok_or_else(windows::core::Error::from_win32)?;
        let mut temp_view = None;
        device.CreateShaderResourceView(&temp, None, Some(&mut temp_view))?;
        let mut temp_rt = None;
        device.CreateRenderTargetView(&temp, None, Some(&mut temp_rt))?;
        let mut blurred = None;
        device.CreateTexture2D(&desc, None, Some(&mut blurred))?;
        let blurred = blurred.ok_or_else(windows::core::Error::from_win32)?;
        let mut blurred_view = None;
        device.CreateShaderResourceView(&blurred, None, Some(&mut blurred_view))?;
        let mut blurred_rt = None;
        device.CreateRenderTargetView(&blurred, None, Some(&mut blurred_rt))?;
        let mut output = None;
        device.CreateTexture2D(&desc, None, Some(&mut output))?;
        let output = output.ok_or_else(windows::core::Error::from_win32)?;
        let mut output_view = None;
        device.CreateRenderTargetView(&output, None, Some(&mut output_view))?;
        let surface: IDXGISurface = output.cast()?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let bitmap: ID2D1Bitmap = rt
            .CreateBitmapFromDxgiSurface(&surface, Some(&props))?
            .cast()?;
        let mut vertex = None;
        device.CreateVertexShader(
            include_bytes!("../shaders/liquid_glass_vs.cso"),
            None,
            Some(&mut vertex),
        )?;
        let mut blur_h = None;
        let blur_h_code: &[u8] = if input_format == DXGI_FORMAT_R16G16B16A16_FLOAT {
            include_bytes!("../shaders/liquid_glass_blur_h_hdr_ps.cso")
        } else {
            include_bytes!("../shaders/liquid_glass_blur_h_ps.cso")
        };
        device.CreatePixelShader(blur_h_code, None, Some(&mut blur_h))?;
        let mut blur_v = None;
        device.CreatePixelShader(
            include_bytes!("../shaders/liquid_glass_blur_v_ps.cso"),
            None,
            Some(&mut blur_v),
        )?;
        let mut pixel = None;
        device.CreatePixelShader(
            include_bytes!("../shaders/liquid_glass_ps.cso"),
            None,
            Some(&mut pixel),
        )?;
        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        device.CreateSamplerState(&sampler_desc, Some(&mut sampler))?;
        Ok(Self {
            width,
            height,
            input_format,
            input,
            input_view: input_view.ok_or_else(windows::core::Error::from_win32)?,
            temp_view: temp_view.ok_or_else(windows::core::Error::from_win32)?,
            temp_rt: temp_rt.ok_or_else(windows::core::Error::from_win32)?,
            blurred_view: blurred_view.ok_or_else(windows::core::Error::from_win32)?,
            blurred_rt: blurred_rt.ok_or_else(windows::core::Error::from_win32)?,
            output_view: output_view.ok_or_else(windows::core::Error::from_win32)?,
            #[cfg(test)]
            output,
            bitmap,
            vertex: vertex.ok_or_else(windows::core::Error::from_win32)?,
            blur_h: blur_h.ok_or_else(windows::core::Error::from_win32)?,
            blur_v: blur_v.ok_or_else(windows::core::Error::from_win32)?,
            pixel: pixel.ok_or_else(windows::core::Error::from_win32)?,
            sampler: sampler.ok_or_else(windows::core::Error::from_win32)?,
        })
    }

    unsafe fn render(
        &self,
        source: &ID3D11Texture2D,
        region: &D3D11_BOX,
    ) -> windows::core::Result<()> {
        let context = crate::d2d::global_engine().d3d.GetImmediateContext()?;
        // D2D and this pass share the device. Reset D3D pipeline bindings
        // before and after the draw; never leave the output bound as a target.
        context.ClearState();
        context.CopySubresourceRegion(&self.input, 0, 0, 0, 0, source, 0, Some(region));
        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(&self.vertex, None);
        context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: self.width as f32,
            Height: self.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        // Separable frost: desktop -> horizontal -> vertical -> optics.
        context.OMSetRenderTargets(Some(&[Some(self.temp_rt.clone())]), None);
        context.PSSetShader(&self.blur_h, None);
        context.PSSetShaderResources(0, Some(&[Some(self.input_view.clone())]));
        context.Draw(3, 0);
        context.OMSetRenderTargets(Some(&[Some(self.blurred_rt.clone())]), None);
        context.PSSetShader(&self.blur_v, None);
        context.PSSetShaderResources(0, Some(&[Some(self.temp_view.clone())]));
        context.Draw(3, 0);
        context.OMSetRenderTargets(Some(&[Some(self.output_view.clone())]), None);
        context.PSSetShader(&self.pixel, None);
        context.PSSetShaderResources(0, Some(&[Some(self.blurred_view.clone())]));
        context.Draw(3, 0);
        context.ClearState();
        Ok(())
    }
}

// A process may have only one duplication interface per output. Both Pulse
// windows run on the same UI thread and share one cached GPU desktop texture.
thread_local! {
    static SHARED_OUTPUTS: std::cell::RefCell<Vec<SharedOutput>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct SharedOutput {
    bounds: RECT,
    duplication: IDXGIOutputDuplication,
    texture: Option<ID3D11Texture2D>,
    last_attempt: Option<Instant>,
}

pub fn clear_shared() {
    SHARED_OUTPUTS.with(|outputs| outputs.borrow_mut().clear());
}

fn shared_frame(bounds: RECT) -> Option<(ID3D11Texture2D, RECT)> {
    SHARED_OUTPUTS.with(|outputs| {
        let mut outputs = outputs.borrow_mut();
        let index = match outputs
            .iter()
            .position(|output| contains(output.bounds, bounds))
        {
            Some(index) => index,
            None => {
                let (duplication, output_bounds) = open_output(bounds)?;
                outputs.push(SharedOutput {
                    bounds: output_bounds,
                    duplication,
                    texture: None,
                    last_attempt: None,
                });
                outputs.len() - 1
            }
        };
        let output = &mut outputs[index];
        if !output
            .last_attempt
            .is_some_and(|t| t.elapsed() < Duration::from_millis(125))
        {
            output.last_attempt = Some(Instant::now());
            if unsafe { output.refresh() }.is_err() {
                outputs.remove(index);
                return None;
            }
        }
        Some((output.texture.clone()?, output.bounds))
    })
}

impl SharedOutput {
    unsafe fn refresh(&mut self) -> windows::core::Result<()> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        if let Err(error) = self
            .duplication
            .AcquireNextFrame(0, &mut info, &mut resource)
        {
            return if error.code() == DXGI_ERROR_WAIT_TIMEOUT {
                Ok(())
            } else {
                Err(error)
            };
        }
        let result = (|| {
            let source: ID3D11Texture2D = resource
                .ok_or_else(windows::core::Error::from_win32)?
                .cast()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            let recreate = self.texture.as_ref().is_none_or(|texture| {
                let mut old = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut old);
                old.Width != desc.Width
                    || old.Height != desc.Height
                    || old.Format != desc.Format
                    || old.ArraySize != desc.ArraySize
                    || old.MipLevels != desc.MipLevels
                    || old.SampleDesc.Count != desc.SampleDesc.Count
                    || old.SampleDesc.Quality != desc.SampleDesc.Quality
            });
            if recreate {
                desc.Usage = D3D11_USAGE_DEFAULT;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = 0;
                desc.MiscFlags = 0;
                let mut copy = None;
                crate::d2d::global_engine()
                    .d3d
                    .CreateTexture2D(&desc, None, Some(&mut copy))?;
                self.texture = copy;
            }
            let texture = self
                .texture
                .as_ref()
                .ok_or_else(windows::core::Error::from_win32)?;
            crate::d2d::global_engine()
                .d3d
                .GetImmediateContext()?
                .CopyResource(texture, &source);
            Ok(())
        })();
        let _ = self.duplication.ReleaseFrame();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercise the actual D3D shader and its texture interop, not a CPU model.
    #[test]
    fn gpu_glass_renders_and_updates_desktop_pixels() {
        unsafe {
            let engine = crate::d2d::global_engine();
            let rt = engine
                .device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .unwrap();
            let gpu = GlassGpu::new(64, 48, DXGI_FORMAT_B8G8R8A8_UNORM, &rt)
                .expect("glass GPU resources");
            let mut input_desc = D3D11_TEXTURE2D_DESC::default();
            gpu.input.GetDesc(&mut input_desc);
            let mut input = None;
            engine
                .d3d
                .CreateTexture2D(&input_desc, None, Some(&mut input))
                .unwrap();
            let input = input.unwrap();
            let context = engine.d3d.GetImmediateContext().unwrap();
            let mut staging_desc = input_desc;
            staging_desc.Usage = D3D11_USAGE_STAGING;
            staging_desc.BindFlags = 0;
            staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            engine
                .d3d
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                right: 64,
                bottom: 48,
                front: 0,
                back: 1,
            };
            for (blue, red) in [(24u8, 190u8), (210u8, 35u8)] {
                let pixels: Vec<u8> = (0..64 * 48).flat_map(|_| [blue, 70, red, 255]).collect();
                context.UpdateSubresource(&input, 0, None, pixels.as_ptr() as _, 64 * 4, 0);
                gpu.render(&input, &region).unwrap();
                context.CopyResource(&staging, &gpu.output);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .unwrap();
                let offset = 24 * mapped.RowPitch as usize + 32 * 4;
                let result = std::slice::from_raw_parts((mapped.pData as *const u8).add(offset), 4);
                assert!(
                    (i32::from(result[0]) - i32::from(blue)).abs() <= 3,
                    "blue: {result:?}"
                );
                assert!(
                    (i32::from(result[2]) - i32::from(red)).abs() <= 3,
                    "red: {result:?}"
                );
                assert_eq!(result[3], 255);
                context.Unmap(&staging, 0);
            }
            // A hard black/white boundary must become a mixture in the
            // shader output; this distinguishes blur from simple copying.
            let pixels: Vec<u8> = (0..64 * 48)
                .flat_map(|i| {
                    if i % 64 < 32 {
                        [0, 0, 0, 255]
                    } else {
                        [255, 255, 255, 255]
                    }
                })
                .collect();
            context.UpdateSubresource(&input, 0, None, pixels.as_ptr() as _, 64 * 4, 0);
            gpu.render(&input, &region).unwrap();
            context.CopyResource(&staging, &gpu.output);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let row = 24 * mapped.RowPitch as usize;
            let value = *((mapped.pData as *const u8).add(row + 32 * 4));
            // Twelve pixels away from the edge must still be a mixture. The
            // previous narrow kernel left this pixel almost pure black.
            let near = *((mapped.pData as *const u8).add(row + 20 * 4));
            context.Unmap(&staging, 0);
            assert!(
                (20..235).contains(&value),
                "boundary was not blurred: {value}"
            );
            assert!(
                (12..120).contains(&near),
                "blur radius is too narrow: {near}"
            );
        }
        gpu_glass_converts_float_scrgb_to_sdr();
        verify_card_clipping_and_bubble_seam();
    }

    fn gpu_glass_converts_float_scrgb_to_sdr() {
        unsafe {
            let engine = crate::d2d::global_engine();
            let rt = engine
                .device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .unwrap();
            let gpu = GlassGpu::new(64, 48, DXGI_FORMAT_R16G16B16A16_FLOAT, &rt).unwrap();
            let mut source_desc = D3D11_TEXTURE2D_DESC::default();
            gpu.input.GetDesc(&mut source_desc);
            let mut source = None;
            engine
                .d3d
                .CreateTexture2D(&source_desc, None, Some(&mut source))
                .unwrap();
            let source = source.unwrap();
            // IEEE half floats: linear RGB = 0.25, 0.5, 1.0; alpha = 1.
            let pixels: Vec<u16> = (0..64 * 48)
                .flat_map(|_| [0x3400u16, 0x3800, 0x3c00, 0x3c00])
                .collect();
            let context = engine.d3d.GetImmediateContext().unwrap();
            context.UpdateSubresource(&source, 0, None, pixels.as_ptr() as _, 64 * 8, 0);
            gpu.render(
                &source,
                &D3D11_BOX {
                    left: 0,
                    top: 0,
                    right: 64,
                    bottom: 48,
                    front: 0,
                    back: 1,
                },
            )
            .unwrap();
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            gpu.output.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            engine
                .d3d
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            context.CopyResource(&staging, &gpu.output);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let offset = 24 * mapped.RowPitch as usize + 32 * 4;
            let result = std::slice::from_raw_parts((mapped.pData as *const u8).add(offset), 4);
            assert!((250..=255).contains(&result[0]), "blue: {result:?}");
            assert!((184..=192).contains(&result[1]), "green: {result:?}");
            assert!((133..=141).contains(&result[2]), "red: {result:?}");
            assert_eq!(result[3], 255);
            context.Unmap(&staging, 0);
        }
    }

    fn verify_card_clipping_and_bubble_seam() {
        use windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_TARGET;
        use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_NORMAL;
        unsafe {
            let engine = crate::d2d::global_engine();
            let rt = engine
                .device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .unwrap();
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 240,
                Height: 120,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                ..Default::default()
            };
            let mut surface = None;
            engine
                .d3d
                .CreateTexture2D(&desc, None, Some(&mut surface))
                .unwrap();
            let surface = surface.unwrap();
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let dxgi: IDXGISurface = surface.cast().unwrap();
            let target = rt.CreateBitmapFromDxgiSurface(&dxgi, Some(&props)).unwrap();
            rt.SetTarget(&target);
            let mut read_desc = desc;
            read_desc.Usage = D3D11_USAGE_STAGING;
            read_desc.BindFlags = 0;
            read_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut readback = None;
            engine
                .d3d
                .CreateTexture2D(&read_desc, None, Some(&mut readback))
                .unwrap();
            let readback = readback.unwrap();
            let context = engine.d3d.GetImmediateContext().unwrap();
            let painter = crate::d2d::Painter { rt: &rt, engine };

            rt.BeginDraw();
            rt.Clear(None);
            let ink = painter
                .brush(crate::d2d::Rgba::new(1.0, 1.0, 1.0, 1.0))
                .unwrap();
            painter.text_clipped(
                "Millennium-Science-Technology-Long-Provider-Name",
                crate::d2d::rect(10.0, 5.0, 100.0, 25.0),
                16.0,
                DWRITE_FONT_WEIGHT_NORMAL,
                &ink,
                0,
                1,
            );
            rt.EndDraw(None, None).unwrap();
            context.CopyResource(&readback, &surface);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let bytes = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                mapped.RowPitch as usize * 120,
            );
            let alpha = |x: usize, y: usize| bytes[y * mapped.RowPitch as usize + x * 4 + 3];
            assert!(
                (10..110).any(|x| (0..35).any(|y| alpha(x, y) > 0)),
                "test text was not rendered"
            );
            assert!(
                (112..235).all(|x| (0..35).all(|y| alpha(x, y) == 0)),
                "long text escaped its allocated column"
            );
            context.Unmap(&readback, 0);

            rt.BeginDraw();
            rt.Clear(None);
            crate::pulse_rail::bubble(
                &painter,
                200.0,
                100.0,
                1.0,
                crate::geometry::Edge::Right,
                50.0,
                1.0,
                true,
                None,
            )
            .unwrap();
            rt.EndDraw(None, None).unwrap();
            context.CopyResource(&readback, &surface);
            context
                .Map(&readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let bytes = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                mapped.RowPitch as usize * 120,
            );
            let body = bytes[50 * mapped.RowPitch as usize + 175 * 4 + 3];
            let seam = bytes[50 * mapped.RowPitch as usize + 179 * 4 + 3];
            assert!(
                body > 40 && (i32::from(body) - i32::from(seam)).abs() <= 4,
                "double-painted bubble seam: {body} / {seam}"
            );
            context.Unmap(&readback, 0);
            rt.SetTarget(None);
        }
    }
}
