//! Vello-rendered vector shapes (feature = "vector").
//!
//! M3 approach: shapes are rasterized once at compile time to cached PNGs
//! (keyed by shape/fill/size) and enter the GES timeline as image clips —
//! so GES-level transforms and opacity/position animations apply to them
//! like any other clip. Live per-frame vector animation (path morphs)
//! comes later with a real Vello source element.

use crate::document::{parse_color, ShapeKind};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use vello::kurbo::{Affine, BezPath, Circle, Ellipse, Point, RoundedRect, Stroke};
use vello::peniko::{Color, Fill};
use vello::wgpu;
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Gpu { device, queue })
    })
    .as_ref()
}

fn star_path(center: Point, points: u32, outer: f64, inner: f64) -> BezPath {
    let mut path = BezPath::new();
    for i in 0..(points * 2) {
        let r = if i % 2 == 0 { outer } else { inner };
        let a = std::f64::consts::PI * (i as f64) / (points as f64) - std::f64::consts::FRAC_PI_2;
        let p = Point::new(center.x + r * a.cos(), center.y + r * a.sin());
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    path
}

fn polygon_path(center: Point, sides: u32, radius: f64) -> BezPath {
    let mut path = BezPath::new();
    for i in 0..sides {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (sides as f64)
            - std::f64::consts::FRAC_PI_2;
        let p = Point::new(center.x + radius * a.cos(), center.y + radius * a.sin());
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    path
}

fn build_shape_scene(kind: ShapeKind, fill: Color, w: f64, h: f64) -> Scene {
    let mut scene = Scene::new();
    let cx = w / 2.0;
    let cy = h / 2.0;
    let r = w.min(h) / 2.0 - 2.0;
    match kind {
        ShapeKind::Rect => scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill,
            None,
            &RoundedRect::new(0.0, 0.0, w, h, w.min(h) * 0.08),
        ),
        ShapeKind::Circle => {
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &Circle::new((cx, cy), r))
        }
        ShapeKind::Ellipse => scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill,
            None,
            &Ellipse::new((cx, cy), (w / 2.0 - 2.0, h / 2.0 - 2.0), 0.0),
        ),
        ShapeKind::Star => scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill,
            None,
            &star_path(Point::new(cx, cy), 5, r, r * 0.42),
        ),
        ShapeKind::Polygon => scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill,
            None,
            &polygon_path(Point::new(cx, cy), 6, r),
        ),
        ShapeKind::Line => {
            let mut path = BezPath::new();
            path.move_to((2.0, cy));
            path.line_to((w - 2.0, cy));
            scene.stroke(&Stroke::new(h.max(4.0) * 0.35), Affine::IDENTITY, fill, None, &path);
        }
        ShapeKind::Arrow => {
            let shaft = h * 0.28;
            let head = (w * 0.28).min(h);
            let mut path = BezPath::new();
            path.move_to((2.0, cy - shaft / 2.0));
            path.line_to((w - head, cy - shaft / 2.0));
            path.line_to((w - head, cy - h / 2.0 + 2.0));
            path.line_to((w - 2.0, cy));
            path.line_to((w - head, cy + h / 2.0 - 2.0));
            path.line_to((w - head, cy + shaft / 2.0));
            path.line_to((2.0, cy + shaft / 2.0));
            path.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &path);
        }
    }
    scene
}

/// Rasterize a shape to a transparent PNG in `cache_dir`, returning its
/// path. Cached by shape/fill/size.
pub fn shape_png(
    cache_dir: &Path,
    kind: ShapeKind,
    fill_hex: &str,
    width: u32,
    height: u32,
) -> Result<PathBuf> {
    let file = cache_dir.join(format!(
        "shape-{kind:?}-{}-{width}x{height}.png",
        fill_hex.trim_start_matches('#')
    ));
    if file.exists() {
        return Ok(file);
    }
    std::fs::create_dir_all(cache_dir)?;

    let gpu = gpu().context("no GPU/Vulkan adapter available for shape rendering")?;
    let mut renderer = Renderer::new(&gpu.device, RendererOptions::default())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let argb = parse_color(fill_hex);
    let color = Color::from_rgba8(
        ((argb >> 16) & 0xff) as u8,
        ((argb >> 8) & 0xff) as u8,
        (argb & 0xff) as u8,
        ((argb >> 24) & 0xff) as u8,
    );
    let scene = build_shape_scene(kind, color, width as f64, height as f64);

    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shape target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_texture(
            &gpu.device,
            &gpu.queue,
            &scene,
            &view,
            &RenderParams {
                base_color: Color::from_rgba8(0, 0, 0, 0),
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let bytes_per_row = (width * 4).next_multiple_of(256);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let data = slice.get_mapped_range();

    let mut img = image::RgbaImage::new(width, height);
    for y in 0..height {
        let row = &data[(y * bytes_per_row) as usize..][..(width * 4) as usize];
        for x in 0..width {
            let i = (x * 4) as usize;
            img.put_pixel(x, y, image::Rgba([row[i], row[i + 1], row[i + 2], row[i + 3]]));
        }
    }
    img.save(&file).with_context(|| format!("saving {}", file.display()))?;
    Ok(file)
}
