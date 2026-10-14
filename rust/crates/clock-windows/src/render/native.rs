//! DirectWrite/Direct2D painting. Every object here belongs to the UI thread that
//! created it: `Renderer` and `ClockSurface` are `!Send`, matching the
//! single-threaded Direct2D factory they are built from. Nothing here is shared
//! with discovery or timer workers.
use super::{
    Extent, Fit, Overflow, PixelSize, RenderError, TextMeasurer, WorstCase, font_size_px,
    padding_px, plan_fit,
};
use crate::theme::{Palette, Rgb};
use clock_core::ClockText;
use std::marker::PhantomData;
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Direct2D::{
                Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT},
                D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
                D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
                D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
                DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        },
        UI::WindowsAndMessaging::{
            GetClientRect, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SystemParametersInfoW,
        },
    },
    core::{HRESULT, PCWSTR},
};
use windows_numerics::Vector2;

/// `D2DERR_RECREATE_TARGET`: the render target's device was lost or the
/// display configuration changed. Resources must be rebuilt, never reused.
const RECREATE_TARGET: HRESULT = HRESULT(0x8899000Cu32 as i32);
const FALLBACK_FONT: &str = "Segoe UI";
/// Layout box used only to measure a line; large enough that nothing wraps.
const MEASURE_BOX: f32 = 16_384.0;

fn native(operation: &'static str) -> impl FnOnce(windows::core::Error) -> RenderError {
    move |error| RenderError::Native {
        operation,
        code: error.code().0,
    }
}
fn color(rgb: Rgb) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(rgb.r) / 255.0,
        g: f32::from(rgb.g) / 255.0,
        b: f32::from(rgb.b) / 255.0,
        a: 1.0,
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn null_terminated(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The system UI font (message font), falling back to Segoe UI.
fn system_font_family() -> String {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is correctly sized and writable.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
            Default::default(),
        )
    }
    .is_ok();
    let face = &metrics.lfMessageFont.lfFaceName;
    let length = face
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(face.len());
    if ok && length > 0 {
        String::from_utf16_lossy(&face[..length])
    } else {
        FALLBACK_FONT.into()
    }
}

/// Process-wide (per UI thread) factories. Drop every surface before the renderer
/// is dropped; surfaces keep their own COM references, so order is not unsafe,
/// only wasteful.
pub struct Renderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    family: String,
    family_wide: Vec<u16>,
    /// Bumps when the system font changes so cached layouts are rebuilt.
    font_generation: u64,
    _ui_thread: PhantomData<*const ()>,
}
impl Renderer {
    pub fn new() -> Result<Self, RenderError> {
        // SAFETY: plain factory creation on the calling thread.
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                .map_err(native("D2D1CreateFactory"))?;
        // SAFETY: as above.
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .map_err(native("DWriteCreateFactory"))?;
        let family = system_font_family();
        Ok(Self {
            d2d,
            dwrite,
            family_wide: null_terminated(&family),
            family,
            font_generation: 0,
            _ui_thread: PhantomData,
        })
    }
    pub fn font_family(&self) -> &str {
        &self.family
    }
    /// Re-read the system font after a settings change. Returns true if it changed.
    pub fn refresh_system_font(&mut self) -> bool {
        let family = system_font_family();
        if family == self.family {
            return false;
        }
        self.family_wide = null_terminated(&family);
        self.family = family;
        self.font_generation += 1;
        true
    }
    pub fn create_surface(&self, hwnd: HWND) -> ClockSurface {
        ClockSurface {
            hwnd,
            target: None,
            brush: None,
            format: None,
            verdict: None,
            painted: None,
            device_generation: 0,
            _ui_thread: PhantomData,
        }
    }
    fn text_format(&self, font_px: f32) -> Result<IDWriteTextFormat, RenderError> {
        // SAFETY: `family_wide` is NUL-terminated and outlives the call.
        let format = unsafe {
            self.dwrite.CreateTextFormat(
                PCWSTR(self.family_wide.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font_px,
                PCWSTR(windows::core::w!("").as_ptr()),
            )
        }
        .map_err(native("CreateTextFormat"))?;
        // SAFETY: valid interface; these only set properties.
        unsafe {
            // Overflow is reported, never hidden by wrapping or trimming.
            format
                .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                .map_err(native("SetWordWrapping"))?;
            format
                .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)
                .map_err(native("SetTextAlignment"))?;
            format
                .SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
                .map_err(native("SetParagraphAlignment"))?;
        }
        Ok(format)
    }
    fn metrics(
        &self,
        format: &IDWriteTextFormat,
        text: &str,
        width: f32,
        height: f32,
    ) -> Result<
        (
            windows::Win32::Graphics::DirectWrite::IDWriteTextLayout,
            DWRITE_TEXT_METRICS,
        ),
        RenderError,
    > {
        let units = wide(text);
        // SAFETY: valid factory/format; `units` outlives the call.
        let layout = unsafe { self.dwrite.CreateTextLayout(&units, format, width, height) }
            .map_err(native("CreateTextLayout"))?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: `metrics` is writable.
        unsafe { layout.GetMetrics(&mut metrics) }.map_err(native("GetMetrics"))?;
        Ok((layout, metrics))
    }
}

