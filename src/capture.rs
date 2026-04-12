use std::{mem::zeroed, slice};

use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_11_1,
            },
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::{
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY,
                    DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
                DXGI_ERROR_WAIT_TIMEOUT, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
                IDXGIOutput1, IDXGIOutputDuplication,
            },
        },
    },
    core::Interface,
};

use crate::{AppResult, config::DXGI_FRAME_TIMEOUT_MS};

pub(crate) struct CapturedImage {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) pixels: Vec<u8>,
}

pub(crate) struct PrimaryDisplayCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    bounds: DesktopBounds,
    desktop_image_in_system_memory: bool,
}

impl PrimaryDisplayCapture {
    pub(crate) fn new() -> AppResult<Self> {
        find_primary_display()
    }

    pub(crate) fn width(&self) -> i32 {
        self.bounds.width
    }

    pub(crate) fn height(&self) -> i32 {
        self.bounds.height
    }

    fn from_output(
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        output: IDXGIOutput,
    ) -> AppResult<Self> {
        let output_desc = unsafe { output.GetDesc()? };
        if !output_desc.AttachedToDesktop.as_bool() {
            return Err("Primary DXGI output is not attached to the desktop".into());
        }

        let output_width =
            output_desc.DesktopCoordinates.right - output_desc.DesktopCoordinates.left;
        let output_height =
            output_desc.DesktopCoordinates.bottom - output_desc.DesktopCoordinates.top;
        if output_width <= 0 || output_height <= 0 {
            return Err("Primary DXGI output reported an invalid size".into());
        }

        let output1: IDXGIOutput1 = output.cast()?;
        let duplication = unsafe { output1.DuplicateOutput(&device)? };
        let duplication_desc = unsafe { duplication.GetDesc() };

        if duplication_desc.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(format!(
                "DXGI output used unexpected pixel format {:?}",
                duplication_desc.ModeDesc.Format
            )
            .into());
        }

        if duplication_desc.Rotation != DXGI_MODE_ROTATION_IDENTITY
            && duplication_desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED
        {
            return Err("DXGI capture does not yet handle rotated displays".into());
        }

        Ok(Self {
            device,
            context,
            duplication,
            bounds: DesktopBounds {
                width: output_width,
                height: output_height,
            },
            desktop_image_in_system_memory: duplication_desc.DesktopImageInSystemMemory.as_bool(),
        })
    }

    pub(crate) fn capture(&self) -> AppResult<CapturedImage> {
        let mut pixels = vec![0_u8; self.bounds.width as usize * self.bounds.height as usize * 4];
        let mut pointer_only_frames = 0;
        let (_frame, desktop_resource) = loop {
            let mut frame_info = unsafe { zeroed() };
            let mut desktop_resource = None;

            match unsafe {
                self.duplication.AcquireNextFrame(
                    DXGI_FRAME_TIMEOUT_MS,
                    &mut frame_info,
                    &mut desktop_resource,
                )
            } {
                Ok(()) => {}
                Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    return Err("Timed out waiting for a DXGI desktop frame".into());
                }
                Err(error) => {
                    return Err(format!("Could not acquire DXGI desktop frame: {error}").into());
                }
            }

            if frame_info.AccumulatedFrames > 0 || pointer_only_frames >= 8 {
                let desktop_resource =
                    desktop_resource.ok_or("DXGI did not return a desktop resource")?;
                break (AcquiredFrame::new(&self.duplication), desktop_resource);
            }

            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }

            pointer_only_frames += 1;
        };

        if self.desktop_image_in_system_memory {
            copy_system_memory_surface(&self.duplication, &self.bounds, &mut pixels)?;
        } else {
            let texture: ID3D11Texture2D = desktop_resource.cast()?;
            let staging = create_staging_texture(&self.device, &texture)?;

            unsafe {
                self.context.CopyResource(&staging, &texture);
            }

            copy_staging_texture(&self.context, &staging, &self.bounds, &mut pixels)?;
        }

        if pixels
            .chunks_exact(4)
            .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
        {
            return Err("DXGI capture returned only black pixels".into());
        }

        Ok(CapturedImage {
            width: self.bounds.width,
            height: self.bounds.height,
            pixels,
        })
    }
}

fn find_primary_display() -> AppResult<PrimaryDisplayCapture> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut adapter_index = 0;

    loop {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(format!("Could not enumerate DXGI adapter: {error}").into()),
        };
        adapter_index += 1;

        let desc = unsafe { adapter.GetDesc1()? };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }

        let (device, context) = create_d3d11_device(&adapter)?;
        let mut output_index = 0;

        loop {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => {
                    return Err(format!("Could not enumerate DXGI output: {error}").into());
                }
            };
            output_index += 1;

            if !is_primary_output(&output)? {
                continue;
            }

            return PrimaryDisplayCapture::from_output(device, context, output);
        }
    }

    Err("Could not find the primary DXGI output".into())
}

fn is_primary_output(output: &IDXGIOutput) -> AppResult<bool> {
    let output_desc = unsafe { output.GetDesc()? };
    Ok(output_desc.AttachedToDesktop.as_bool()
        && output_desc.DesktopCoordinates.left == 0
        && output_desc.DesktopCoordinates.top == 0)
}

fn create_d3d11_device(adapter: &IDXGIAdapter1) -> AppResult<(ID3D11Device, ID3D11DeviceContext)> {
    let adapter: IDXGIAdapter = adapter.cast()?;
    let feature_levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut device = None;
    let mut context = None;

    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }

    let device = device.ok_or("D3D11CreateDevice did not return a device")?;
    let context = context.ok_or("D3D11CreateDevice did not return an immediate context")?;
    Ok((device, context))
}

fn create_staging_texture(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> AppResult<ID3D11Texture2D> {
    let mut desc = unsafe { zeroed::<D3D11_TEXTURE2D_DESC>() };
    unsafe {
        texture.GetDesc(&mut desc);
    }

    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };

    let mut staging = None;
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut staging))?;
    }

    staging.ok_or("CreateTexture2D did not return a staging texture".into())
}

fn copy_system_memory_surface(
    duplication: &IDXGIOutputDuplication,
    bounds: &DesktopBounds,
    desktop_pixels: &mut [u8],
) -> AppResult<()> {
    let mapped = unsafe { duplication.MapDesktopSurface()? };
    let _mapped = MappedDesktopSurface::new(duplication);

    copy_mapped_rows(
        mapped.pBits as *const u8,
        mapped.Pitch as usize,
        bounds.width as usize,
        bounds.height as usize,
        bounds,
        desktop_pixels,
    )
}

fn copy_staging_texture(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    bounds: &DesktopBounds,
    desktop_pixels: &mut [u8],
) -> AppResult<()> {
    let mut desc = unsafe { zeroed::<D3D11_TEXTURE2D_DESC>() };
    unsafe {
        texture.GetDesc(&mut desc);
    }

    if desc.Width != bounds.width as u32 || desc.Height != bounds.height as u32 {
        return Err(format!(
            "DXGI output texture size {}x{} did not match desktop output size {}x{}",
            desc.Width, desc.Height, bounds.width, bounds.height
        )
        .into());
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context.Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    }
    let _mapped = MappedTexture::new(context, texture);

    copy_mapped_rows(
        mapped.pData as *const u8,
        mapped.RowPitch as usize,
        desc.Width as usize,
        desc.Height as usize,
        bounds,
        desktop_pixels,
    )
}

fn copy_mapped_rows(
    src_pixels: *const u8,
    src_pitch: usize,
    src_width: usize,
    src_height: usize,
    bounds: &DesktopBounds,
    desktop_pixels: &mut [u8],
) -> AppResult<()> {
    let src_row_len = src_width * 4;
    let desktop_stride = bounds.width as usize * 4;

    for row in 0..src_height {
        let src = unsafe { slice::from_raw_parts(src_pixels.add(row * src_pitch), src_row_len) };
        let dst_start = row * desktop_stride;
        let dst_end = dst_start + src_row_len;
        desktop_pixels
            .get_mut(dst_start..dst_end)
            .ok_or("DXGI output pixels fell outside the virtual desktop buffer")?
            .copy_from_slice(src);
    }

    Ok(())
}

struct DesktopBounds {
    width: i32,
    height: i32,
}

struct AcquiredFrame<'a> {
    duplication: &'a IDXGIOutputDuplication,
}

impl<'a> AcquiredFrame<'a> {
    fn new(duplication: &'a IDXGIOutputDuplication) -> Self {
        Self { duplication }
    }
}

impl Drop for AcquiredFrame<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.duplication.ReleaseFrame();
        }
    }
}

struct MappedDesktopSurface<'a> {
    duplication: &'a IDXGIOutputDuplication,
}

impl<'a> MappedDesktopSurface<'a> {
    fn new(duplication: &'a IDXGIOutputDuplication) -> Self {
        Self { duplication }
    }
}

impl Drop for MappedDesktopSurface<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.duplication.UnMapDesktopSurface();
        }
    }
}

struct MappedTexture<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
}

impl<'a> MappedTexture<'a> {
    fn new(context: &'a ID3D11DeviceContext, texture: &'a ID3D11Texture2D) -> Self {
        Self { context, texture }
    }
}

impl Drop for MappedTexture<'_> {
    fn drop(&mut self) {
        unsafe {
            self.context.Unmap(self.texture, 0);
        }
    }
}