struct DirectWriteMeasurer<'a> {
    renderer: &'a Renderer,
    format: &'a IDWriteTextFormat,
}
impl TextMeasurer for DirectWriteMeasurer<'_> {
    fn measure_line(&mut self, line: &str) -> Result<Extent, RenderError> {
        let (_, metrics) = self
            .renderer
            .metrics(self.format, line, MEASURE_BOX, MEASURE_BOX)?;
        Ok(Extent {
            // Trailing spaces in a user format still occupy room.
            width: metrics.widthIncludingTrailingWhitespace,
            height: metrics.height,
        })
    }
}

pub struct PaintRequest<'a> {
    pub text: &'a ClockText,
    /// Candidates for later dates; see `WorstCase::for_format`.
    pub worst_case: &'a WorstCase,
    pub dpi: u32,
    pub font_pt: u8,
    pub palette: Palette,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintOutcome {
    /// Pixels were drawn and presented.
    Painted,
    /// Identical inputs and a live device: nothing to draw.
    Unchanged,
    /// Current or future text does not fit. Nothing was drawn; the caller hides
    /// this overlay so the native clock remains available.
    Overflow(Overflow),
}

#[derive(Clone, Debug, PartialEq)]
struct PaintKey {
    lines: Vec<String>,
    palette: Palette,
    font_bits: u32,
    region: PixelSize,
    font_generation: u64,
    device_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct VerdictKey {
    font_bits: u32,
    region: PixelSize,
    padding: i32,
    worst_case: u64,
    font_generation: u64,
}

/// One HWND's drawing state. Dropping it releases the render target and brush
/// (COM `Release`); there are no raw handles to leak.
pub struct ClockSurface {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    brush: Option<(Palette, ID2D1SolidColorBrush)>,
    format: Option<(u32, u64, IDWriteTextFormat)>,
    /// Worst-case measurement only changes with font, region or format.
    verdict: Option<(VerdictKey, Option<Overflow>)>,
    painted: Option<PaintKey>,
    /// Identifies a render-target incarnation so a recreated device always repaints.
    device_generation: u64,
    _ui_thread: PhantomData<*const ()>,
}
impl ClockSurface {
    /// Discard device-dependent resources (also what `D2DERR_RECREATE_TARGET`
    /// does). The next `paint` recreates them and repaints. Also the hook used
    /// by tests to exercise the loss path without a GPU reset.
    pub fn invalidate_device(&mut self) {
        self.brush = None;
        self.target = None;
        self.painted = None;
        self.device_generation += 1;
    }
    /// Force the next `paint` to draw even if nothing else changed.
    pub fn mark_dirty(&mut self) {
        self.painted = None;
    }
    fn client_size(&self) -> Result<PixelSize, RenderError> {
        let mut rect = RECT::default();
        // SAFETY: `rect` is writable and the HWND is owned by this UI thread.
        unsafe { GetClientRect(self.hwnd, &mut rect) }.map_err(native("GetClientRect"))?;
        Ok(PixelSize {
            width: rect.right.saturating_sub(rect.left).max(0),
            height: rect.bottom.saturating_sub(rect.top).max(0),
        })
    }
    fn ensure_target(
        &mut self,
        renderer: &Renderer,
        size: PixelSize,
    ) -> Result<&ID2D1HwndRenderTarget, RenderError> {
        let pixels = D2D_SIZE_U {
            width: size.width as u32,
            height: size.height as u32,
        };
        if let Some(target) = &self.target {
            // SAFETY: valid target.
            let current = unsafe { target.GetPixelSize() };
            if current.width != pixels.width || current.height != pixels.height {
                // SAFETY: valid target; `pixels` outlives the call.
                if unsafe { target.Resize(&pixels) }.is_err() {
                    self.invalidate_device();
                } else {
                    self.painted = None;
                }
            }
        }
        if self.target.is_none() {
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    // Opaque: keeps ClearType and guarantees nothing shows through.
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                // 96 DPI over physical pixels; fonts carry the real scale.
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: Default::default(),
            };
            let window = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: pixels,
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            // SAFETY: both property structs outlive the call.
            let target = unsafe { renderer.d2d.CreateHwndRenderTarget(&properties, &window) }
                .map_err(native("CreateHwndRenderTarget"))?;
            // SAFETY: freshly created target.
            unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE) };
            self.target = Some(target);
            self.brush = None;
            self.painted = None;
        }
        Ok(self.target.as_ref().expect("target created above"))
    }
    fn text_format(
        &mut self,
        renderer: &Renderer,
        font_px: f32,
    ) -> Result<IDWriteTextFormat, RenderError> {
        let bits = font_px.to_bits();
        if let Some((cached_bits, generation, format)) = &self.format
            && *cached_bits == bits
            && *generation == renderer.font_generation
        {
            return Ok(format.clone());
        }
        let format = renderer.text_format(font_px)?;
        self.format = Some((bits, renderer.font_generation, format.clone()));
        Ok(format)
    }

    /// Measure, decide fit, then draw. Layout is always measured before pixels are
    /// touched; an overflow leaves the previous frame untouched and unshown.
    pub fn paint(
        &mut self,
        renderer: &Renderer,
        request: &PaintRequest<'_>,
    ) -> Result<PaintOutcome, RenderError> {
        let font_px =
            font_size_px(request.font_pt, request.dpi).ok_or(RenderError::InvalidScale)?;
        let padding = padding_px(request.dpi);
        let region = self.client_size()?;
        if region.width == 0 || region.height == 0 {
            return Ok(PaintOutcome::Overflow(Overflow {
                required: PixelSize {
                    width: padding.saturating_mul(2),
                    height: 1,
                },
                available: region,
            }));
        }
        let format = self.text_format(renderer, font_px)?;

        // Worst-case verdict is cached; the current text is always re-measured.
        let key = VerdictKey {
            font_bits: font_px.to_bits(),
            region,
            padding,
            worst_case: request.worst_case.fingerprint(),
            font_generation: renderer.font_generation,
        };
        let mut measurer = DirectWriteMeasurer {
            renderer,
            format: &format,
        };
        let future = match &self.verdict {
            Some((cached, verdict)) if *cached == key => *verdict,
            _ => {
                let empty = ClockText { lines: Vec::new() };
                let verdict = match plan_fit(
                    &mut measurer,
                    &empty,
                    request.worst_case,
                    region,
                    padding,
                    font_px,
                )? {
                    Fit::Fits(_) => None,
                    Fit::Overflow(overflow) => Some(overflow),
                };
                self.verdict = Some((key, verdict));
                verdict
            }
        };
        if let Some(overflow) = future {
            self.painted = None;
            return Ok(PaintOutcome::Overflow(overflow));
        }
        let current = WorstCase::from_lines(Vec::<Vec<String>>::new());
        if let Fit::Overflow(overflow) = plan_fit(
            &mut measurer,
            request.text,
            &current,
            region,
            padding,
            font_px,
        )? {
            self.painted = None;
            return Ok(PaintOutcome::Overflow(overflow));
        }

        let paint_key = PaintKey {
            lines: request.text.lines.clone(),
            palette: request.palette,
            font_bits: font_px.to_bits(),
            region,
            font_generation: renderer.font_generation,
            device_generation: self.device_generation,
        };
        if self.painted.as_ref() == Some(&paint_key) && self.target.is_some() {
            return Ok(PaintOutcome::Unchanged);
        }

        // One retry: the first EndDraw may report a lost device.
        for attempt in 0..2 {
            let device_generation = self.device_generation;
            match self.draw(renderer, &format, request, region, padding) {
                Ok(Some(overflow)) => {
                    self.painted = None;
                    return Ok(PaintOutcome::Overflow(overflow));
                }
                Ok(None) => {
                    self.painted = Some(PaintKey {
                        device_generation: self.device_generation,
                        ..paint_key
                    });
                    return Ok(PaintOutcome::Painted);
                }
                Err(RenderError::Native { code, .. })
                    if code == RECREATE_TARGET.0 && attempt == 0 =>
                {
                    self.invalidate_device();
                    debug_assert_ne!(device_generation, self.device_generation);
                }
                Err(error) => {
                    self.painted = None;
                    return Err(error);
                }
            }
        }
        unreachable!("the second attempt always returns")
    }

    fn draw(
        &mut self,
        renderer: &Renderer,
        format: &IDWriteTextFormat,
        request: &PaintRequest<'_>,
        region: PixelSize,
        padding: i32,
    ) -> Result<Option<Overflow>, RenderError> {
        let box_width = (region.width - padding * 2).max(1) as f32;
        let box_height = region.height as f32;
        let text = request.text.lines.join("\n");
        let (layout, metrics) = renderer.metrics(format, &text, box_width, box_height)?;
        // Final guard on the exact layout being drawn: no silent truncation.
        if metrics.widthIncludingTrailingWhitespace > box_width + 0.5
            || metrics.height > box_height + 0.5
        {
            return Ok(Some(Overflow {
                required: PixelSize {
                    width: (metrics.widthIncludingTrailingWhitespace.ceil() as i32)
                        .saturating_add(padding * 2),
                    height: metrics.height.ceil() as i32,
                },
                available: region,
            }));
        }
        self.ensure_target(renderer, region)?;
        let palette = request.palette;
        let target = self.target.clone().expect("ensured above");
        let brush = match &self.brush {
            Some((cached, brush)) if *cached == palette => brush.clone(),
            _ => {
                let value = color(palette.foreground);
                // SAFETY: `value` outlives the call.
                let brush = unsafe { target.CreateSolidColorBrush(&value, None) }
                    .map_err(native("CreateSolidColorBrush"))?;
                self.brush = Some((palette, brush.clone()));
                brush
            }
        };
        let background = color(palette.background);
        // SAFETY: draw calls are bracketed on the UI thread that owns the target.
        unsafe {
            target.BeginDraw();
            target.Clear(Some(&background));
            target.DrawTextLayout(
                Vector2 {
                    X: padding as f32,
                    Y: 0.0,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            target.EndDraw(None, None)
        }
        .map_err(native("EndDraw"))?;
        Ok(None)
    }
}
